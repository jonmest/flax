//! Load balancing logic and request processing pipeline
//!
//! This module provides the core load balancing functionality including:
//! - Worker event loop powered by io_uring
//! - Connection pool management
//! - io_uring operation helpers

pub mod connection_pool;
pub mod uring_ops;
pub mod worker;

// Re-export main types
pub use connection_pool::ConnectionPool;
pub use worker::{run_worker, WorkerConfig};
