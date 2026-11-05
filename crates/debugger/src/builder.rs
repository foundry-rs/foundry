//! Debugger builder.

use crate::soldb::{ContractIdentity, SoldbDebugger, write_bundle};
use alloy_primitives::{
    Address, Bytes,
    map::{AddressHashMap, HashMap},
};
use eyre::Result;
use foundry_common::{ContractsByArtifact, get_contract_name};
use foundry_evm_core::Breakpoints;
use foundry_evm_traces::{
    CallTrace, CallTraceArena, CallTraceDecoder, CallTraceNode, Traces,
    debug::{ContractSources, DebugRecording},
};
use std::path::Path;

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `run` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    trace_arenas: Vec<CallTraceArena>,
    /// The steps a debug recorder recorded for each trace arena, if one ran.
    debug_steps: Vec<Option<DebugRecording>>,
    /// Identified contracts.
    identified_contracts: AddressHashMap<String>,
    /// Known local contracts and their compiler metadata.
    known_contracts: ContractsByArtifact,
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
            *self.debug_steps.last_mut().unwrap() = arena.debug_steps;
        }
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn trace_arena(mut self, arena: CallTraceArena) -> Self {
        self.trace_arenas.push(arena);
        self.debug_steps.push(None);
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
        }
        self
    }

    /// Sets known local contracts and their compiler metadata.
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

    /// Opens SolDB, or writes a bundle for its CLI, profiler, and DAP server.
    ///
    /// Every trace arena belongs to the same session, including deployment and setup.
    pub fn run(self, dump: Option<&Path>) -> Result<()> {
        let Self {
            trace_arenas,
            debug_steps,
            identified_contracts,
            known_contracts,
            sources,
            breakpoints,
        } = self;
        // The steps come from the recorder when it ran for every arena, else from the tracer.
        let recordings = if debug_steps.iter().all(Option::is_some) {
            debug_steps.into_iter().flatten().collect()
        } else {
            eyre::ensure!(
                debug_steps.iter().all(Option::is_none),
                "only some debug traces have recorded steps"
            );
            Vec::new()
        };
        let mut identified_code = HashMap::default();
        let contract_identities = trace_arenas
            .iter()
            .map(|arena| {
                arena
                    .nodes()
                    .iter()
                    .map(|node| {
                        node_contract_identity(
                            node,
                            &known_contracts,
                            &identified_contracts,
                            &mut identified_code,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        match dump {
            Some(dir) => write_bundle(
                dir,
                trace_arenas,
                recordings,
                &contract_identities,
                &sources,
                &known_contracts,
            ),
            None => SoldbDebugger::new(
                trace_arenas,
                recordings,
                &contract_identities,
                &sources,
                &known_contracts,
                &breakpoints,
            )?
            .try_run(),
        }
    }
}

/// Identifies the contract executed by `node` from its recorded bytecode, since an address can
/// execute different code over time (e.g. after `vm.etch`).
fn node_contract_identity<'a>(
    node: &CallTraceNode,
    known_contracts: &'a ContractsByArtifact,
    identified_contracts: &AddressHashMap<String>,
    identified_code: &mut HashMap<(Address, Bytes), Option<ContractIdentity<'a>>>,
) -> Option<ContractIdentity<'a>> {
    let trace = &node.trace;
    let address_name = identified_contracts.get(&trace.address);
    if trace.kind.is_any_create() {
        return identify_creation(known_contracts, address_name, trace);
    }
    match &trace.bytecode {
        Some(code) if !code.is_empty() => identified_code
            .entry((trace.address, code.clone()))
            .or_insert_with(|| identify_code(known_contracts, address_name, code))
            .clone(),
        _ => None,
    }
}

/// Identifies `code` by an exact match against local artifacts. Identities that aren't local
/// artifacts (e.g. from Etherscan) can't be checked, so they are kept when nothing matches.
fn identify_code<'a>(
    known_contracts: &'a ContractsByArtifact,
    address_name: Option<&String>,
    code: &[u8],
) -> Option<ContractIdentity<'a>> {
    // Prefer the address identity only among equally strong runtime matches.
    if let Some((id, _)) = known_contracts
        .find_by_deployed_code_exact_preferred(code, |id| address_name == Some(&id.name))
    {
        return Some(ContractIdentity::Local(id));
    }
    // External identities cannot be checked against local artifacts.
    address_name
        .filter(|name| !known_contracts.iter().any(|(id, _)| id.name == **name))
        .cloned()
        .map(ContractIdentity::External)
}

/// Identifies a creation by its init code, which it has even when the constructor reverts.
fn identify_creation<'a>(
    known_contracts: &'a ContractsByArtifact,
    address_name: Option<&String>,
    trace: &CallTrace,
) -> Option<ContractIdentity<'a>> {
    if let Some((id, _)) = known_contracts.find_by_creation_code_prefix(&trace.data) {
        return Some(ContractIdentity::Local(id));
    }
    // The address of a failed creation holds no code, so its identity can't be trusted.
    if !trace.success {
        return None;
    }
    // The init code check above fails for unlinked artifacts or a different metadata hash.
    // The deployed code check ignores both.
    if let Some((id, _)) = known_contracts
        .find_by_deployed_code_exact_preferred(&trace.output, |id| address_name == Some(&id.name))
    {
        return Some(ContractIdentity::Local(id));
    }
    identify_name(known_contracts, address_name)
}

/// Identifies a contract by name alone, unless two local artifacts share the name.
fn identify_name<'a>(
    known_contracts: &'a ContractsByArtifact,
    address_name: Option<&String>,
) -> Option<ContractIdentity<'a>> {
    let name = address_name?;
    let mut local = known_contracts.iter().filter(|(id, _)| id.name == *name);
    match (local.next(), local.next()) {
        (None, _) => Some(ContractIdentity::External(name.clone())),
        (Some((id, _)), None) => Some(ContractIdentity::Local(id)),
        _ => None,
    }
}
