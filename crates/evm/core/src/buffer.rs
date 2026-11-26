use alloy_primitives::U256;
use evm2::interpreter::opcode::op as opcode;

/// Used to keep track of which buffer is currently active to be drawn by the debugger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    Memory,
    Calldata,
    Returndata,
}

impl BufferKind {
    /// Helper to cycle through the active buffers.
    pub const fn next(&self) -> Self {
        match self {
            Self::Memory => Self::Calldata,
            Self::Calldata => Self::Returndata,
            Self::Returndata => Self::Memory,
        }
    }

    /// Helper to format the title of the active buffer pane
    pub fn title(&self, size: usize) -> String {
        match self {
            Self::Memory => format!("Memory (max expansion: {size} bytes)"),
            Self::Calldata => format!("Calldata (size: {size} bytes)"),
            Self::Returndata => format!("Returndata (size: {size} bytes)"),
        }
    }
}

/// Container for buffer access information.
pub struct BufferAccess {
    pub offset: usize,
    pub len: usize,
}

/// Container for read and write buffer access information.
pub struct BufferAccesses {
    /// The read buffer kind and access information.
    pub read: Option<(BufferKind, BufferAccess)>,
    /// The only mutable buffer is the memory buffer, so don't store the buffer kind.
    pub write: Option<BufferAccess>,
}

/// A utility function to get the buffer access.
///
/// The memory_access variable stores the index on the stack that indicates the buffer
/// offset/len accessed by the given opcode:
///    (read buffer, buffer read offset, buffer read len, write memory offset, write memory len)
///    \>= 1: the stack index
///    0: no memory access
///    -1: a fixed len of 32 bytes
///    -2: a fixed len of 1 byte
///
/// The return value is a tuple about accessed buffer region by the given opcode:
///    (read buffer, buffer read offset, buffer read len, write memory offset, write memory len)
pub fn get_buffer_accesses(op: u8, stack: &[U256]) -> Option<BufferAccesses> {
    let (read, write) = match op {
        opcode::KECCAK256 | opcode::RETURN | opcode::REVERT => {
            (Some((BufferKind::Memory, 1, 2)), None)
        }
        opcode::CALLDATACOPY => (Some((BufferKind::Calldata, 2, 3)), Some((1, 3))),
        opcode::RETURNDATACOPY => (Some((BufferKind::Returndata, 2, 3)), Some((1, 3))),
        opcode::CALLDATALOAD => (Some((BufferKind::Calldata, 1, -1)), None),
        opcode::CODECOPY => (None, Some((1, 3))),
        opcode::EXTCODECOPY => (None, Some((2, 4))),
        opcode::MLOAD => (Some((BufferKind::Memory, 1, -1)), None),
        opcode::MSTORE => (None, Some((1, -1))),
        opcode::MSTORE8 => (None, Some((1, -2))),
        opcode::LOG0 | opcode::LOG1 | opcode::LOG2 | opcode::LOG3 | opcode::LOG4 => {
            (Some((BufferKind::Memory, 1, 2)), None)
        }
        opcode::CREATE | opcode::CREATE2 => (Some((BufferKind::Memory, 2, 3)), None),
        opcode::CALL | opcode::CALLCODE => (Some((BufferKind::Memory, 4, 5)), None),
        opcode::DELEGATECALL | opcode::STATICCALL => (Some((BufferKind::Memory, 3, 4)), None),
        opcode::MCOPY => (Some((BufferKind::Memory, 2, 3)), Some((1, 3))),
        _ => return None,
    };

    let get_size = |stack_index| match stack_index {
        -2 => Some(1),
        -1 => Some(32),
        0 => None,
        1.. => stack.iter().rev().nth(stack_index as usize - 1).map(|word| word.saturating_to()),
        _ => panic!("invalid stack index"),
    };

    let read = read.and_then(|(buffer, offset, len)| {
        Some((buffer, BufferAccess { offset: get_size(offset)?, len: get_size(len)? }))
    });
    let write = write.and_then(|(offset, len)| {
        Some(BufferAccess { offset: get_size(offset)?, len: get_size(len)? })
    });
    Some(BufferAccesses { read, write })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_accesses_read_the_stack_from_the_top() {
        // The top of the stack is the last item: `MCOPY(dest = 7, src = 5, len = 3)`.
        let stack = [3, 5, 7].map(U256::from);
        let accesses = get_buffer_accesses(opcode::MCOPY, &stack).unwrap();
        let (kind, read) = accesses.read.unwrap();
        assert_eq!((kind, read.offset, read.len), (BufferKind::Memory, 5, 3));
        let write = accesses.write.unwrap();
        assert_eq!((write.offset, write.len), (7, 3));

        // `MSTORE8` writes one byte at the top of the stack.
        let write = get_buffer_accesses(opcode::MSTORE8, &stack).unwrap().write.unwrap();
        assert_eq!((write.offset, write.len), (7, 1));
    }

    #[test]
    fn buffer_accesses_drop_what_the_stack_cannot_give() {
        // A short stack leaves the accesses empty, and other opcodes have none.
        let accesses = get_buffer_accesses(opcode::MCOPY, &[U256::from(1)]).unwrap();
        assert!(accesses.read.is_none() && accesses.write.is_none());
        assert!(get_buffer_accesses(opcode::ADD, &[U256::from(1), U256::from(2)]).is_none());
    }
}
