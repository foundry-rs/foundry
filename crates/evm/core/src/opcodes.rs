//! Opcode utils

use revm::bytecode::opcode::OpCode;

/// Returns true if the opcode modifies memory.
/// <https://reth.rs/docs/reth_ethereum/evm/revm/revm/bytecode/opcode/struct.OpCode.html#method.modifies_memory>
/// <https://github.com/crytic/evm-opcodes>
#[inline]
pub const fn modifies_memory(opcode: OpCode) -> bool {
    matches!(
        opcode,
        OpCode::EXTCODECOPY |
            OpCode::MLOAD |
            OpCode::MSTORE |
            OpCode::MSTORE8 |
            OpCode::MCOPY |
            OpCode::CODECOPY |
            OpCode::CALLDATACOPY |
            OpCode::RETURNDATACOPY |
            OpCode::CALL |
            OpCode::CALLCODE |
            OpCode::DELEGATECALL |
            OpCode::STATICCALL
    )
}
