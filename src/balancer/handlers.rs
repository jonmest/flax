use std::os::fd::RawFd;

use io_uring::{IoUring, opcode};

use crate::backend::{BackendConnectionCache, select_backend};
use crate::balancer::config::WorkerConfig;
use crate::core::connection_pair::ConnectionPair;
use crate::core::stream_pump::{Direction, Operation, StreamPump};
use crate::core::user_data::pack_user_data;
use crate::protocol::peek_request_headers;
use crate::util::fd::close_fd_quiet;

use super::connection_pool::ConnectionPool;
use super::uring_ops::{
    post_accept, post_connect_backend, post_recv_headers, post_recv_pump, post_send_pump,
};

pub fn handle_accept(
    ring: &mut IoUring,
    pool: &mut ConnectionPool,
    id: usize,
    res: i32,
    listen_fd: RawFd,
    _config: &WorkerConfig,
) {
    if res < 0 {
        // accept failed, re-arm on same slot
        post_accept(ring, listen_fd, id);
        return;
    }

    // accept succeeded - store client FD and start reading headers
    if let Some(pair) = pool.get_mut(id) {
        pair.client_fd = res;
        pair.header_buffer.start = 0;
        pair.header_buffer.end = 0;
        post_recv_headers(ring, pair);
    }

    // keep accept pipeline full - allocate new slot
    let nid = pool.alloc();
    pool.ensure_slot(nid, -1);
    post_accept(ring, listen_fd, nid);
}

pub fn handle_recv_headers(
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
                // need more data
                post_recv_headers(ring, pair);
                return;
            }
            Err(_) => {
                // malformed request - drop connection
                pair.had_error = true;
                teardown = true;
            }
            Ok(meta) => {
                // headers complete - route to backend
                let backend_addr = select_backend();

                // persist request metadata
                pair.request_content_length = meta.content_length_value;
                pair.request_transfer_encoding_chunked = meta.transfer_encoding_is_chunked;
                pair.backend_address = Some(backend_addr);

                // swap buffers to avoid copy (zero-copy if data is at front)
                let (header_buf, start, end) = pair.header_buffer.drain();
                let pump = &mut pair.pump_client_to_backend;
                let old_pump_buf = std::mem::replace(&mut pump.buffer, header_buf);
                pair.header_buffer.replace_buffer(old_pump_buf);

                // move data to front if needed (zero-copy when start==0)
                let data_len = end - start;
                if start != 0 && data_len > 0 {
                    pump.buffer.copy_within(start..end, 0);
                }
                pump.bytes_ready_to_send = data_len;
                pump.bytes_already_sent = 0;

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

pub fn handle_connect_backend(ring: &mut IoUring, pool: &mut ConnectionPool, id: usize, _res: i32) {
    let Some(pair) = pool.get_mut(id) else {
        return;
    };

    // check SO_ERROR to see if connection completed
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
        // still connecting - wait a tiny bit and try again (this shouldn't happen for localhost)
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

    // connection established - start bidirectional streaming
    pair.start_streaming();

    // client → backend: send buffered request
    if pair.pump_client_to_backend.bytes_ready_to_send > 0 {
        post_send_pump(
            ring,
            id,
            &mut pair.pump_client_to_backend,
            Operation::Send(Direction::ClientToBackend),
        );
    }

    // backend → client: start receiving response
    post_recv_pump(
        ring,
        id,
        &mut pair.pump_backend_to_client,
        Operation::Recv(Direction::BackendToClient),
    );
}

pub fn handle_recv_client_to_backend(
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

pub fn handle_send_client_to_backend(
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
        // partial send - continue sending
        post_send_pump(ring, id, pump, Operation::Send(Direction::ClientToBackend));
    } else {
        // all data sent - reset and receive more
        pump.reset_buffer();
        post_recv_pump(ring, id, pump, Operation::Recv(Direction::ClientToBackend));
    }
}

pub fn handle_recv_backend_to_client(
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

pub fn handle_send_backend_to_client(
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
        // partial send - continue sending
        post_send_pump(ring, id, pump, Operation::Send(Direction::BackendToClient));
    } else {
        // all data sent - reset and receive more
        pump.reset_buffer();
        post_recv_pump(ring, id, pump, Operation::Recv(Direction::BackendToClient));
    }
}

pub fn finish_request(pair: &mut ConnectionPair, cache: &mut BackendConnectionCache) -> bool {
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

pub fn reset_pump_after_finish(pump: &mut StreamPump) {
    pump.reset_buffer();
    pump.read_fd = -1;
    pump.write_fd = -1;
    pump.recv_in_flight = false;
    pump.send_in_flight = false;
    pump.remaining_request_body_bytes = None;
}
