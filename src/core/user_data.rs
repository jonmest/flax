/*
* IO_Uring accepts a 64-bit integer called user_data per submission to the submission queue.
* We want to use this to identify what kind of event completed. For our purposes, we want to store:
*
* 1. Which connection the event is for, this is specifically an index to the connection in our
*    Connection vector.
*
* 2. Which operation - Accept, Recv, Send, Connect, Timeout.
*
* 3. Maybe which direction - client-to-backend or backend-to-client.
*/

use crate::core::stream_pump::{Direction, Operation, OpCode};

const OPCODE_BITS: u64 = 7;
const DIR_BITS:    u64 = 1;
const ID_BITS:     u64 = 64 - (OPCODE_BITS + DIR_BITS);

const OPCODE_MASK: u64 = (1 << OPCODE_BITS) - 1;               // 0x7F
const DIR_MASK:    u64 = (1 << DIR_BITS) - 1;                  // 0x01
const ID_MASK:     u64 = (1 << ID_BITS) - 1;                   // 0x00FF_FFFF_FFFF_FFFF

const OPCODE_SHIFT: u64 = 0;
const DIR_SHIFT:    u64 = OPCODE_SHIFT + OPCODE_BITS;          // 7
const ID_SHIFT:     u64 = DIR_SHIFT + DIR_BITS;                // 8

#[inline]
pub fn pack_user_data(pair_id: usize, op: Operation) -> u64 {
    let (opcode, dir_bit): (OpCode, u8) = match op {
        Operation::Accept                  => (OpCode::Accept,      0),
        Operation::ConnectBackend          => (OpCode::ConnectBack, 0),
        Operation::Recv(Direction::ClientToBackend)    => (OpCode::Recv,    Direction::ClientToBackend as u8),
        Operation::Recv(Direction::BackendToClient)    => (OpCode::Recv,    Direction::BackendToClient as u8),
        Operation::Send(Direction::ClientToBackend)    => (OpCode::Send,    Direction::ClientToBackend as u8),
        Operation::Send(Direction::BackendToClient)    => (OpCode::Send,    Direction::BackendToClient as u8),
        Operation::Timeout(Direction::ClientToBackend) => (OpCode::Timeout, Direction::ClientToBackend as u8),
        Operation::Timeout(Direction::BackendToClient) => (OpCode::Timeout, Direction::BackendToClient as u8),
        Operation::RecvHeaders             => (OpCode::RecvHeaders, 0),
    };

    let id = pair_id as u64;
    debug_assert!((id & !ID_MASK) == 0, "pair_id exceeds 56 bits");

    ((id & ID_MASK)   << ID_SHIFT)
    | (((dir_bit as u64) & DIR_MASK) << DIR_SHIFT)
    | ((opcode as u64) & OPCODE_MASK)
}

#[inline]
pub fn unpack_user_data(tag: u64) -> (usize, Operation) {
    let id  = ((tag >> ID_SHIFT)  & ID_MASK)    as usize;
    let dir = ((tag >> DIR_SHIFT) & DIR_MASK)   as u8;
    let opc = ((tag >> OPCODE_SHIFT) & OPCODE_MASK) as u8;

    let op = match OpCode::try_from_u8(opc) {
        Some(OpCode::Accept)      => Operation::Accept,
        Some(OpCode::ConnectBack) => Operation::ConnectBackend,
        Some(OpCode::Recv)        => Operation::Recv(if dir == 0 { Direction::ClientToBackend } else { Direction::BackendToClient }),
        Some(OpCode::Send)        => Operation::Send(if dir == 0 { Direction::ClientToBackend } else { Direction::BackendToClient }),
        Some(OpCode::Timeout)     => Operation::Timeout(if dir == 0 { Direction::ClientToBackend } else { Direction::BackendToClient }),
        Some(OpCode::RecvHeaders) => Operation::RecvHeaders,
        None => {
            // TODO: Handle as error maybe?
            Operation::Accept
        }
    };

    (id, op)
}