use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::{io, ptr};

use io_uring::{opcode, types, IoUring};

use crate::core::connection_pair::ConnectionPair;
use crate::core::socket::make_backend_socket;
use crate::core::stream_pump::{Operation, StreamPump};
use crate::core::user_data::pack_user_data;

/// io_uring SQE submission helpers
///
/// These functions submit various operations to the io_uring submission queue.
/// They handle the low-level details of creating SQEs with proper user_data tagging.

/// Post an accept operation for a new client connection
pub fn post_accept(ring: &mut IoUring, listen_fd: RawFd, pair_id: usize) {
    let sqe = opcode::Accept::new(types::Fd(listen_fd), ptr::null_mut(), ptr::null_mut())
        .build()
        .user_data(pack_user_data(pair_id, Operation::Accept));
    unsafe {
        ring.submission()
            .push(&sqe)
            .expect("SQ full (accept)");
    }
}

/// Post a recv operation to read HTTP headers from the client
pub fn post_recv_headers(ring: &mut IoUring, pair: &mut ConnectionPair) {
    let (ptr, len) = pair.header_buffer.write_ptr_len();
    let sqe = opcode::Recv::new(types::Fd(pair.client_fd), ptr, len as u32)
        .build()
        .user_data(pack_user_data(pair.id, Operation::RecvHeaders));
    unsafe {
        ring.submission()
            .push(&sqe)
            .expect("SQ full (recv headers)");
    }
}

/// Post a connect operation to establish backend connection
///
/// Uses blocking connect (instant for localhost), then switches to non-blocking for io_uring.
pub fn post_connect_backend(
    ring: &mut IoUring,
    pair: &mut ConnectionPair,
    backend_addr: SocketAddr,
) -> io::Result<()> {
    let (backend_fd, storage, slen) = make_backend_socket(backend_addr)?;
    pair.attach_backend_socket(backend_fd);

    // Store sockaddr first so we can get a stable pointer
    pair.set_backend_sockaddr(storage, slen);

    // Get pointer to the stored sockaddr (guaranteed to remain valid)
    let ptr = pair
        .backend_sockaddr_storage
        .as_ref()
        .unwrap()
        .as_ref() as *const _ as *const libc::sockaddr;

    eprintln!("[DEBUG CONNECT] Connecting to {}", backend_addr);

    // Blocking connect (instant for localhost)
    let connect_result = unsafe {
        libc::connect(pair.backend_fd, ptr, slen)
    };

    if connect_result < 0 {
        let err = io::Error::last_os_error();
        eprintln!("[DEBUG CONNECT] Connect failed: {:?}", err);
        return Err(err);
    }

    eprintln!("[DEBUG CONNECT] Connected! Setting non-blocking");

    // Now set non-blocking for io_uring operations
    unsafe {
        let flags = libc::fcntl(pair.backend_fd, libc::F_GETFL);
        libc::fcntl(pair.backend_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    // Trigger handler
    let sqe = opcode::Nop::new()
        .build()
        .user_data(pack_user_data(pair.id, Operation::ConnectBackend));
    unsafe {
        ring.submission()
            .push(&sqe)
            .expect("SQ full (nop)");
    }

    Ok(())
}

/// Post a recv operation on a stream pump
///
/// This receives data from the source FD into the pump's buffer.
/// Only posts if there's free space and no recv is already in flight.
pub fn post_recv_pump(
    ring: &mut IoUring,
    pair_id: usize,
    pump: &mut StreamPump,
    tag: Operation,
) {
    if pump.recv_in_flight {
        return;
    }
    let free = pump.buffer.len() - pump.bytes_ready_to_send;
    if free == 0 {
        return;
    }
    let ptr = unsafe { pump.buffer.as_mut_ptr().add(pump.bytes_ready_to_send) };
    let len = free as u32;
    let sqe = opcode::Recv::new(types::Fd(pump.read_fd), ptr, len)
        .build()
        .user_data(pack_user_data(pair_id, tag));
    pump.recv_in_flight = true;
    unsafe {
        ring.submission()
            .push(&sqe)
            .expect("SQ full (recv pump)");
    }
}

/// Post a send operation on a stream pump
///
/// This sends buffered data to the destination FD.
/// Only posts if there's data to send and no send is already in flight.
pub fn post_send_pump(
    ring: &mut IoUring,
    pair_id: usize,
    pump: &mut StreamPump,
    tag: Operation,
) {
    eprintln!("[DEBUG post_send_pump] pair_id={}, write_fd={}, send_in_flight={}, bytes_ready={}, bytes_sent={}",
              pair_id, pump.write_fd, pump.send_in_flight, pump.bytes_ready_to_send, pump.bytes_already_sent);

    if pump.send_in_flight {
        eprintln!("[DEBUG post_send_pump] Send already in flight, skipping");
        return;
    }
    if pump.bytes_already_sent >= pump.bytes_ready_to_send {
        eprintln!("[DEBUG post_send_pump] No data to send, skipping");
        return;
    }
    let ptr = unsafe { pump.buffer.as_mut_ptr().add(pump.bytes_already_sent) };
    let len = (pump.bytes_ready_to_send - pump.bytes_already_sent) as u32;

    eprintln!("[DEBUG post_send_pump] Submitting send: fd={}, len={}", pump.write_fd, len);

    let sqe = opcode::Send::new(types::Fd(pump.write_fd), ptr, len)
        .build()
        .user_data(pack_user_data(pair_id, tag));
    pump.send_in_flight = true;
    unsafe {
        ring.submission()
            .push(&sqe)
            .expect("SQ full (send pump)");
    }
}
