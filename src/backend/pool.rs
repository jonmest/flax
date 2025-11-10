use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backend {
    pub address: SocketAddr,
}

impl Backend {
    pub fn new(address: SocketAddr) -> Self {
        Self { address }
    }
}

#[derive(Debug)]
pub struct BackendPool {
    backends: RwLock<Vec<Backend>>,
    counter: AtomicUsize,
}

impl BackendPool {
    pub fn new(backends: Vec<Backend>) -> Self {
        Self {
            backends: RwLock::new(backends),
            counter: AtomicUsize::new(0),
        }
    }

    pub fn select(&self) -> Option<SocketAddr> {
        let backends = self.backends.read().unwrap();
        if backends.is_empty() {
            return None;
        }
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % backends.len();
        Some(backends[idx].address)
    }

    pub fn add_backend(&self, backend: Backend) {
        let mut backends = self.backends.write().unwrap();
        backends.push(backend);
    }

    pub fn remove_backend(&self, address: SocketAddr) -> bool {
        let mut backends = self.backends.write().unwrap();
        if let Some(pos) = backends.iter().position(|b| b.address == address) {
            backends.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn list_backends(&self) -> Vec<SocketAddr> {
        let backends = self.backends.read().unwrap();
        backends.iter().map(|b| b.address).collect()
    }

    pub fn count(&self) -> usize {
        self.backends.read().unwrap().len()
    }

    pub fn clear(&self) {
        let mut backends = self.backends.write().unwrap();
        backends.clear();
    }
}

static POOL: OnceLock<BackendPool> = OnceLock::new();

pub fn init_backend_pool(backends: Vec<Backend>) {
    POOL.set(BackendPool::new(backends))
        .expect("Backend pool already initialized");
}

pub fn get_backend_pool() -> &'static BackendPool {
    POOL.get().expect("Backend pool not initialized - call init_backend_pool first")
}

pub fn select_backend() -> SocketAddr {
    get_backend_pool()
        .select()
        .unwrap_or_else(|| "127.0.0.1:8081".parse().unwrap())
}
