use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    os::fd::RawFd,
};

use crate::util::fd::close_fd_quiet;

const MAX_CACHED: usize = 200;

pub struct BackendConnectionCache {
    map: HashMap<SocketAddr, VecDeque<RawFd>>,
}

// Not thread safe! To be used exclusively by thread.
impl BackendConnectionCache {
    pub fn new() -> Option<Self> {
        Some(Self {
            map: HashMap::new(),
        })
    }

    pub fn borrow_connection(&mut self, addr: &SocketAddr) -> Option<RawFd> {
        match self.map.get_mut(addr) {
            Some(deque) => deque.pop_front(),
            None => None,
        }
    }

    pub fn return_connection(&mut self, addr: &SocketAddr, fd: RawFd) {
        let deque = self.map.entry(*addr).or_default();

        if deque.len() < MAX_CACHED {
            deque.push_back(fd);
        } else {
            close_fd_quiet(fd);
        };
    }
}
