//! Debugger builder.

use crate::soldb::{ContractIdentity, SoldbDebugger, write_bundle};
use alloy_primitives::{
    Address, Bytes,
    map::{AddressHashMap, HashMap},
};
use eyre::Result;
use foundry_common::{ContractsByArtifact, get_contract_name};
use foundry_compilers::ArtifactId;
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
        let mut created = AddressHashMap::default();
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
                            &mut created,
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
/// execute different code over time (e.g. after `vm.etch`). `created` holds the artifact that
/// created each address, in the order the nodes run.
fn node_contract_identity<'a>(
    node: &CallTraceNode,
    known_contracts: &'a ContractsByArtifact,
    identified_contracts: &AddressHashMap<String>,
    identified_code: &mut HashMap<(Address, Bytes), Option<ContractIdentity<'a>>>,
    created: &mut AddressHashMap<&'a ArtifactId>,
) -> Option<ContractIdentity<'a>> {
    let trace = &node.trace;
    let address_name = identified_contracts.get(&trace.address);
    if trace.kind.is_any_create() {
        let identity = identify_creation(known_contracts, address_name, trace);
        if trace.success
            && let Some(ContractIdentity::Local(id)) = identity
        {
            created.insert(trace.address, id);
        }
        return identity;
    }
    match &trace.bytecode {
        Some(code) if !code.is_empty() => identified_code
            .entry((trace.address, code.clone()))
            .or_insert_with(|| {
                identify_code(
                    known_contracts,
                    address_name,
                    created.get(&trace.address).copied(),
                    code,
                )
            })
            .clone(),
        _ => None,
    }
}

/// Identifies `code` by an exact match against local artifacts. Identities that aren't local
/// artifacts (e.g. from Etherscan) can't be checked, so they are kept when nothing matches.
fn identify_code<'a>(
    known_contracts: &'a ContractsByArtifact,
    address_name: Option<&String>,
    creator: Option<&ArtifactId>,
    code: &[u8],
) -> Option<ContractIdentity<'a>> {
    // Artifacts can share a runtime, e.g. with `bytecode_hash = "none"` and different
    // constructors, so ties go to the artifact that created the address, else to its name.
    let preferred = |id: &ArtifactId| match creator {
        Some(creator) => id == creator,
        None => address_name == Some(&id.name),
    };
    // If several artifacts match except for metadata, we can't tell which ran, so none is used.
    if let Ok(Some((id, _))) =
        known_contracts.find_by_deployed_code_exact_unambiguous_preferred(code, preferred)
    {
        return Some(ContractIdentity::Local(id));
    }
    identify_external(known_contracts, address_name)
}

/// Identifies a creation by its init code, which exists even if the constructor reverts. A
/// constructor can return another contract's runtime, so the returned code isn't used.
fn identify_creation<'a>(
    known_contracts: &'a ContractsByArtifact,
    address_name: Option<&String>,
    trace: &CallTrace,
) -> Option<ContractIdentity<'a>> {
    if let Some((id, _)) = known_contracts.find_by_creation_code_exact(&trace.data) {
        return Some(ContractIdentity::Local(id));
    }
    // The address of a failed creation holds no code, so its identity can't be trusted.
    if !trace.success {
        return None;
    }
    identify_external(known_contracts, address_name)
}

/// Returns the address name, e.g. from Etherscan.
/// Returns nothing if a local artifact has that name.
fn identify_external<'a>(
    known_contracts: &ContractsByArtifact,
    address_name: Option<&String>,
) -> Option<ContractIdentity<'a>> {
    address_name
        .filter(|name| !known_contracts.iter().any(|(id, _)| id.name == **name))
        .cloned()
        .map(ContractIdentity::External)
}
