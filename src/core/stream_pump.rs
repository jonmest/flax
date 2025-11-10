use std::os::fd::RawFd;

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]

pub enum OpCode {
    Accept       = 1,
    ConnectBack  = 2,
    Recv         = 3,
    Send         = 4,
    Timeout      = 5,
    RecvHeaders  = 6,
}

impl OpCode {
    #[inline]
    pub fn try_from_u8(v: u8) -> Option<Self> {
        use OpCode::*;
        Some(match v {
            1 => Accept,
            2 => ConnectBack,
            3 => Recv,
            4 => Send,
            5 => Timeout,
            6 => RecvHeaders,
            _ => return None,
        })
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Direction {
    ClientToBackend = 0,
    BackendToClient = 1,
}

/// Public operation enum used by your code.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    Accept,
    ConnectBackend,
    Recv(Direction),
    Send(Direction),
    Timeout(Direction),
    RecvHeaders,
}

/// Single-direction forwarding state.
pub struct StreamPump {
    /// Source to read from (recv).
    pub read_fd: RawFd,
    /// Destination to write to (send).
    pub write_fd: RawFd,

    /// Fixed-size I/O buffer reused forever.
    pub buffer: Vec<u8>,

    /// Number of valid bytes currently in `buffer` (prefix).
    pub bytes_ready_to_send: usize,
    /// How many of those bytes have been sent so far (for partial sends).
    pub bytes_already_sent: usize,

    /// True if a Recv SQE is outstanding.
    pub recv_in_flight: bool,
    /// True if a Send SQE is outstanding.
    pub send_in_flight: bool,

    /// Optional body-length tracking (use only for Client→Backend).
    pub remaining_request_body_bytes: Option<usize>,
}

impl StreamPump {
    pub fn new(read_fd: RawFd, write_fd: RawFd, buffer_capacity: usize) -> Self {
        Self {
            read_fd,
            write_fd,
            buffer: vec![0u8; buffer_capacity],
            bytes_ready_to_send: 0,
            bytes_already_sent: 0,
            recv_in_flight: false,
            send_in_flight: false,
            remaining_request_body_bytes: None,
        }
    }

    /// True when there is nothing buffered and no ops outstanding.
    pub fn is_idle(&self) -> bool {
        !self.recv_in_flight && !self.send_in_flight && self.bytes_ready_to_send == 0
    }

    /// Reset buffer after a full send.
    pub fn reset_buffer(&mut self) {
        self.bytes_ready_to_send = 0;
        self.bytes_already_sent = 0;
    }
}
