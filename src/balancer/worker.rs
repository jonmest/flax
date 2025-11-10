use std::io;
use std::os::fd::RawFd;

use io_uring::IoUring;

use crate::backend::select_backend;
use crate::core::constants;
use crate::core::stream_pump::{Direction, Operation};
use crate::core::user_data::unpack_user_data;
use crate::protocol::peek_request_headers;

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

    // Prime the accept pipeline
    for _ in 0..config.initial_accepts {
        let id = pool.alloc();
        pool.ensure_slot(id, -1);
        post_accept(&mut ring, listen_fd, id);
    }

    // Main event loop
    let mut iteration = 0u64;
    loop {
        iteration += 1;
        if iteration % 10000 == 0 {
            eprintln!("[DEBUG LOOP] iteration {}", iteration);
        }

        eprintln!("[DEBUG LOOP] About to submit_and_wait");
        ring.submit_and_wait(1)?;
        eprintln!("[DEBUG LOOP] Returned from submit_and_wait");

        // Drain all completed events
        let mut events = Vec::new();
        {
            let mut cq = ring.completion();
            while let Some(cqe) = cq.next() {
                events.push((cqe.user_data(), cqe.result()));
            }
        }
        eprintln!("[DEBUG LOOP] Got {} events", events.len());

        // Process each completed event
        for (tag, res) in events {
            let (id, op) = unpack_user_data(tag);
            eprintln!("[DEBUG EVENT LOOP] id={}, op={:?}, res={}", id, op, res);

            let Some(_pair) = pool.get_mut(id) else {
                eprintln!("[DEBUG EVENT LOOP] pair not found for id={}", id);
                continue;
            };

            match op {
                Operation::Accept => handle_accept(
                    &mut ring,
                    &mut pool,
                    id,
                    res,
                    listen_fd,
                    &config,
                ),

                Operation::RecvHeaders => handle_recv_headers(&mut ring, &mut pool, id, res),

                Operation::ConnectBackend => {
                    handle_connect_backend(&mut ring, &mut pool, id, res)
                }

                Operation::Recv(Direction::ClientToBackend) => {
                    handle_recv_client_to_backend(&mut ring, &mut pool, id, res)
                }

                Operation::Send(Direction::ClientToBackend) => {
                    handle_send_client_to_backend(&mut ring, &mut pool, id, res)
                }

                Operation::Recv(Direction::BackendToClient) => {
                    handle_recv_backend_to_client(&mut ring, &mut pool, id, res)
                }

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
    eprintln!("[DEBUG ACCEPT] id={}, res={}", id, res);

    if res < 0 {
        eprintln!("[DEBUG ACCEPT] Accept failed, re-arming");
        // Accept failed, re-arm on same slot
        post_accept(ring, listen_fd, id);
        return;
    }

    eprintln!("[DEBUG ACCEPT] Accept succeeded, client_fd={}", res);

    // Accept succeeded - store client FD and start reading headers
    if let Some(pair) = pool.get_mut(id) {
        pair.client_fd = res;
        pair.header_buffer.start = 0;
        pair.header_buffer.end = 0;
        eprintln!("[DEBUG ACCEPT] Posting recv_headers for id={}", id);
        post_recv_headers(ring, pair);
    }

    // Keep accept pipeline full - allocate new slot
    let nid = pool.alloc();
    pool.ensure_slot(nid, -1);
    post_accept(ring, listen_fd, nid);
}

fn handle_recv_headers(ring: &mut IoUring, pool: &mut ConnectionPool, id: usize, res: i32) {
    eprintln!("[DEBUG] handle_recv_headers: id={}, res={}", id, res);

    if res <= 0 {
        eprintln!("[DEBUG] recv_headers failed, res={}", res);
        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        eprintln!("[DEBUG] pair not found for id={}", id);
        return;
    };

    pair.header_buffer.wrote(res as usize);
    eprintln!("[DEBUG] wrote {} bytes to header buffer", res);

    let win = pair.header_buffer.window();
    eprintln!("[DEBUG] window size: {} bytes", win.len());
    eprintln!("[DEBUG] window content: {:?}", String::from_utf8_lossy(&win[..win.len().min(100)]));

    match peek_request_headers(win) {
        Err("Incomplete message") => {
            eprintln!("[DEBUG] incomplete message, need more data");
            // Need more data
            post_recv_headers(ring, pair);
        }
        Err(e) => {
            eprintln!("[DEBUG] malformed request: {}", e);
            // Malformed request - drop connection
            pool.teardown(id);
        }
        Ok(meta) => {
            // Headers complete - route to backend
            let backend_addr = select_backend();

            // Persist request metadata
            pair.request_content_length = meta.content_length_value;
            pair.request_transfer_encoding_chunked = meta.transfer_encoding_is_chunked;

            // CRITICAL: Copy the ENTIRE request (headers + any body bytes)
            // We need to forward the complete HTTP request to the backend
            let request_data = win.to_vec();
            eprintln!("[DEBUG] Got request: {} bytes", request_data.len());

            // Connect to backend
            if let Err(e) = post_connect_backend(ring, pair, backend_addr) {
                eprintln!("connect setup failed: {e}");
                pool.teardown(id);
                return;
            }
            eprintln!("[DEBUG] Connecting to backend: {}", backend_addr);

            // Consume all the data we just copied
            pair.header_buffer.consume_to(pair.header_buffer.end);

            // Stage the complete request (headers + body) for sending to backend
            if !request_data.is_empty() {
                let pump = &mut pair.pump_client_to_backend;
                let n = request_data.len().min(pump.buffer.len());
                pump.buffer[..n].copy_from_slice(&request_data[..n]);
                pump.bytes_ready_to_send = n;
                pump.bytes_already_sent = 0;
                eprintln!("[DEBUG] Staged {} bytes for sending to backend", n);
            }
        }
    }
}

fn handle_connect_backend(ring: &mut IoUring, pool: &mut ConnectionPool, id: usize, res: i32) {
    eprintln!("[DEBUG] handle_connect_backend: id={}", id);

    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    // Connection already established - start bidirectional streaming
    pair.start_streaming();

    // Client → Backend: send buffered request
    if pair.pump_client_to_backend.bytes_ready_to_send > 0 {
        eprintln!("[DEBUG] Sending {} bytes to backend", pair.pump_client_to_backend.bytes_ready_to_send);
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
    eprintln!("[DEBUG SEND C->B] id={}, res={}", id, res);

    if res < 0 {
        eprintln!("[DEBUG SEND C->B] Send failed, tearing down");
        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        eprintln!("[DEBUG SEND C->B] Pair not found");
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
    id: usize,
    res: i32,
) {
    if res <= 0 {
        pool.teardown(id);
        return;
    }

    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    let pump = &mut pair.pump_backend_to_client;
    pump.recv_in_flight = false;
    pump.bytes_ready_to_send += res as usize;
    post_send_pump(ring, id, pump, Operation::Send(Direction::BackendToClient));
}

fn handle_send_backend_to_client(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
) {
    if res < 0 {
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
