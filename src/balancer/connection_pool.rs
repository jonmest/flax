use std::collections::VecDeque;
use std::os::fd::RawFd;

use crate::core::connection_pair::ConnectionPair;
use crate::protocol::HttpBuf;
use crate::util::fd::close_fd_quiet;

/// Connection pool using slab allocation with a freelist
///
/// This manages a pool of ConnectionPair objects, reusing slots
/// to avoid allocations in the hot path.
pub struct ConnectionPool {
    pairs: Vec<Option<ConnectionPair>>,
    freelist: VecDeque<usize>,
    io_buffer_capacity: usize,
    header_buffer_capacity: usize,
}

impl ConnectionPool {
    pub fn new(
        initial_capacity: usize,
        io_buffer_capacity: usize,
        header_buffer_capacity: usize,
    ) -> Self {
        Self {
            pairs: Vec::with_capacity(initial_capacity),
            freelist: VecDeque::new(),
            io_buffer_capacity,
            header_buffer_capacity,
        }
    }

    pub fn alloc(&mut self) -> usize {
        if let Some(id) = self.freelist.pop_front() {
            return id;
        }
        let id = self.pairs.len();
        self.pairs.push(None);
        id
    }

    pub fn ensure_slot(&mut self, id: usize, client_fd: RawFd) {
        if id >= self.pairs.len() {
            self.pairs.resize_with(id + 1, || None);
        }
        let mut p = ConnectionPair::new_with_client(
            id,
            client_fd,
            self.io_buffer_capacity,
            self.header_buffer_capacity,
        );
        p.header_buffer = HttpBuf::with_capacity(self.header_buffer_capacity);
        self.pairs[id] = Some(p);
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut ConnectionPair> {
        self.pairs.get_mut(id).and_then(|p| p.as_mut())
    }

    pub fn teardown(&mut self, id: usize) {
        if let Some(p) = self.pairs.get_mut(id).and_then(|p| p.take()) {
            if p.client_fd >= 0 {
                close_fd_quiet(p.client_fd);
            }
            if p.backend_fd >= 0 {
                close_fd_quiet(p.backend_fd);
            }
            self.freelist.push_back(id);
        }
    }

    pub fn recycle_slot_only(&mut self, id: usize) {
        if let Some(slot) = self.pairs.get_mut(id)
            && let Some(p) = slot.take()
            && p.client_fd >= 0
        {
            close_fd_quiet(p.client_fd);
        }
    }

    pub fn pairs_mut(&mut self) -> &mut Vec<Option<ConnectionPair>> {
        &mut self.pairs
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}
