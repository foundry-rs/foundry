use alloy_primitives::{Address, Bytes};
use foundry_evm_traces::{CallKind, CallTraceArena};
use revm_inspectors::tracing::types::{CallTraceStep, DecodedCallTrace, TraceMemberOrder};
use serde::{Deserialize, Serialize};

/// Represents a part of the execution frame before the next call or end of the execution.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DebugNode {
    /// Execution context.
    ///
    /// Note that this is the address of the *code*, not necessarily the address of the storage.
    pub address: Address,
    /// The kind of call this is.
    pub kind: CallKind,
    /// Calldata of the call.
    pub calldata: Bytes,
    /// The gas limit of the call.
    pub gas_limit: u64,
    /// Stable id for the original call trace node within the flattened debugger arena.
    #[serde(default)]
    pub trace_node_idx: usize,
    /// Index of the first step in the original call trace node.
    #[serde(default)]
    pub step_offset: usize,
    /// Decoded call data for the current execution context, if available.
    #[serde(default)]
    pub decoded: Option<Box<DecodedCallTrace>>,
    /// The debug steps.
    pub steps: Vec<CallTraceStep>,
}

impl DebugNode {
    /// Creates a new debug node.
    pub const fn new(
        address: Address,
        kind: CallKind,
        steps: Vec<CallTraceStep>,
        calldata: Bytes,
        gas_limit: u64,
        decoded: Option<Box<DecodedCallTrace>>,
    ) -> Self {
        Self {
            address,
            kind,
            steps,
            calldata,
            gas_limit,
            trace_node_idx: 0,
            step_offset: 0,
            decoded,
        }
    }
}

/// Flattens given [CallTraceArena] into a list of [DebugNode]s.
///
/// This is done by recursively traversing the call tree and collecting the steps in-between the
/// calls.
pub fn flatten_call_trace(arena: CallTraceArena, out: &mut Vec<DebugNode>) {
    #[derive(Debug, Clone, Copy)]
    struct PendingNode {
        node_idx: usize,
        steps_count: usize,
        step_offset: usize,
    }

    fn inner(arena: &CallTraceArena, node_idx: usize, out: &mut Vec<PendingNode>) {
        let mut pending = PendingNode { node_idx, steps_count: 0, step_offset: 0 };
        let mut next_step_offset = 0;
        let node = &arena.nodes()[node_idx];
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Call(idx) => {
                    out.push(pending);
                    pending =
                        PendingNode { node_idx, steps_count: 0, step_offset: next_step_offset };
                    inner(arena, node.children[*idx], out);
                }
                TraceMemberOrder::Step(step_idx) => {
                    if pending.steps_count == 0 {
                        pending.step_offset = *step_idx;
                    }
                    pending.steps_count += 1;
                    next_step_offset = step_idx.saturating_add(1);
                }
                _ => {}
            }
        }
        out.push(pending);
    }
    let mut nodes = Vec::new();
    inner(&arena, 0, &mut nodes);

    let mut arena_nodes = arena.into_nodes();
    let trace_node_idx_offset =
        out.iter().map(|node| node.trace_node_idx).max().map_or(0, |idx| idx.saturating_add(1));

    for pending in nodes {
        let steps = {
            let other_steps =
                arena_nodes[pending.node_idx].trace.steps.split_off(pending.steps_count);
            std::mem::replace(&mut arena_nodes[pending.node_idx].trace.steps, other_steps)
        };

        // Skip nodes with empty steps as there's nothing to display for them.
        if steps.is_empty() {
            continue;
        }

        let call = &arena_nodes[pending.node_idx].trace;
        let calldata = if call.kind.is_any_create() { Bytes::new() } else { call.data.clone() };
        let mut node = DebugNode::new(
            call.address,
            call.kind,
            steps,
            calldata,
            call.gas_limit,
            call.decoded.clone(),
        );
        node.trace_node_idx = trace_node_idx_offset.saturating_add(pending.node_idx);
        node.step_offset = pending.step_offset;

        out.push(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_traces::{CallTrace, CallTraceNode};
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};

    fn step(pc: usize) -> CallTraceStep {
        CallTraceStep {
            pc,
            op: OpCode::STOP,
            stack: None,
            push_stack: None,
            memory: None,
            returndata: Bytes::new(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost: 0,
            storage_change: None,
            status: Some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
    }

    #[test]
    fn flatten_records_original_step_offsets_for_split_segments() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step(0), step(1), step(2)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
                TraceMemberOrder::Step(2),
            ];
            root.children.push(1);
        }

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace { kind: CallKind::Call, steps: vec![step(10)], ..Default::default() },
            ordering: vec![TraceMemberOrder::Step(0)],
            ..Default::default()
        });

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_eq!(flattened.len(), 3);
        assert_eq!((flattened[0].trace_node_idx, flattened[0].step_offset), (0, 0));
        assert_eq!(flattened[0].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [0]);
        assert_eq!((flattened[1].trace_node_idx, flattened[1].step_offset), (1, 0));
        assert_eq!(flattened[1].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [10]);
        assert_eq!((flattened[2].trace_node_idx, flattened[2].step_offset), (0, 1));
        assert_eq!(flattened[2].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn flatten_keeps_trace_node_ids_unique_when_appending_multiple_arenas() {
        let mut flattened = vec![DebugNode { trace_node_idx: 1, ..Default::default() }];

        let mut arena = CallTraceArena::default();
        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step(0)];
            root.ordering = vec![TraceMemberOrder::Step(0)];
        }

        flatten_call_trace(arena, &mut flattened);

        assert_eq!(flattened[1].trace_node_idx, 2);
    }
}
