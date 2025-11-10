use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
    os::fd::RawFd,
};

pub fn close_fd(fd: RawFd) -> io::Result<()> {
    let ret = unsafe { libc::close(fd) };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn close_fd_quiet(fd: RawFd) {
    // After this call, consider fd dead in all code paths.
    let ret = unsafe { libc::close(fd) };
    if ret != 0 {
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EBADF) => {
                eprintln!("close({fd}) -> EBADF (double close / invalid fd)");
            }
            Some(libc::EINTR) => {
                eprintln!("close({fd}) interrupted by signal (EINTR); not retrying");
            }
            _ => {
                eprintln!("close({fd}) failed: {err}");
            }
        }
    }
}

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
