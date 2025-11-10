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

    // Non-blocking connect
    let connect_result = unsafe {
        libc::connect(pair.backend_fd, ptr, slen)
    };

    if connect_result < 0 {
        let err = io::Error::last_os_error();
        // EINPROGRESS (115) is expected for non-blocking connect
        if err.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(err);
        }
    }

    // Trigger handler - it will check if connection is ready
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
    if pump.send_in_flight {
        return;
    }
    if pump.bytes_already_sent >= pump.bytes_ready_to_send {
        return;
    }
    let ptr = unsafe { pump.buffer.as_mut_ptr().add(pump.bytes_already_sent) };
    let len = (pump.bytes_ready_to_send - pump.bytes_already_sent) as u32;

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
