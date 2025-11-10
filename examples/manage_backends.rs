/// Example: Managing backends at runtime
///
/// This example demonstrates how to:
/// - Initialize the backend pool
/// - Add backends dynamically
/// - Remove backends
/// - List current backends

use flax::core::route::{get_backend_pool, init_backend_pool, Backend};

fn main() {
    // Initialize with some default backends
    init_backend_pool(vec![
        Backend::new("127.0.0.1:8081".parse().unwrap()),
        Backend::new("127.0.0.1:8082".parse().unwrap()),
    ]);

    let pool = get_backend_pool();

    println!("Initial backends: {:?}", pool.list_backends());
    println!("Backend count: {}", pool.count());

    // Add a new backend
    println!("\nAdding 127.0.0.1:8083...");
    pool.add_backend(Backend::new("127.0.0.1:8083".parse().unwrap()));
    println!("Backends after addition: {:?}", pool.list_backends());
    println!("Backend count: {}", pool.count());

    // Select some backends (round-robin)
    println!("\nSelecting backends in round-robin:");
    for i in 0..6 {
        if let Some(addr) = pool.select() {
            println!("  Request {}: {}", i + 1, addr);
        }
    }

    // Remove a backend
    println!("\nRemoving 127.0.0.1:8082...");
    let removed = pool.remove_backend("127.0.0.1:8082".parse().unwrap());
    println!("Removed: {}", removed);
    println!("Backends after removal: {:?}", pool.list_backends());

    // Select more backends
    println!("\nSelecting backends after removal:");
    for i in 0..4 {
        if let Some(addr) = pool.select() {
            println!("  Request {}: {}", i + 1, addr);
        }
    }

    // Clear all backends
    println!("\nClearing all backends...");
    pool.clear();
    println!("Backends after clear: {:?}", pool.list_backends());
    println!("Backend count: {}", pool.count());

    // Try to select from empty pool
    match pool.select() {
        Some(addr) => println!("Selected: {}", addr),
        None => println!("No backends available (pool is empty)"),
    }
}
