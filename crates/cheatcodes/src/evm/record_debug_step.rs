use alloy_primitives::U256;

use alloy_rpc_types::serde_helpers::quantity::vec;
use revm::interpreter::OpCode;

use foundry_debugger::tui::draw::get_buffer_accesses;
use foundry_debugger::tui::context::BufferKind;

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
pub(crate) fn get_memory_input_for_opcode(
    opcode: u8,
    stack: Vec<U256>,
    memory: &[u8],
) -> Vec<u8> {
    if let Some(accesses) = get_buffer_accesses(opcode, &stack) {
        if let Some((kind, access)) = accesses.read {
            return match kind {
                BufferKind::Memory => get_slice_from_memory(
                    memory,
                    access.offset,
                    access.len
                ),
                _ => vec![]
            }
        }
    };

    vec![]
}

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
pub(crate) fn get_stack_inputs_for_opcode(opcode: u8, stack: Vec<U256>) -> Vec<U256> {

    let Some(op) = OpCode::new(opcode) else {
        // unknown opcode
        return vec![]
    };

    let stack_input_size = op.inputs();
    let mut inputs = Vec::new();
    for i in 0..stack_input_size {
        inputs.push(peak_stack(&stack, i.into()));
    }
    inputs

}

fn peak_stack(stack: &[U256], i: usize) -> U256 {
    stack[stack.len() - 1 - i]
}

fn get_slice_from_memory(memory: &[u8], start_index: usize, size: usize) -> Vec<u8> {
    let memory_len = memory.len();

    let end_index = start_index + size;

    // Return the slice if start_index is within the slice, else return an empty slice
    if start_index < memory_len && end_index < memory_len {
        memory[start_index..end_index].to_vec()
    } else {
        Vec::new() // return an empty Vec<u8> if out of range
    }
}
