use alloy_primitives::{Address, Bytes};
use foundry_evm_traces::{CallKind, CallTraceArena};
use revm_inspectors::tracing::types::{CallTraceStep, TraceMemberOrder};
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
    /// The debug steps.
    pub steps: Vec<CallTraceStep>,
}

impl DebugNode {
    /// Creates a new debug node.
    pub fn new(
        address: Address,
        kind: CallKind,
        steps: Vec<CallTraceStep>,
        calldata: Bytes,
    ) -> Self {
        Self { address, kind, steps, calldata }
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
    }

    fn inner(arena: &CallTraceArena, node_idx: usize, out: &mut Vec<PendingNode>) {
        let mut pending = PendingNode { node_idx, steps_count: 0 };
        let node = &arena.nodes()[node_idx];
        for order in node.ordering.iter() {
            match order {
                TraceMemberOrder::Call(idx) => {
                    out.push(pending);
                    pending.steps_count = 0;
                    inner(arena, node.children[*idx], out);
                }
                TraceMemberOrder::Step(_) => {
                    pending.steps_count += 1;
                }
                _ => {}
            }
        }
        out.push(pending);
    }
    let mut nodes = Vec::new();
    inner(&arena, 0, &mut nodes);

    let mut arena_nodes = arena.into_nodes();

    for pending in nodes {
        let steps = {
            let other_steps =
                arena_nodes[pending.node_idx].trace.steps.split_off(pending.steps_count);
            std::mem::replace(&mut arena_nodes[pending.node_idx].trace.steps, other_steps)
        };

        // Skip nodes with empty steps as there's nothing to display for them.
        if steps.is_empty() {
            continue
        }

        let call = &arena_nodes[pending.node_idx].trace;
        let calldata = if call.kind.is_any_create() { Bytes::new() } else { call.data.clone() };
        let node = DebugNode::new(call.address, call.kind, steps, calldata);

        out.push(node);
    }
}
