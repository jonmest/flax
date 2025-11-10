pub const READ_BUF: usize = 4096;

// operation tags for low 8 bits of user_data
pub const OP_ACCEPT: u8 = 1;
pub const OP_CONNECT: u8 = 2;
pub const OP_RECV: u8 = 3;
pub const OP_SEND: u8 = 4;

pub const INITIAL_ACCEPTS_PER_WORKER: usize = 8;
pub const IO_BUFFER_CAPACITY: usize = 32 * 1024;
pub const HEADER_BUFFER_CAPACITY: usize = 8 * 1024;