use std::io;
use std::os::fd::RawFd;

use io_uring::{IoUring, opcode};

use crate::backend::{BackendConnectionCache, select_backend};
use crate::core::connection_pair::ConnectionPair;
use crate::core::constants;
use crate::core::stream_pump::{Direction, Operation, StreamPump};
use crate::core::user_data::{pack_user_data, unpack_user_data};
use crate::protocol::peek_request_headers;
use crate::util::fd::close_fd_quiet;

use super::connection_pool::ConnectionPool;
use super::uring_ops::{
    post_accept, post_connect_backend, post_recv_headers, post_recv_pump, post_send_pump,
};

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Number of initial accept operations to prime the pipeline
    pub initial_accepts: usize,
    /// Size of the io_uring submission/completion queue
    pub ring_size: u32,
    /// Capacity for I/O buffers (bidirectional streaming)
    pub io_buffer_capacity: usize,
    /// Capacity for HTTP header buffers
    pub header_buffer_capacity: usize,
    /// Initial capacity for connection pool
    pub pool_capacity: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            initial_accepts: constants::INITIAL_ACCEPTS_PER_WORKER,
            ring_size: 512,
            io_buffer_capacity: constants::IO_BUFFER_CAPACITY,
            header_buffer_capacity: constants::HEADER_BUFFER_CAPACITY,
            pool_capacity: 4096,
        }
    }
}

/// Run a worker event loop
///
/// This is the main io_uring reactor that processes incoming connections,
/// parses HTTP headers, routes to backends, and streams data bidirectionally.
///
/// # Arguments
/// * `listen_fd` - File descriptor for the listening socket (SO_REUSEPORT)
/// * `config` - Worker configuration
pub fn run_worker(listen_fd: RawFd, config: WorkerConfig) -> io::Result<()> {
    let mut ring = IoUring::new(config.ring_size)?;
    let mut pool = ConnectionPool::new(
        config.pool_capacity,
        config.io_buffer_capacity,
        config.header_buffer_capacity,
    );
    let mut backend_connection_cache = {
        let cache = BackendConnectionCache::new();
        if cache.is_none() {
            eprintln!("Failed to instantiate backend connection cache.");
        };
        cache.unwrap()
    };

    // Prime the accept pipeline
    for _ in 0..config.initial_accepts {
        let id = pool.alloc();
        pool.ensure_slot(id, -1);
        post_accept(&mut ring, listen_fd, id);
    }

    // Main event loop
    loop {
        ring.submit_and_wait(1)?;

        // Drain all completed events
        let mut events = Vec::new();
        {
            let mut cq = ring.completion();
            while let Some(cqe) = cq.next() {
                events.push((cqe.user_data(), cqe.result()));
            }
        }

        // Process each completed event
        for (tag, res) in events {
            let (id, op) = unpack_user_data(tag);

            let Some(_pair) = pool.get_mut(id) else {
                continue;
            };

            match op {
                Operation::Accept => {
                    handle_accept(&mut ring, &mut pool, id, res, listen_fd, &config)
                }

                Operation::RecvHeaders => handle_recv_headers(
                    &mut ring,
                    &mut pool,
                    &mut backend_connection_cache,
                    id,
                    res,
                ),

                Operation::ConnectBackend => handle_connect_backend(&mut ring, &mut pool, id, res),

                Operation::Recv(Direction::ClientToBackend) => {
                    handle_recv_client_to_backend(&mut ring, &mut pool, id, res)
                }

                Operation::Send(Direction::ClientToBackend) => {
                    handle_send_client_to_backend(&mut ring, &mut pool, id, res)
                }

                Operation::Recv(Direction::BackendToClient) => handle_recv_backend_to_client(
                    &mut ring,
                    &mut pool,
                    &mut backend_connection_cache,
                    id,
                    res,
                ),

                Operation::Send(Direction::BackendToClient) => {
                    handle_send_backend_to_client(&mut ring, &mut pool, id, res)
                }

                Operation::Timeout(_) => {
                    pool.teardown(id);
                }
            }
        }
    }
}

// ============================================================================
// Event Handlers
// ============================================================================

fn handle_accept(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
    listen_fd: RawFd,
    _config: &WorkerConfig,
) {
    if res < 0 {
        // Accept failed, re-arm on same slot
        post_accept(ring, listen_fd, id);
        return;
    }

    // Accept succeeded - store client FD and start reading headers
    if let Some(pair) = pool.get_mut(id) {
        pair.client_fd = res;
        pair.header_buffer.start = 0;
        pair.header_buffer.end = 0;
        post_recv_headers(ring, pair);
    }

    // Keep accept pipeline full - allocate new slot
    let nid = pool.alloc();
    pool.ensure_slot(nid, -1);
    post_accept(ring, listen_fd, nid);
}

fn handle_recv_headers(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    cache: &mut BackendConnectionCache,
    id: usize,
    res: i32,
) {
    if res <= 0 {
        if let Some(pair) = pool.get_mut(id) {
            pair.had_error = true;
        }
        pool.teardown(id);
        return;
    }

    let mut teardown = false;
    {
        let Some(pair) = pool.get_mut(id) else {
            return;
        };
        pair.header_buffer.wrote(res as usize);
        let win = pair.header_buffer.window();
        match peek_request_headers(win) {
            Err("Incomplete message") => {
                // Need more data
                post_recv_headers(ring, pair);
                return;
            }
            Err(_) => {
                // Malformed request - drop connection
                pair.had_error = true;
                teardown = true;
            }
            Ok(meta) => {
                // Headers complete - route to backend
                let backend_addr = select_backend();

                // Persist request metadata
                pair.request_content_length = meta.content_length_value;
                pair.request_transfer_encoding_chunked = meta.transfer_encoding_is_chunked;
                pair.backend_address = Some(backend_addr);

                let pump = &mut pair.pump_client_to_backend;
                let n = win.len().min(pump.buffer.len());

                pump.buffer[..n].copy_from_slice(&win[..n]);
                pump.bytes_ready_to_send = n;
                pump.bytes_already_sent = 0;

                pair.header_buffer.consume_to(pair.header_buffer.end);

                if let Some(backend_fd) = cache.borrow_connection(&backend_addr) {
                    pair.attach_backend_socket(backend_fd);
                    pair.start_streaming();

                    if pair.pump_client_to_backend.bytes_ready_to_send > 0 {
                        post_send_pump(
                            ring,
                            id,
                            &mut pair.pump_client_to_backend,
                            Operation::Send(Direction::ClientToBackend),
                        );
                    }

                    post_recv_pump(
                        ring,
                        id,
                        &mut pair.pump_backend_to_client,
                        Operation::Recv(Direction::BackendToClient),
                    );
                } else if post_connect_backend(ring, pair, backend_addr).is_err() {
                    pair.had_error = true;
                    teardown = true;
                }
            }
        }
    }

    if teardown {
        pool.teardown(id);
    }
}

fn handle_connect_backend(ring: &mut IoUring, pool: &mut ConnectionPool, id: usize, _res: i32) {
    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    // Check SO_ERROR to see if connection completed
    let mut err_code: i32 = 0;
    let mut err_len: libc::socklen_t = std::mem::size_of::<i32>() as libc::socklen_t;
    unsafe {
        libc::getsockopt(
            pair.backend_fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut err_code as *mut _ as *mut libc::c_void,
            &mut err_len,
        )
    };

    if err_code == libc::EINPROGRESS {
        // Still connecting - wait a tiny bit and try again (this shouldn't happen for localhost)
        let sqe = opcode::Nop::new()
            .build()
            .user_data(pack_user_data(id, Operation::ConnectBackend));
        unsafe {
            ring.submission().push(&sqe).expect("SQ full (nop retry)");
        }
        return;
    }

    if err_code != 0 {
        pair.had_error = true;
        pool.teardown(id);
        return;
    }

    // Connection established - start bidirectional streaming
    pair.start_streaming();

    // Client → Backend: send buffered request
    if pair.pump_client_to_backend.bytes_ready_to_send > 0 {
        post_send_pump(
            ring,
            id,
            &mut pair.pump_client_to_backend,
            Operation::Send(Direction::ClientToBackend),
        );
    }

    // Backend → Client: start receiving response
    post_recv_pump(
        ring,
        id,
        &mut pair.pump_backend_to_client,
        Operation::Recv(Direction::BackendToClient),
    );
}

fn handle_recv_client_to_backend(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
) {
    if res <= 0 {
        if let Some(pair) = pool.get_mut(id) {
            pair.had_error = true;
        }

        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    let pump = &mut pair.pump_client_to_backend;
    pump.recv_in_flight = false;
    pump.bytes_ready_to_send += res as usize;
    post_send_pump(ring, id, pump, Operation::Send(Direction::ClientToBackend));
}

fn handle_send_client_to_backend(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
) {
    if res < 0 {
        if let Some(pair) = pool.get_mut(id) {
            pair.had_error = true;
        }
        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    let pump = &mut pair.pump_client_to_backend;
    pump.send_in_flight = false;
    pump.bytes_already_sent += res as usize;

    if pump.bytes_already_sent < pump.bytes_ready_to_send {
        // Partial send - continue sending
        post_send_pump(ring, id, pump, Operation::Send(Direction::ClientToBackend));
    } else {
        // All data sent - reset and receive more
        pump.reset_buffer();
        post_recv_pump(ring, id, pump, Operation::Recv(Direction::ClientToBackend));
    }
}

fn handle_recv_backend_to_client(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    cache: &mut BackendConnectionCache,
    id: usize,
    res: i32,
) {
    if res < 0 {
        if let Some(pair) = pool.get_mut(id) {
            pair.had_error = true;
        }
        pool.teardown(id);
        return;
    }
    let reuse_backend = {
        let Some(pair) = pool.get_mut(id) else {
            return;
        };

        if res == 0 {
            pair.pump_backend_to_client.recv_in_flight = false;
            Some(finish_request(pair, cache))
        } else {
            let pump = &mut pair.pump_backend_to_client;
            pump.recv_in_flight = false;
            pump.bytes_ready_to_send += res as usize;
            post_send_pump(ring, id, pump, Operation::Send(Direction::BackendToClient));
            return;
        }
    };

    if let Some(true) = reuse_backend {
        pool.recycle_slot_only(id);
    } else if reuse_backend.is_some() {
        pool.teardown(id);
    }
}

fn handle_send_backend_to_client(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
) {
    if res < 0 {
        if let Some(pair) = pool.get_mut(id) {
            pair.had_error = true;
        }
        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    let pump = &mut pair.pump_backend_to_client;
    pump.send_in_flight = false;
    pump.bytes_already_sent += res as usize;

    if pump.bytes_already_sent < pump.bytes_ready_to_send {
        // Partial send - continue sending
        post_send_pump(ring, id, pump, Operation::Send(Direction::BackendToClient));
    } else {
        // All data sent - reset and receive more
        pump.reset_buffer();
        post_recv_pump(ring, id, pump, Operation::Recv(Direction::BackendToClient));
    }
}

fn finish_request(pair: &mut ConnectionPair, cache: &mut BackendConnectionCache) -> bool {
    let pumps_idle = pair.pump_client_to_backend.is_idle() && pair.pump_backend_to_client.is_idle();
    let healthy_backend = !pair.had_error && pair.backend_fd >= 0;

    let mut reused = false;

    if pumps_idle && healthy_backend {
        if let Some(addr) = pair.backend_address {
            cache.return_connection(&addr, pair.backend_fd);
            reused = true;
        } else {
            close_fd_quiet(pair.backend_fd);
        }
        pair.backend_fd = -1;
    } else if pair.backend_fd >= 0 {
        close_fd_quiet(pair.backend_fd);
        pair.backend_fd = -1;
    }

    pair.backend_address = None;
    pair.backend_sockaddr_storage = None;
    pair.backend_sockaddr_len = 0;
    pair.request_content_length = None;
    pair.request_transfer_encoding_chunked = false;
    pair.had_error = false;

    reset_pump_after_finish(&mut pair.pump_client_to_backend);
    reset_pump_after_finish(&mut pair.pump_backend_to_client);

    pair.header_buffer.start = 0;
    pair.header_buffer.end = 0;

    reused
}

fn reset_pump_after_finish(pump: &mut StreamPump) {
    pump.reset_buffer();
    pump.read_fd = -1;
    pump.write_fd = -1;
    pump.recv_in_flight = false;
    pump.send_in_flight = false;
    pump.remaining_request_body_bytes = None;
}
