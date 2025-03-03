use alloy_primitives::{Bytes, U256};

use foundry_evm_traces::CallTraceArena;
use revm::interpreter::{InstructionResult, OpCode};

use foundry_evm_core::buffer::{get_buffer_accesses, BufferKind};
use revm_inspectors::tracing::types::{CallTraceStep, RecordedMemory, TraceMemberOrder};
use spec::Vm::DebugStep;

// Do a depth first traverse of the nodes and steps and return steps
// that are after `node_start_idx`
pub(crate) fn flatten_call_trace(
    root: usize,
    arena: &CallTraceArena,
    node_start_idx: usize,
) -> Vec<&CallTraceStep> {
    let mut steps = Vec::new();
    let mut record_started = false;

    // Start the recursion from the root node
    recursive_flatten_call_trace(root, arena, node_start_idx, &mut record_started, &mut steps);
    steps
}

// Inner recursive function to process nodes.
// This implementation directly mutates `record_started` and `flatten_steps`.
// So the recursive call can change the `record_started` flag even for the parent
// unfinished processing, and append steps to the `flatten_steps` as the final result.
fn recursive_flatten_call_trace<'a>(
    node_idx: usize,
    arena: &'a CallTraceArena,
    node_start_idx: usize,
    record_started: &mut bool,
    flatten_steps: &mut Vec<&'a CallTraceStep>,
) {
    // Once node_idx exceeds node_start_idx, start recording steps
    // for all the recursive processing.
    if !*record_started && node_idx >= node_start_idx {
        *record_started = true;
    }

    let node = &arena.nodes()[node_idx];

    for order in node.ordering.iter() {
        match order {
            TraceMemberOrder::Step(step_idx) => {
                if *record_started {
                    let step = &node.trace.steps[*step_idx];
                    flatten_steps.push(step);
                }
            }
            TraceMemberOrder::Call(call_idx) => {
                let child_node_idx = node.children[*call_idx];
                recursive_flatten_call_trace(
                    child_node_idx,
                    arena,
                    node_start_idx,
                    record_started,
                    flatten_steps,
                );
            }
            _ => {}
        }
    }
}

// Function to convert CallTraceStep to DebugStep
pub(crate) fn convert_call_trace_to_debug_step(step: &CallTraceStep) -> DebugStep {
    let opcode = step.op.get();
    let stack = get_stack_inputs_for_opcode(opcode, step.stack.as_ref());

    let memory = get_memory_input_for_opcode(opcode, step.stack.as_ref(), step.memory.as_ref());

    let is_out_of_gas = step.status == InstructionResult::OutOfGas
        || step.status == InstructionResult::MemoryOOG
        || step.status == InstructionResult::MemoryLimitOOG
        || step.status == InstructionResult::PrecompileOOG
        || step.status == InstructionResult::InvalidOperandOOG;

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
    let mut memory_input = Bytes::new();
    let Some(stack_data) = stack else { return memory_input };
    let Some(memory_data) = memory else { return memory_input };

    if let Some(accesses) = get_buffer_accesses(opcode, stack_data) {
        if let Some((BufferKind::Memory, access)) = accesses.read {
            memory_input = get_slice_from_memory(memory_data.as_bytes(), access.offset, access.len);
        }
    };

    memory_input
}

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
fn get_stack_inputs_for_opcode(opcode: u8, stack: Option<&Vec<U256>>) -> Vec<U256> {
    let mut inputs = Vec::new();

    let Some(op) = OpCode::new(opcode) else { return inputs };
    let Some(stack_data) = stack else { return inputs };

    let stack_input_size = op.inputs() as usize;
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
