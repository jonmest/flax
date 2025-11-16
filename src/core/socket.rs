use libc::{sockaddr_in6, sockaddr_storage};
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{IntoRawFd, RawFd};

/// Socket utility functions for load balancer
///
/// This module provides low-level socket operations including:
/// - Backend connection socket creation
/// - SO_REUSEPORT listener setup for multi-core workers

pub fn make_backend_socket(
    addr: SocketAddr,
) -> io::Result<(RawFd, Box<sockaddr_storage>, libc::socklen_t)> {
    let (domain, (storage, len)) = match addr {
        SocketAddr::V4(_) => {
            let sock_addr: socket2::SockAddr = addr.into();
            let mut ss: sockaddr_storage = unsafe { std::mem::zeroed() };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sock_addr.as_ptr() as *const u8,
                    &mut ss as *mut _ as *mut u8,
                    sock_addr.len() as usize,
                );
            }
            (Domain::IPV4, (Box::new(ss), sock_addr.len()))
        }
        SocketAddr::V6(a6) => {
            let mut st: sockaddr_in6 = unsafe { std::mem::zeroed() };
            st.sin6_family = libc::AF_INET6 as u16;
            st.sin6_port = a6.port().to_be();
            st.sin6_addr = libc::in6_addr {
                s6_addr: a6.ip().octets(),
            };
            st.sin6_scope_id = a6.scope_id();
            let mut ss: sockaddr_storage = unsafe { std::mem::zeroed() };
            unsafe {
                std::ptr::write(&mut ss as *mut _ as *mut sockaddr_in6, st);
            }
            (
                Domain::IPV6,
                (
                    Box::new(ss),
                    std::mem::size_of::<sockaddr_in6>() as libc::socklen_t,
                ),
            )
        }
    };

    let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_nonblocking(true)?; // MUST be non-blocking for io_uring
    let fd = sock.into_raw_fd();

    Ok((fd, storage, len))
}

/// Create a SO_REUSEPORT listening socket
///
/// This creates a TCP listener with SO_REUSEPORT enabled, allowing
/// multiple workers to bind to the same address for load distribution.
///
/// Each worker gets its own listener, and the kernel distributes incoming
/// connections across all workers bound to the same port.
pub fn make_reuseport_listener(addr: SocketAddr) -> io::Result<TcpListener> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_reuse_address(true)?;
    sock.set_reuse_port(true)?;
    sock.bind(&addr.into())?;
    sock.listen(1024)?;
    Ok(sock.into())
}
