use alloy_primitives::U256;
use revm::interpreter::opcode;

/// Used to keep track of which buffer is currently active to be drawn by the debugger.
#[derive(Debug, PartialEq)]
pub enum BufferKind {
    Memory,
    Calldata,
    Returndata,
}

impl BufferKind {
    /// Helper to cycle through the active buffers.
    pub fn next(&self) -> Self {
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
    let buffer_access = match op {
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
        opcode::RETURNDATALOAD => (Some((BufferKind::Returndata, 1, -1)), None),
        opcode::EOFCREATE => (Some((BufferKind::Memory, 3, 4)), None),
        opcode::RETURNCONTRACT => (Some((BufferKind::Memory, 1, 2)), None),
        opcode::DATACOPY => (None, Some((1, 3))),
        opcode::EXTCALL | opcode::EXTSTATICCALL | opcode::EXTDELEGATECALL => {
            (Some((BufferKind::Memory, 2, 3)), None)
        }
        _ => Default::default(),
    };

    let stack_len = stack.len();
    let get_size = |stack_index| match stack_index {
        -2 => Some(1),
        -1 => Some(32),
        0 => None,
        1.. => {
            if (stack_index as usize) <= stack_len {
                Some(stack[stack_len - stack_index as usize].saturating_to())
            } else {
                None
            }
        }
        _ => panic!("invalid stack index"),
    };

    if buffer_access.0.is_some() || buffer_access.1.is_some() {
        let (read, write) = buffer_access;
        let read_access = read.and_then(|b| {
            let (buffer, offset, len) = b;
            Some((buffer, BufferAccess { offset: get_size(offset)?, len: get_size(len)? }))
        });
        let write_access = write.and_then(|b| {
            let (offset, len) = b;
            Some(BufferAccess { offset: get_size(offset)?, len: get_size(len)? })
        });
        Some(BufferAccesses { read: read_access, write: write_access })
    } else {
        None
    }
}
