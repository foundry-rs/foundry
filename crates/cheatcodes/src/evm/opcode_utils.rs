/// Utility functions for opcode related stuff

use revm::interpreter::{opcode};
use alloy_primitives::U256;


// The expected `stack_input` here is already handled by the `get_stack_inputs_for_opcode()` function.
pub(crate) fn get_memory_input_for_opcode(
    opcode: u8,
    stack_inputs: Vec<U256>,
    memory: &[u8]
) -> Vec<u8> {
    match opcode {
        0x20 => { // KECCAK256 (SHA3)
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0x51 => { // MLOAD
            let offset = stack_inputs[0].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, 32)
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
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xA1 => { // LOG1
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xA2 => { // LOG2
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xA3 => { // LOG3
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xA4 => { // LOG4
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xF0 => { // CREATE
            let offset = stack_inputs[1].to::<u64>() as usize;
            let len = stack_inputs[2].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xF1 => { // CALL
            let args_offset = stack_inputs[3].to::<u64>() as usize;
            let args_len = stack_inputs[4].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, args_offset, args_len)
        },
        0xF2 => { // CALLCODE
            let args_offset = stack_inputs[3].to::<u64>() as usize;
            let args_len = stack_inputs[4].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, args_offset, args_len)
        },
        0xF3 => { // RETURN
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xF4 => { // DELEGATECALL
            let args_offset = stack_inputs[2].to::<u64>() as usize;
            let args_len = stack_inputs[3].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, args_offset, args_len)
        },
        0xF5 => { // CREATE2
            let offset = stack_inputs[1].to::<u64>() as usize;
            let len = stack_inputs[2].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },
        0xFA => { // STATICCALL
            let args_offset = stack_inputs[2].to::<u64>() as usize;
            let args_len = stack_inputs[3].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, args_offset, args_len)
        },
        0xFD => { // REVERT
            let offset = stack_inputs[0].to::<u64>() as usize;
            let len = stack_inputs[1].to::<u64>() as usize;
            get_slice_from_shared_memory(memory, offset, len)
        },

        // Other opcodes...
        _ => vec![], // For unrecognized opcodes
    }
}


// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
pub(crate) fn get_stack_inputs_for_opcode(opcode: u8, stack: Vec<U256>) -> Vec<U256> {
    match opcode {
        opcode::STOP => vec![],
        opcode::ADD | opcode::MUL | opcode::SUB | opcode::DIV |
        opcode::SDIV | opcode::MOD | opcode::SMOD => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::ADDMOD | opcode::MULMOD => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::EXP => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SIGNEXTEND => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::LT => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::GT => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SLT => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SGT => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::EQ => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::ISZERO => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::AND => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::OR => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::XOR => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::NOT => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::BYTE => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SHL => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SHR => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SAR => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::KECCAK256 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::ADDRESS => vec![],
        opcode::BALANCE => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::ORIGIN => vec![],
        opcode::CALLER => vec![],
        opcode::CALLVALUE => vec![],
        opcode::CALLDATALOAD => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::CALLDATASIZE => vec![],
        opcode::CALLDATACOPY => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::CODESIZE => vec![],
        opcode::CODECOPY => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::GASPRICE => vec![],
        opcode::EXTCODESIZE => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::EXTCODECOPY => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            let d = peak_stack(&stack, 3);
            vec![a, b, c, d]
        },
        opcode::RETURNDATASIZE => vec![],
        opcode::RETURNDATACOPY => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::EXTCODEHASH => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::BLOCKHASH => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::COINBASE => vec![],
        opcode::TIMESTAMP => vec![],
        opcode::NUMBER => vec![],
        opcode::DIFFICULTY => vec![],
        opcode::GASLIMIT => vec![],
        opcode::CHAINID => vec![],
        opcode::SELFBALANCE => vec![],
        opcode::BASEFEE => vec![],
        opcode::POP => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::MLOAD => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::MSTORE => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::MSTORE8 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::SLOAD => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::SSTORE => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::JUMP => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::JUMPI => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::PC => vec![],
        opcode::MSIZE => vec![],
        opcode::GAS => vec![],
        opcode::JUMPDEST => vec![],
        opcode::TLOAD => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        opcode::TSTORE => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::MCOPY => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::DUP1 | opcode::DUP2 | opcode::DUP3 | opcode::DUP4 | opcode::DUP5 |
        opcode::DUP6 | opcode::DUP7 | opcode::DUP8 | opcode::DUP9 | opcode::DUP10 => {
            let mut inputs = Vec::new();
            for i in 0..(opcode as usize - opcode::DUP1 as usize + 1) {
                inputs.push(peak_stack(&stack, i));
            }
            inputs
        },
        opcode::SWAP1 | opcode::SWAP2 | opcode::SWAP3 | opcode::SWAP4 | opcode::SWAP5 |
        opcode::SWAP6 | opcode::SWAP7 | opcode::SWAP8 | opcode::SWAP9 | opcode::SWAP10 => {
            let mut inputs = Vec::new();
            for i in 0..(opcode as usize - opcode::SWAP1 as usize + 2) {
                inputs.push(peak_stack(&stack, i));
            }
            inputs
        },
        opcode::LOG0 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::LOG1 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::LOG2 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            let d = peak_stack(&stack, 3);
            vec![a, b, c, d]
        },
        opcode::LOG3 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            let d = peak_stack(&stack, 3);
            let e = peak_stack(&stack, 4);
            vec![a, b, c, d, e]
        },
        opcode::LOG4 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            let d = peak_stack(&stack, 3);
            let e = peak_stack(&stack, 4);
            let f = peak_stack(&stack, 5);
            vec![a, b, c, d, e, f]
        },
        opcode::CREATE => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            vec![a, b, c]
        },
        opcode::CALL => {
            let inputs = (0..7).map(|i| peak_stack(&stack, i)).collect::<Vec<U256>>();
            inputs
        },
        opcode::CALLCODE => {
            let inputs = (0..7).map(|i| peak_stack(&stack, i)).collect::<Vec<U256>>();
            inputs
        },
        opcode::RETURN => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::DELEGATECALL => {
            let inputs = (0..6).map(|i| peak_stack(&stack, i)).collect::<Vec<U256>>();
            inputs
        },
        opcode::CREATE2 => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            let c = peak_stack(&stack, 2);
            let d = peak_stack(&stack, 3);
            vec![a, b, c, d]
        },
        opcode::STATICCALL => {
            let inputs = (0..6).map(|i| peak_stack(&stack, i)).collect::<Vec<U256>>();
            inputs
        },
        opcode::REVERT => {
            let a = peak_stack(&stack, 0);
            let b = peak_stack(&stack, 1);
            vec![a, b]
        },
        opcode::INVALID => vec![],
        opcode::SELFDESTRUCT => {
            let a = peak_stack(&stack, 0);
            vec![a]
        },
        _ => vec![],
    }
}


fn peak_stack(stack: &Vec<U256>, i: usize) -> U256 {
    return stack[stack.len() - 1 - i];
}

fn get_slice_from_shared_memory(memory: &[u8], start_index: usize, size: usize) -> Vec<u8> {
    let memory_len = memory.len();

    let end_index = start_index + size;

    // Return the slice if start_index is within the slice, else return an empty slice
    if start_index < memory_len && end_index < memory_len {
        memory[start_index..end_index].to_vec()
    } else {
        Vec::new() // return an empty Vec<u8> if out of range
    }
}
