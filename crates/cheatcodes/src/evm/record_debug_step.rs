use alloy_primitives::U256;

use foundry_evm_traces::CallTraceArena;
use revm::interpreter::{InstructionResult, OpCode};

use foundry_evm_core::buffer::{get_buffer_accesses, BufferKind};
use revm_inspectors::tracing::types::{CallTraceStep, RecordedMemory};
use spec::Vm::DebugStep;

// A depth first traverse to flatten the recorded steps.
pub(crate) fn flatten_call_trace(
    root: usize,
    arena: &CallTraceArena,
    node_start_idx: usize,
) -> Vec<&CallTraceStep> {
    let mut out = Vec::new();
    let mut nodes = Vec::new(); // Use a Vec as a stack
    nodes.push(root);

    while let Some(node_idx) = nodes.pop() {
        // Pop from the end of the stack
        let node = &arena.nodes()[node_idx];
        if node_idx >= node_start_idx {
            for step in &node.trace.steps {
                out.push(step);
            }
        }
        // Push children onto the stack in reverse order so that the first child is
        // processed first
        for &child_idx in node.children.iter().rev() {
            nodes.push(child_idx);
        }
    }

    out
}

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
        memoryData: memory,
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
) -> Vec<u8> {
    let Some(stack_data) = stack else { return vec![] };
    let Some(memory_data) = memory else { return vec![] };

    if let Some(accesses) = get_buffer_accesses(opcode, stack_data) {
        if let Some((kind, access)) = accesses.read {
            return match kind {
                BufferKind::Memory => {
                    get_slice_from_memory(memory_data.as_bytes(), access.offset, access.len)
                }
                _ => vec![],
            }
        }
    };

    vec![]
}

// The expected `stack` here is from the trace stack, where the top of the stack
// is the last value of the vector
fn get_stack_inputs_for_opcode(opcode: u8, stack: Option<&Vec<U256>>) -> Vec<U256> {
    let Some(op) = OpCode::new(opcode) else {
        // unknown opcode
        return vec![]
    };

    let Some(stack_data) = stack else { return vec![] };

    let stack_input_size = op.inputs();
    let mut inputs = Vec::new();
    for i in 0..stack_input_size {
        inputs.push(peak_stack(stack_data, i.into()));
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
