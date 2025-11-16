use std::io;
use std::os::fd::RawFd;

use io_uring::IoUring;

use crate::{
    backend::BackendConnectionCache,
    balancer::config::WorkerConfig,
    core::{
        stream_pump::{Direction, Operation},
        user_data::unpack_user_data,
    },
};

use super::{
    connection_pool::ConnectionPool,
    handlers::{
        handle_accept, handle_connect_backend, handle_recv_backend_to_client,
        handle_recv_client_to_backend, handle_recv_headers, handle_send_backend_to_client,
        handle_send_client_to_backend,
    },
    uring_ops::post_accept,
};

/// Run a worker event loop
///
/// This is the main io_uring reactor that processes incoming connections,
/// parses HTTP headers, routes to backends, and streams data bidirectionally.
///
/// # Arguments
/// * `listen_fd` - File descriptor for the listening socket (SO_REUSEPORT)
/// * `config` - Worker configuration
pub fn run_worker(listen_fd: RawFd, config: WorkerConfig) -> io::Result<()> {
    let mut ring = IoUring::builder()
        .setup_single_issuer()
        .setup_defer_taskrun()
        .build(config.ring_size)?;

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

    for _ in 0..config.initial_accepts {
        let id = pool.alloc();
        pool.ensure_slot(id, -1);
        post_accept(&mut ring, listen_fd, id);
    }

    let mut events: [(u64, i32); 512] = [(0, 0); 512];
    let mut event_count_batch: usize = 0;

    loop {
        event_count_batch = 0;
        {
            let mut cq = ring.completion();
            while let Some(cqe) = cq.next() {
                if event_count_batch < 512 {
                    events[event_count_batch] = (cqe.user_data(), cqe.result());
                    event_count_batch += 1;
                }
            }
        }

        let has_pending = !ring.submission().is_empty();
        if has_pending {
            ring.submit()?;

            // after submit, check CQ again - operations may have completed
            if event_count_batch == 0 {
                let cq = ring.completion();
                for cqe in cq {
                    if event_count_batch < 512 {
                        events[event_count_batch] = (cqe.user_data(), cqe.result());
                        event_count_batch += 1;
                    }
                }
            }
        }

        // if still no events nor pending work, block until at least one arrives
        if event_count_batch == 0 {
            ring.submit_and_wait(1)?;

            let cq = ring.completion();
            for cqe in cq {
                if event_count_batch < 512 {
                    events[event_count_batch] = (cqe.user_data(), cqe.result());
                    event_count_batch += 1;
                }
            }
        }

        for i in 0..event_count_batch {
            let (tag, res) = events[i];
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
