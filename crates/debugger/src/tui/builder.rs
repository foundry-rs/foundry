//! TUI debugger builder.

use crate::{DebugNode, Debugger};
use alloy_primitives::{Address, Bytes};
use foundry_common::{compile::ContractSources, evm::Breakpoints, get_contract_name};
use foundry_evm_traces::{CallTraceArena, CallTraceDecoder, Traces};
use revm_inspectors::tracing::types::TraceMemberOrder;
use std::collections::HashMap;

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    debug_arena: Vec<DebugNode>,
    /// Identified contracts.
    identified_contracts: HashMap<Address, String>,
    /// Map of source files.
    sources: ContractSources,
    /// Map of the debugger breakpoints.
    breakpoints: Breakpoints,
}

impl DebuggerBuilder {
    /// Creates a new debugger builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Extends the debug arena.
    #[inline]
    pub fn traces(mut self, traces: Traces) -> Self {
        for (_, arena) in traces {
            self = self.trace_arena(arena);
        }
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn trace_arena(mut self, arena: CallTraceArena) -> Self {
        flatten_call_trace(arena, &mut self.debug_arena);
        self
    }

    /// Extends the identified contracts from multiple decoders.
    #[inline]
    pub fn decoders(mut self, decoders: &[CallTraceDecoder]) -> Self {
        for decoder in decoders {
            self = self.decoder(decoder);
        }
        self
    }

    /// Extends the identified contracts from a decoder.
    #[inline]
    pub fn decoder(self, decoder: &CallTraceDecoder) -> Self {
        let c = decoder.contracts.iter().map(|(k, v)| (*k, get_contract_name(v).to_string()));
        self.identified_contracts(c)
    }

    /// Extends the identified contracts.
    #[inline]
    pub fn identified_contracts(
        mut self,
        identified_contracts: impl IntoIterator<Item = (Address, String)>,
    ) -> Self {
        self.identified_contracts.extend(identified_contracts);
        self
    }

    /// Sets the sources for the debugger.
    #[inline]
    pub fn sources(mut self, sources: ContractSources) -> Self {
        self.sources = sources;
        self
    }

    /// Sets the breakpoints for the debugger.
    #[inline]
    pub fn breakpoints(mut self, breakpoints: Breakpoints) -> Self {
        self.breakpoints = breakpoints;
        self
    }

    /// Builds the debugger.
    #[inline]
    pub fn build(self) -> Debugger {
        let Self { debug_arena, identified_contracts, sources, breakpoints } = self;
        Debugger::new(debug_arena, identified_contracts, sources, breakpoints)
    }
}

fn flatten_call_trace(arena: CallTraceArena, out: &mut Vec<DebugNode>) {
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
