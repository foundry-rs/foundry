//! Debugger builder.

use crate::{
    Debugger, DebuggerLayout, debugger::DebuggerStats, node::flatten_call_trace_with_precompiles,
};
use alloy_primitives::{
    Address, Bytes,
    map::{AddressHashMap, HashMap},
};
use foundry_common::{ContractsByArtifact, get_contract_name, slot_identifier::SlotIdentifier};
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::{
    CallTraceArena, CallTraceDecoder, CallTraceNode, Traces,
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
    /// Full artifact identifiers for identified contracts.
    contract_identifiers: AddressHashMap<String>,
    /// Known local contracts and their compiler metadata.
    known_contracts: ContractsByArtifact,
    /// Active precompile labels for the current trace context.
    precompile_labels: AddressHashMap<String>,
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
    ///
    /// Internal calls are decoded during [`Self::build`], after resolving each frame's contract.
    #[inline]
    pub fn traces(mut self, traces: Traces) -> Self {
        for (_, arena) in traces {
            self = self.trace_arena(arena.arena);
        }
        self
    }

    /// Extends the debug arena.
    ///
    /// Internal calls are decoded during [`Self::build`], after resolving each frame's contract.
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
    pub fn decoder(mut self, decoder: &CallTraceDecoder) -> Self {
        for (address, identifier) in &decoder.contracts {
            self.identified_contracts.insert(*address, get_contract_name(identifier).to_string());
            self.contract_identifiers.insert(*address, identifier.clone());
        }
        self.precompile_labels.extend(decoder.precompile_labels());
        self
    }

    /// Sets known local contracts used to identify storage slots.
    #[inline]
    pub fn known_contracts(mut self, known_contracts: &ContractsByArtifact) -> Self {
        self.known_contracts = known_contracts.clone();
        self
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
        let Self {
            trace_arenas,
            stats,
            identified_contracts,
            contract_identifiers,
            known_contracts,
            precompile_labels,
            sources,
            breakpoints,
            layout,
        } = self;
        let slot_identifiers = contract_identifiers
            .into_iter()
            .filter_map(|(address, identifier)| {
                let (_, contract) =
                    known_contracts.find_by_name_or_identifier(&identifier).ok().flatten()?;
                let layout = contract.storage_layout.clone()?;
                Some((address, SlotIdentifier::new(layout)))
            })
            .collect();
        let mut identified_code = HashMap::default();
        let mut debug_arena = Vec::new();
        for mut arena in trace_arenas {
            let contract_names = arena
                .nodes_mut()
                .iter_mut()
                .map(|node| {
                    identify_node(
                        node,
                        &known_contracts,
                        &identified_contracts,
                        &sources,
                        &mut identified_code,
                    )
                })
                .collect::<Vec<_>>();
            flatten_call_trace_with_precompiles(
                arena,
                &mut debug_arena,
                &precompile_labels,
                &contract_names,
            );
        }
        Debugger::new_with_stats(
            debug_arena,
            stats,
            identified_contracts,
            slot_identifiers,
            sources,
            breakpoints,
            layout,
        )
    }
}

/// Identifies the contract executed by `node` from its recorded bytecode, since an address can
/// execute different code over time (e.g. after `vm.etch`), and decodes its internal calls.
fn identify_node(
    node: &mut CallTraceNode,
    known_contracts: &ContractsByArtifact,
    identified_contracts: &AddressHashMap<String>,
    sources: &ContractSources,
    identified_code: &mut HashMap<(Address, Bytes), Option<String>>,
) -> Option<String> {
    let address = node.trace.address;
    let address_name = identified_contracts.get(&address);
    let contract_name = match &node.trace.bytecode {
        Some(code) if !code.is_empty() && !node.trace.kind.is_any_create() => identified_code
            .entry((address, code.clone()))
            .or_insert_with(|| identify_code(known_contracts, address_name, code))
            .clone(),
        _ => address_name.cloned(),
    };
    if contract_name.as_ref() != address_name
        && let Some(decoded) = node.trace.decoded.as_mut()
        && decoded.label.as_ref() == address_name
    {
        decoded.label.clone_from(&contract_name);
    }
    if let Some(contract_name) = &contract_name
        && !sources.artifacts_by_name.is_empty()
    {
        DebugTraceIdentifier::identify_node_steps_with_sources(node, sources, contract_name);
    }
    contract_name
}

/// Identifies `code` by an exact match against local artifacts. Identities that aren't local
/// artifacts (e.g. from Etherscan) can't be checked, so they are kept when nothing matches.
fn identify_code(
    known_contracts: &ContractsByArtifact,
    address_name: Option<&String>,
    code: &[u8],
) -> Option<String> {
    // Prefer the address identity only among equally strong runtime matches.
    if let Some((id, _)) = known_contracts
        .find_by_deployed_code_exact_preferred(code, |id| address_name == Some(&id.name))
    {
        return Some(id.name.clone());
    }
    // External identities cannot be checked against local artifacts.
    address_name.filter(|name| !known_contracts.iter().any(|(id, _)| id.name == **name)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_traces::{CallKind, CallTrace, CallTraceStep, TraceMemberOrder};
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};

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
            state_gas_cost: None,
            state_gas_reservoir: None,
            state_gas_spent: 0,
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
        let builder = Debugger::builder().trace_arena(trace_arena(100, 1));

        assert_eq!(builder.stats.session_subcalls, 1);
        assert_eq!(builder.stats.session_trace_gas_used, 100);
        assert_eq!(builder.trace_arenas.len(), 1);
    }

    #[test]
    fn trace_arena_accumulates_session_stats_across_multiple_arenas() {
        let builder =
            Debugger::builder().trace_arena(trace_arena(100, 1)).trace_arena(trace_arena(200, 2));

        assert_eq!(builder.stats.session_subcalls, 3);
        assert_eq!(builder.stats.session_trace_gas_used, 300);
        assert_eq!(builder.trace_arenas.len(), 2);
    }
}
