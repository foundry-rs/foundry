use alloy_primitives::{Bytes, U256};

use revm::interpreter::{InstructionResult, OpCode};

use foundry_evm_core::buffer::{get_buffer_accesses, BufferKind};
use revm_inspectors::tracing::types::{CallTraceStep, RecordedMemory};
use spec::Vm::DebugStep;

// Function to convert CallTraceStep to DebugStep
pub(crate) fn convert_call_trace_to_debug_step(step: &CallTraceStep) -> DebugStep {
    let opcode = step.op.get();
    let stack = get_stack_inputs_for_opcode(opcode, step.stack.as_ref());

    let memory = get_memory_input_for_opcode(opcode, step.stack.as_ref(), step.memory.as_ref());

    let is_out_of_gas = step.status == InstructionResult::OutOfGas ||
        step.status == InstructionResult::MemoryOOG ||
        step.status == InstructionResult::MemoryLimitOOG ||
        step.status == InstructionResult::PrecompileOOG ||
        step.status == InstructionResult::InvalidOperandOOG;

    DebugStep {
        stack,
        memoryInput: memory,
        opcode: step.op.get(),
        depth: step.depth,
        isOutOfGas: is_out_of_gas,
        contractAddr: step.contract,
    }
}

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
fn get_memory_input_for_opcode(
    opcode: u8,
    stack: Option<&Vec<U256>>,
    memory: Option<&RecordedMemory>,
) -> Bytes {
    let Some(stack_data) = stack else { return Bytes::new() };
    let Some(memory_data) = memory else { return Bytes::new() };

    if let Some(accesses) = get_buffer_accesses(opcode, stack_data) {
        if let Some((kind, access)) = accesses.read {
            return match kind {
                BufferKind::Memory => {
                    get_slice_from_memory(memory_data.as_bytes(), access.offset, access.len)
                }
                _ => Bytes::new(),
            }
        }
    };

    Bytes::new()
}

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
fn get_stack_inputs_for_opcode(opcode: u8, stack: Option<&Vec<U256>>) -> Vec<U256> {
    let Some(op) = OpCode::new(opcode) else {
        // unknown opcode
        return vec![]
    };

    let Some(stack_data) = stack else { return vec![] };

    let stack_input_size = op.inputs() as usize;
    let mut inputs = Vec::new();
    for i in 0..stack_input_size {
        inputs.push(stack_data[stack_data.len() - 1 - i]);
    }
    inputs
}

fn get_slice_from_memory(memory: &Bytes, start_index: usize, size: usize) -> Bytes {
    let memory_len = memory.len();

    let end_bound = start_index + size;

    // Return the bytes if data is within the range.
    if start_index < memory_len && end_bound <= memory_len {
        return memory.slice(start_index..end_bound);
    }

    // Pad zero bytes if attempting to load memory partially out of range.
    if start_index < memory_len && end_bound > memory_len {
        let mut result = memory.slice(start_index..memory_len).to_vec();
        result.resize(size, 0u8);
        return Bytes::from(result);
    }

    // Return empty bytes with the size if not in range at all.
    Bytes::from(vec![0u8; size])
}
