// src/main.rs
// Entry point for the Flax load balancer

mod backend;
mod balancer;
mod core;
mod protocol;

use backend::{init_backend_pool, Backend};
use balancer::{run_worker, WorkerConfig};
use core::socket::make_reuseport_listener;

use core_affinity::CoreId;
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::{io, thread};

/// Main entry point - spawns one worker per CPU core
///
/// Each worker:
/// - Gets its own SO_REUSEPORT listener on the same address
/// - Is pinned to a dedicated CPU core
/// - Runs an independent io_uring event loop
///
/// This architecture provides:
/// - Zero cross-core communication
/// - Maximum cache locality
/// - Linear scalability with cores
fn main() -> io::Result<()> {
    let listen_addr: SocketAddr = "0.0.0.0:3000".parse().unwrap();

    // Initialize backend pool with default backends
    init_backend_pool(vec![
        Backend::new("127.0.0.1:8081".parse().unwrap()),
        Backend::new("127.0.0.1:8082".parse().unwrap()),
        Backend::new("127.0.0.1:8083".parse().unwrap()),
    ]);

    // Determine number of workers
    let cores: Vec<CoreId> = core_affinity::get_core_ids().expect("get_core_ids failed");
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(cores.len())
        .min(cores.len());

    eprintln!("Starting Flax load balancer");
    eprintln!("  Listen address: {}", listen_addr);
    eprintln!("  Workers: {}", workers);
    eprintln!("  Backends: 127.0.0.1:8081, 127.0.0.1:8082, 127.0.0.1:8083");

    // Spawn one worker per core
    let mut handles = Vec::with_capacity(workers);
    for i in 0..workers {
        let listener = make_reuseport_listener(listen_addr)?;
        let core = cores[i];
        let config = WorkerConfig::default();

        let h = thread::spawn(move || {
            core_affinity::set_for_current(core);
            eprintln!("[worker {i}] pinned to core {}", core.id);
            if let Err(e) = run_worker(listener.as_raw_fd(), config) {
                eprintln!("[worker {i}] fatal: {e}");
            }
        });
        handles.push(h);
    }

    // Wait for all workers
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}
