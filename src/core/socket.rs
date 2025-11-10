use libc::{sockaddr_in, sockaddr_in6, sockaddr_storage};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{IntoRawFd, RawFd};
use std::io;

/// Socket utility functions for load balancer
///
/// This module provides low-level socket operations including:
/// - Backend connection socket creation
/// - SO_REUSEPORT listener setup for multi-core workers


pub fn make_backend_socket(addr: SocketAddr) -> io::Result<(RawFd, Box<sockaddr_storage>, libc::socklen_t)> {
    let (domain, (storage, len)) = match addr {
        SocketAddr::V4(a4) => {
            let mut st: sockaddr_in = unsafe { std::mem::zeroed() };
            st.sin_family = libc::AF_INET as u16;
            st.sin_port = a4.port().to_be();
            // s_addr expects network byte order (big-endian)
            st.sin_addr = libc::in_addr { s_addr: u32::from_be_bytes(a4.ip().octets()) };
            let mut ss: sockaddr_storage = unsafe { std::mem::zeroed() };
            unsafe { std::ptr::write(&mut ss as *mut _ as *mut sockaddr_in, st); }

            eprintln!("[DEBUG SOCKET] Creating IPv4 socket for {}:{}", a4.ip(), a4.port());
            eprintln!("[DEBUG SOCKET] sin_family={}, sin_port={:#x}, sin_addr.s_addr={:#x}",
                      st.sin_family, st.sin_port, st.sin_addr.s_addr);

            (Domain::IPV4, (Box::new(ss), std::mem::size_of::<sockaddr_in>() as libc::socklen_t))
        }
        SocketAddr::V6(a6) => {
            let mut st: sockaddr_in6 = unsafe { std::mem::zeroed() };
            st.sin6_family = libc::AF_INET6 as u16;
            st.sin6_port = a6.port().to_be();
            st.sin6_addr = libc::in6_addr { s6_addr: a6.ip().octets() };
            st.sin6_scope_id = a6.scope_id();
            let mut ss: sockaddr_storage = unsafe { std::mem::zeroed() };
            unsafe { std::ptr::write(&mut ss as *mut _ as *mut sockaddr_in6, st); }
            (Domain::IPV6, (Box::new(ss), std::mem::size_of::<sockaddr_in6>() as libc::socklen_t))
        }
    };

    let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    // Keep socket blocking for connect - localhost is instant anyway
    // sock.set_nonblocking(true)?;
    sock.set_nodelay(true)?;
    let fd = sock.into_raw_fd();

    eprintln!("[DEBUG SOCKET] Created socket fd={}, len={}", fd, len);

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
    sock.set_reuse_port(true)?; // requires socket2 = { version="0.6", features=["all"] }
    sock.bind(&addr.into())?;
    sock.listen(1024)?;
    Ok(sock.into())
}
