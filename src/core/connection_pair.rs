use std::net::SocketAddr;
use std::os::fd::RawFd;

use libc::sockaddr_storage;

use crate::core::stream_pump::StreamPump;
use crate::protocol::HttpBuf;

pub struct ConnectionPair {
    pub id: usize,
    pub client_fd: RawFd,
    pub backend_fd: RawFd,

    pub backend_address: Option<SocketAddr>,
    pub backend_sockaddr_storage: Option<Box<sockaddr_storage>>,
    pub backend_sockaddr_len: libc::socklen_t,

    pub header_buffer: HttpBuf,

    pub pump_client_to_backend: StreamPump,
    pub pump_backend_to_client: StreamPump,

    pub request_content_length: Option<usize>,
    pub request_transfer_encoding_chunked: bool,
    pub had_error: bool,
}

impl ConnectionPair {
    pub fn new_with_client(
        id: usize,
        client_fd: RawFd,
        io_buffer_capacity: usize,
        header_buffer_capacity: usize,
    ) -> Self {
        Self {
            id,
            client_fd,
            backend_address: None,
            backend_fd: -1,
            backend_sockaddr_storage: None,
            backend_sockaddr_len: 0,

            header_buffer: HttpBuf::with_capacity(header_buffer_capacity),

            pump_client_to_backend: StreamPump::new(client_fd, -1, io_buffer_capacity),
            pump_backend_to_client: StreamPump::new(-1, client_fd, io_buffer_capacity),

            request_content_length: None,
            request_transfer_encoding_chunked: false,
            had_error: false,
        }
    }

    /// Called after creating the backend socket fd (nonblocking) but before posting Connect.
    pub fn attach_backend_socket(&mut self, backend_fd: RawFd) {
        self.backend_fd = backend_fd;
        self.pump_client_to_backend.write_fd = backend_fd;
        self.pump_backend_to_client.read_fd = backend_fd;
    }

    /// Store sockaddr backing memory and length so the pointer stays valid during Connect.
    pub fn set_backend_sockaddr(&mut self, storage: Box<sockaddr_storage>, len: libc::socklen_t) {
        self.backend_sockaddr_storage = Some(storage);
        self.backend_sockaddr_len = len;
    }

    /// Called once Connect completes successfully to arm both directions.
    pub fn start_streaming(&mut self) {
        // Now that connect is done, both fds are valid in both pumps.
        self.pump_client_to_backend.read_fd = self.client_fd;
        self.pump_client_to_backend.write_fd = self.backend_fd;

        self.pump_backend_to_client.read_fd = self.backend_fd;
        self.pump_backend_to_client.write_fd = self.client_fd;
    }
}
