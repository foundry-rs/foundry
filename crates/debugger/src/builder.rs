//! Debugger builder.

use crate::{Debugger, DebuggerLayout, debugger::DebuggerStats, node::flatten_call_trace};
use alloy_primitives::{Address, map::AddressHashMap};
use foundry_common::get_contract_name;
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::{
    CallTraceArena, CallTraceDecoder, Traces,
    debug::{ContractSources, DebugTraceIdentifier},
};

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    trace_arenas: Vec<CallTraceArena>,
    /// Aggregate stats for the traces passed to the debugger.
    stats: DebuggerStats,
    /// Identified contracts.
    identified_contracts: AddressHashMap<String>,
    /// Map of source files.
    sources: ContractSources,
    /// Map of the debugger breakpoints.
    breakpoints: Breakpoints,
    /// TUI layout selection.
    layout: DebuggerLayout,
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
            self = self.trace_arena(arena.arena);
        }
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn trace_arena(mut self, arena: CallTraceArena) -> Self {
        if let Some(root) = arena.nodes().first() {
            self.stats.session_trace_gas_used =
                self.stats.session_trace_gas_used.saturating_add(root.trace.gas_used);
        }
        self.stats.session_subcalls =
            self.stats.session_subcalls.saturating_add(arena.nodes().len().saturating_sub(1));
        self.trace_arenas.push(arena);
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

    /// Sets the TUI layout for the debugger.
    #[inline]
    pub const fn layout(mut self, layout: DebuggerLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Builds the debugger.
    #[inline]
    pub fn build(self) -> Debugger {
        let Self { mut trace_arenas, stats, identified_contracts, sources, breakpoints, layout } =
            self;
        identify_internal_calls(&mut trace_arenas, &identified_contracts, &sources);
        let mut debug_arena = Vec::new();
        for arena in trace_arenas {
            flatten_call_trace(arena, &mut debug_arena);
        }
        Debugger::new_with_stats(
            debug_arena,
            stats,
            identified_contracts,
            sources,
            breakpoints,
            layout,
        )
    }
}

fn identify_internal_calls(
    trace_arenas: &mut [CallTraceArena],
    identified_contracts: &AddressHashMap<String>,
    sources: &ContractSources,
) {
    if sources.artifacts_by_name.is_empty() {
        return;
    }

    for arena in trace_arenas {
        for node in arena.nodes_mut() {
            let Some(contract_name) = identified_contracts.get(&node.trace.address) else {
                continue;
            };
            DebugTraceIdentifier::identify_node_steps_with_sources(node, sources, contract_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use foundry_evm_traces::{CallKind, CallTrace, CallTraceNode};
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};
    use revm_inspectors::tracing::types::{CallTraceStep, TraceMemberOrder};

    fn step() -> CallTraceStep {
        CallTraceStep {
            pc: 0,
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

    fn trace_arena(gas_used: u64, subcalls: usize) -> CallTraceArena {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps.push(step());
            root.trace.gas_limit = 1;
            root.trace.gas_used = gas_used;
            root.ordering.push(TraceMemberOrder::Step(0));

            for idx in 1..=subcalls {
                root.ordering.push(TraceMemberOrder::Call(idx - 1));
                root.children.push(idx);
            }
        }

        for idx in 1..=subcalls {
            arena.nodes_mut().push(CallTraceNode {
                parent: Some(0),
                idx,
                trace: CallTrace { depth: 1, kind: CallKind::Call, ..Default::default() },
                ..Default::default()
            });
        }

        arena
    }

    #[test]
    fn trace_arena_accumulates_stats() {
        let builder = DebuggerBuilder::new().trace_arena(trace_arena(100, 1));

        assert_eq!(builder.stats.session_subcalls, 1);
        assert_eq!(builder.stats.session_trace_gas_used, 100);
        assert_eq!(builder.trace_arenas.len(), 1);
    }

    #[test]
    fn trace_arena_accumulates_session_stats_across_multiple_arenas() {
        let builder = DebuggerBuilder::new()
            .trace_arena(trace_arena(100, 1))
            .trace_arena(trace_arena(200, 2));

        assert_eq!(builder.stats.session_subcalls, 3);
        assert_eq!(builder.stats.session_trace_gas_used, 300);
        assert_eq!(builder.trace_arenas.len(), 2);
    }
}
