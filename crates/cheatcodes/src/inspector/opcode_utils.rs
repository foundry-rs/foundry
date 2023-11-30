/// Utility functions for opcode related stuff

use revm::{
    interpreter::{
        opcode, Stack, InstructionResult, SharedMemory
    }
};
use alloy_primitives::{U256};

pub(crate) enum OpcodeError {
    UnrecognizedOpcode,
    StackError(InstructionResult),
}

impl From<InstructionResult> for OpcodeError {
    fn from(err: InstructionResult) -> Self {
        OpcodeError::StackError(err)
    }
}

pub(crate) fn get_memory_input_for_opcode(
    opcode: u8,
    stack_inputs: &[U256],
    shared_memory: &SharedMemory
) -> Vec<u8> {
    match opcode {
        0x20 => { // KECCAK256 (SHA3)
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0x51 => { // MLOAD
            let offset = stack_inputs[0].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, 32)
        },
        0x52 => { // MSTORE
            // This opcode does not return data, it only writes to memory
            vec![]
        },
        0x53 => { // MSTORE8
            // This opcode also does not return data, it only writes to memory
            vec![]
        },
        0xA0 => { // LOG0
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xA1 => { // LOG1
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xA2 => { // LOG2
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xA3 => { // LOG3
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xA4 => { // LOG4
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xF0 => { // CREATE
            let offset = stack_inputs[1].to::<u64>() as usize;
            let len = stack_inputs[2].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xF1 => { // CALL
            let args_offset = stack_inputs[3].to::<u64>() as usize;
            let args_len = stack_inputs[4].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, args_offset, args_len)
        },
        0xF2 => { // CALLCODE
            let args_offset = stack_inputs[3].to::<u64>() as usize;
            let args_len = stack_inputs[4].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, args_offset, args_len)
        },
        0xF3 => { // RETURN
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xF4 => { // DELEGATECALL
            let args_offset = stack_inputs[2].to::<u64>() as usize;
            let args_len = stack_inputs[3].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, args_offset, args_len)
        },
        0xF5 => { // CREATE2
            let offset = stack_inputs[1].to::<u64>() as usize;
            let len = stack_inputs[2].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },
        0xFA => { // STATICCALL
            let args_offset = stack_inputs[2].to::<u64>() as usize;
            let args_len = stack_inputs[3].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, args_offset, args_len)
        },
        0xFD => { // REVERT
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(shared_memory, offset, len)
        },

        // Other opcodes...
        _ => vec![], // For unrecognized opcodes
    }
}

pub(crate) fn get_stack_inputs_for_opcode(opcode: u8, stack: &Stack) -> Result<Vec<U256>, OpcodeError> {
    match opcode {
        opcode::STOP => Ok(vec![]),
        opcode::ADD | opcode::MUL | opcode::SUB | opcode::DIV |
        opcode::SDIV | opcode::MOD | opcode::SMOD => {
            // These opcodes require two inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::ADDMOD | opcode::MULMOD => {
            // These require three inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::EXP => {
            // EXP requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SIGNEXTEND => {
            // SIGNEXTEND requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        // Skipping undefined opcodes 0x0C - 0x0F
        opcode::LT => {
            // LT requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::GT => {
            // GT requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SLT => {
            // SLT requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SGT => {
            // SGT requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::EQ => {
            // EQ requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::ISZERO => {
            // ISZERO requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::AND => {
            // AND requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::OR => {
            // OR requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::XOR => {
            // XOR requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::NOT => {
            // NOT requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::BYTE => {
            // BYTE requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SHL => {
            // SHL (Shift Left) requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SHR => {
            // SHR (Shift Right) requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SAR => {
            // SAR (Arithmetic Shift Right) requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        // Skipping undefined opcodes 0x1E - 0x1F
        opcode::KECCAK256 => {
            // KECCAK256 requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        // Skipping undefined opcodes 0x21 - 0x2F
        opcode::ADDRESS => {
            // ADDRESS does not require stack inputs
            Ok(vec![])
        },
        opcode::BALANCE => {
            // BALANCE requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::ORIGIN => {
            // ORIGIN does not require stack inputs
            Ok(vec![])
        },
        opcode::CALLER => {
            // CALLER does not require stack inputs
            Ok(vec![])
        },
        opcode::CALLVALUE => {
            // CALLVALUE does not require stack inputs
            Ok(vec![])
        },
        opcode::CALLDATALOAD => {
            // CALLDATALOAD requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::CALLDATASIZE => {
            // CALLDATASIZE does not require stack inputs
            Ok(vec![])
        },
        opcode::CALLDATACOPY => {
            // CALLDATACOPY requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::CODESIZE => {
            // CODESIZE does not require stack inputs
            Ok(vec![])
        },
        opcode::CODECOPY => {
            // CODECOPY requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::GASPRICE => {
            // GASPRICE does not require stack inputs
            Ok(vec![])
        },
        opcode::EXTCODESIZE => {
            // EXTCODESIZE requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::EXTCODECOPY => {
            // EXTCODECOPY requires four stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            let d = stack.peek(3)?;
            Ok(vec![a, b, c, d])
        },
        opcode::RETURNDATASIZE => {
            // RETURNDATASIZE does not require stack inputs
            Ok(vec![])
        },
        opcode::RETURNDATACOPY => {
            // RETURNDATACOPY requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::EXTCODEHASH => {
            // EXTCODEHASH requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::BLOCKHASH => {
            // BLOCKHASH requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::COINBASE => {
            // COINBASE does not require stack inputs
            Ok(vec![])
        },
        opcode::TIMESTAMP => {
            // TIMESTAMP does not require stack inputs
            Ok(vec![])
        },
        opcode::NUMBER => {
            // NUMBER does not require stack inputs
            Ok(vec![])
        },
        opcode::DIFFICULTY => {
            // DIFFICULTY does not require stack inputs
            Ok(vec![])
        },
        opcode::GASLIMIT => {
            // GASLIMIT does not require stack inputs
            Ok(vec![])
        },
        opcode::CHAINID => {
            // CHAINID does not require stack inputs
            Ok(vec![])
        },
        opcode::SELFBALANCE => {
            // SELFBALANCE does not require stack inputs
            Ok(vec![])
        },
        opcode::BASEFEE => {
            // BASEFEE does not require stack inputs
            Ok(vec![])
        },
        opcode::POP => {
            // POP requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::MLOAD => {
            // MLOAD requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::MSTORE => {
            // MSTORE requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::MSTORE8 => {
            // MSTORE8 requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::SLOAD => {
            // SLOAD requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::SSTORE => {
            // SSTORE requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::JUMP => {
            // JUMP requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::JUMPI => {
            // JUMPI requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::PC => {
            // PC does not require stack inputs
            Ok(vec![])
        },
        opcode::MSIZE => {
            // MSIZE does not require stack inputs
            Ok(vec![])
        },
        opcode::GAS => {
            // GAS does not require stack inputs
            Ok(vec![])
        },
        opcode::JUMPDEST => {
            // JUMPDEST does not require stack inputs
            Ok(vec![])
        },
        opcode::TLOAD => {
            // TLOAD requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },
        opcode::TSTORE => {
            // TSTORE requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::MCOPY => {
            // MCOPY requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },

        // PUSH1 to PUSH32 (0x60 to 0x7F) are not included here as they have varying behaviors

        // DUP1 to DUP16 (0x80 to 0x8F)
        opcode::DUP1 | opcode::DUP2 | opcode::DUP3 | opcode::DUP4 | opcode::DUP5 |
        opcode::DUP6 | opcode::DUP7 | opcode::DUP8 | opcode::DUP9 | opcode::DUP10 => {
            // Each DUP opcode requires N stack inputs where N is its own number
            let mut inputs = Vec::new();
            for i in 0..(opcode as usize - opcode::DUP1 as usize + 1) {
                inputs.push(stack.peek(i)?);
            }
            Ok(inputs)
        },

        // SWAP opcodes (0x90 to 0x9F)
        opcode::SWAP1 | opcode::SWAP2 | opcode::SWAP3 | opcode::SWAP4 | opcode::SWAP5 |
        opcode::SWAP6 | opcode::SWAP7 | opcode::SWAP8 | opcode::SWAP9 | opcode::SWAP10 => {
            let mut inputs = Vec::new();
            for i in 0..(opcode as usize - opcode::SWAP1 as usize + 2) {
                inputs.push(stack.peek(i)?);
            }
            Ok(inputs)
        },

        // LOG opcodes (0xA0 to 0xA4)
        opcode::LOG0 => {
            // LOG0 requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::LOG1 => {
            // LOG1 requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::LOG2 => {
            // LOG2 requires four stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            let d = stack.peek(3)?;
            Ok(vec![a, b, c, d])
        },
        opcode::LOG3 => {
            // LOG3 requires five stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            let d = stack.peek(3)?;
            let e = stack.peek(4)?;
            Ok(vec![a, b, c, d, e])
        },
        opcode::LOG4 => {
            // LOG4 requires six stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            let d = stack.peek(3)?;
            let e = stack.peek(4)?;
            let f = stack.peek(5)?;
            Ok(vec![a, b, c, d, e, f])
        },
        opcode::CREATE => {
            // CREATE requires three stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            Ok(vec![a, b, c])
        },
        opcode::CALL => {
            // CALL requires seven stack inputs
            let inputs = (0..7).map(|i| stack.peek(i)).collect::<Result<Vec<_>, _>>()?;
            Ok(inputs)
        },
        opcode::CALLCODE => {
            // CALLCODE requires seven stack inputs
            let inputs = (0..7).map(|i| stack.peek(i)).collect::<Result<Vec<_>, _>>()?;
            Ok(inputs)
        },
        opcode::RETURN => {
            // RETURN requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::DELEGATECALL => {
            // DELEGATECALL requires six stack inputs
            let inputs = (0..6).map(|i| stack.peek(i)).collect::<Result<Vec<_>, _>>()?;
            Ok(inputs)
        },
        opcode::CREATE2 => {
            // CREATE2 requires four stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            let c = stack.peek(2)?;
            let d = stack.peek(3)?;
            Ok(vec![a, b, c, d])
        },
        opcode::STATICCALL => {
            // STATICCALL requires six stack inputs
            let inputs = (0..6).map(|i| stack.peek(i)).collect::<Result<Vec<_>, _>>()?;
            Ok(inputs)
        },
        opcode::REVERT => {
            // REVERT requires two stack inputs
            let a = stack.peek(0)?;
            let b = stack.peek(1)?;
            Ok(vec![a, b])
        },
        opcode::INVALID => {
            // INVALID does not require stack inputs
            Ok(vec![])
        },
        opcode::SELFDESTRUCT => {
            // SELFDESTRUCT requires one stack input
            let a = stack.peek(0)?;
            Ok(vec![a])
        },

        // Default case for unrecognized opcodes
        _ => Err(OpcodeError::UnrecognizedOpcode),
    }
}


fn get_slice_from_shared_memory(shared_memory: &SharedMemory, start_index: usize, size: usize) -> Vec<u8> {
    let slice = shared_memory.context_memory();
    let slice_len = slice.len();

    let end_index = start_index + size;

    // Return the slice if start_index is within the slice, else return an empty slice
    if start_index < slice_len && end_index < slice_len {
        slice[start_index..end_index].to_vec()
    } else {
        Vec::new() // return an empty Vec<u8> if out of range
    }
}
