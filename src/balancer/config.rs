use crate::core::constants;

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
    pub sqpoll_cpu: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            initial_accepts: constants::INITIAL_ACCEPTS_PER_WORKER,
            ring_size: 512,
            io_buffer_capacity: constants::IO_BUFFER_CAPACITY,
            header_buffer_capacity: constants::HEADER_BUFFER_CAPACITY,
            pool_capacity: 4096,
            sqpoll_cpu: 0,
        }
    }
}

impl WorkerConfig {
    pub fn get(ring_size: u32, pool_capacity: usize, sqpoll_cpu: u32) -> Self {
        Self {
            initial_accepts: constants::INITIAL_ACCEPTS_PER_WORKER,
            ring_size,
            io_buffer_capacity: constants::IO_BUFFER_CAPACITY,
            header_buffer_capacity: constants::HEADER_BUFFER_CAPACITY,
            pool_capacity,
            sqpoll_cpu,
        }
    }
}
