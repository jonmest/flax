use io_uring::{IoUring, opcode, types};
use std::io;
use std::io::Result;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, RawFd};
use std::ptr;

enum State {
    Accept,
    Recv,
    Send,
    Closed,
}

struct Connection {
    id: usize,
    state: State,
    fd: RawFd,
    buf: Vec<u8>,
    len: usize,
}

fn main() -> io::Result<()> {
    let mut ring = IoUring::new(8)?;
    let listener = TcpListener::bind(("127.0.0.1", 3000))?;
    listener.set_nonblocking(true)?;
    let lfd = listener.as_raw_fd();

    let accept_sqe = opcode::Accept::new(types::Fd(lfd), ptr::null_mut(), ptr::null_mut())
        .build()
        .user_data(1);
    unsafe {
        ring.submission().push(&accept_sqe).unwrap();
    }

    let mut buffer = vec![0u8; 4096];

    loop {
        ring.submit_and_wait(1)?;
        let cqe = ring.completion().next().unwrap();

        match cqe.user_data() {
            1 => {
                // Accept completed
                let conn_fd = cqe.result();
                if conn_fd < 0 {
                    continue;
                }
                println!("Accepted fd={}", conn_fd);

                let recv =
                    opcode::Recv::new(types::Fd(conn_fd), buffer.as_mut_ptr(), buffer.len() as _)
                        .build()
                        .user_data(2);
                unsafe {
                    ring.submission().push(&recv).unwrap();
                }
            }
            2 => {
                // Receive is done
                let n = cqe.result();
                if n <= 0 {
                    continue;
                }
                println!("Received {} bytes", n);
                let send = opcode::Send::new(types::Fd(3), buffer.as_ptr(), n as _)
                    .build()
                    .user_data(3);
                unsafe {
                    ring.submission().push(&send).unwrap();
                }
            }
            _ => {}
        }
    }

    Ok(())
}
