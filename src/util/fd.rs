use std::{io, os::fd::RawFd};

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
