//! Debugger builder.

use crate::{DebugNode, Debugger, debugger::DebuggerStats, node::flatten_call_trace};
use alloy_primitives::{Address, map::AddressHashMap};
use foundry_common::get_contract_name;
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::{CallTraceArena, CallTraceDecoder, Traces, debug::ContractSources};

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    debug_arena: Vec<DebugNode>,
    /// Aggregate stats for the traces passed to the debugger.
    stats: DebuggerStats,
    /// Identified contracts.
    identified_contracts: AddressHashMap<String>,
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
            self = self.trace_arena(arena.arena);
        }
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn trace_arena(mut self, arena: CallTraceArena) -> Self {
        if let Some(root) = arena.nodes().first() {
            self.stats.total_gas_used =
                self.stats.total_gas_used.saturating_add(root.trace.gas_used);
        }
        self.stats.subcalls =
            self.stats.subcalls.saturating_add(arena.nodes().len().saturating_sub(1));
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
        let Self { debug_arena, stats, identified_contracts, sources, breakpoints } = self;
        Debugger::new_with_stats(debug_arena, stats, identified_contracts, sources, breakpoints)
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

    #[test]
    fn trace_arena_counts_empty_step_subcalls() {
        let mut arena = CallTraceArena::default();
        let root = &mut arena.nodes_mut()[0];
        root.trace.steps.push(step());
        root.trace.gas_limit = 1;
        root.trace.gas_used = 100;
        root.ordering.push(TraceMemberOrder::Step(0));
        root.ordering.push(TraceMemberOrder::Call(0));
        root.children.push(1);

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace { depth: 1, kind: CallKind::Call, ..Default::default() },
            ..Default::default()
        });

        let builder = DebuggerBuilder::new().trace_arena(arena);

        assert_eq!(builder.stats.subcalls, 1);
        assert_eq!(builder.stats.total_gas_used, 100);
        assert_eq!(builder.debug_arena.len(), 1);
    }
}
