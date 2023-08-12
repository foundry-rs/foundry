use crate::{Breakpoints, ContractBytecodeSome, DrawMemory, Interrupt, TUIExitReason, Tui};
use cast::{
    decode,
    executor::inspector::{
        cheatcodes::{util::BroadcastableTransactions, BroadcastableTransaction},
        DEFAULT_CREATE2_DEPLOYER,
    },
    trace::CallTraceDecoder,
};
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, LeaveAlternateScreen},
};
use ethers::{
    signers::LocalWallet,
    types::{Address, Log},
};
use ethers_solc::{
    artifacts::{BytecodeObject, CompactBytecode, CompactContractBytecode, Libraries},
    contracts::ArtifactContracts,
    ArtifactId, Graph, Project,
};
use forge::{
    debug::{DebugArena, DebugStep},
    trace::Traces,
    CallKind,
};
use foundry_common::get_contract_name;
use std::{
    collections::{BTreeMap, HashMap},
    convert::From,
    fs, io,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::log::trace;

/// Standardized way of firing up the debugger
pub struct DebuggerArgs<'a> {
    pub success: bool,
    pub debug: Vec<DebugArena>,
    pub path: PathBuf,
    pub decoder: &'a CallTraceDecoder,
    pub sources: BTreeMap<ArtifactId, String>,
    pub project: &'a Project,
    pub highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    pub breakpoints: Breakpoints,
    /// Map over file_id -> path
    pub file_ids: BTreeMap<u32, String>,
}

impl DebuggerArgs<'_> {
    pub fn run(&self) -> eyre::Result<TUIExitReason> {
        // trace!(target: "debugger", "running debugger");

        let (sources, artifacts) = filter_sources_and_artifacts(
            self.path.as_os_str().to_str().unwrap(),
            self.sources.clone(),
            self.highlevel_known_contracts.clone(),
            &self.project,
        )?;
        let flattened = self
            .debug
            .last()
            .map(|arena| arena.flatten(0))
            .expect("We should have collected debug information");
        let identified_contracts = self
            .decoder
            .contracts
            .iter()
            .map(|(addr, identifier)| (*addr, get_contract_name(identifier).to_string()))
            .collect();

        let known_contracts_sources = self
            .highlevel_known_contracts
            .iter()
            .map(|(id, _)| {
                let file_sources: BTreeMap<u32, String> = self
                    .file_ids
                    .iter()
                    .map(|(id, path)| {
                        (*id, fs::read_to_string(path).expect("failed to read source"))
                    })
                    .collect();
                (id.name.clone(), file_sources)
            })
            .collect();

        let mut tui = Tui::new(
            flattened,
            0,
            identified_contracts,
            artifacts,
            known_contracts_sources,
            self.breakpoints.clone(),
        )?;

        tui.launch()
    }
}

/// Resolve the import tree of our target path, and get only the artifacts and
/// sources we need. If it's a standalone script, don't filter anything out.
pub fn filter_sources_and_artifacts(
    target: &str,
    sources: BTreeMap<ArtifactId, String>,
    highlevel_known_contracts: ArtifactContracts<ContractBytecodeSome>,
    project: &Project,
) -> eyre::Result<(BTreeMap<ArtifactId, String>, HashMap<String, ContractBytecodeSome>)> {
    // Find all imports
    let graph = Graph::resolve(&project.paths)?;
    let target_path = project.root().join(target);
    let mut target_tree = BTreeMap::new();
    let mut is_standalone = false;

    if let Some(target_index) = graph.files().get(&target_path) {
        target_tree.extend(
            graph
                .all_imported_nodes(*target_index)
                .map(|index| graph.node(index).unpack())
                .collect::<BTreeMap<_, _>>(),
        );

        // Add our target into the tree as well.
        let (target_path, target_source) = graph.node(*target_index).unpack();
        target_tree.insert(target_path, target_source);
    } else {
        is_standalone = true;
    }

    let sources = sources
        .into_iter()
        .filter_map(|(id, path)| {
            let mut resolved = project
                .paths
                .resolve_library_import(project.root(), &PathBuf::from(&path))
                .unwrap_or_else(|| PathBuf::from(&path));

            if !resolved.is_absolute() {
                resolved = project.root().join(&resolved);
            }

            if !is_standalone {
                target_tree.get(&resolved).map(|source| (id, source.content.as_str().to_string()))
            } else {
                Some((
                    id,
                    fs::read_to_string(&resolved).unwrap_or_else(|_| {
                        panic!("Something went wrong reading the source file: {path:?}")
                    }),
                ))
            }
        })
        .collect();

    let artifacts = highlevel_known_contracts
        .into_iter()
        .filter_map(|(id, artifact)| {
            if !is_standalone {
                target_tree.get(&id.source).map(|_| (id.name, artifact))
            } else {
                Some((id.name, artifact))
            }
        })
        .collect();

    Ok((sources, artifacts))
}
