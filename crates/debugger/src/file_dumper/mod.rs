//! The file dumper implementation

use crate::{context::DebuggerContext, DebugNode};
use alloy_primitives::Address;
use eyre::Result;
use foundry_common::fs::write_json_file;
use foundry_compilers::{artifacts::sourcemap::Jump, multi::MultiCompilerLanguage};
use foundry_evm_traces::debug::{ArtifactData, ContractSources, SourceData};
use serde::Serialize;
use std::{collections::HashMap, ops::Deref, path::PathBuf};
use foundry_compilers::artifacts::sourcemap::SourceElement;

/// The file dumper
pub struct FileDumper<'a> {
    path: &'a PathBuf,
    debugger_context: &'a mut DebuggerContext,
}

impl<'a> FileDumper<'a> {
    pub fn new(path: &'a PathBuf, debugger_context: &'a mut DebuggerContext) -> Self {
        Self { path, debugger_context }
    }

    pub fn run(&mut self) -> Result<()> {
        let data = DebuggerDump::from(self.debugger_context);
        write_json_file(self.path, &data).unwrap();
        Ok(())
    }
}

impl DebuggerDump {
    fn from(debugger_context: &DebuggerContext) -> Self {
        Self {
            contracts: ContractsDump::new(debugger_context),
            debug_arena: debugger_context.debug_arena.clone(),
        }
    }
}

#[derive(Serialize)]
struct DebuggerDump {
    contracts: ContractsDump,
    debug_arena: Vec<DebugNode>,
}

#[derive(Serialize)]
pub struct SourceElementDump {
    offset: u32,
    length: u32,
    index: i32,
    jump: u32,
    modifier_depth: u32,
}

#[derive(Serialize)]
struct ContractsDump {
    // Map of call address to contract name
    identified_contracts: HashMap<Address, String>,
    sources: ContractsSourcesDump,
}

#[derive(Serialize)]
struct ContractsSourcesDump {
    sources_by_id: HashMap<String, HashMap<u32, SourceDataDump>>,
    artifacts_by_name: HashMap<String, Vec<ArtifactDataDump>>,
}

#[derive(Serialize)]
struct SourceDataDump {
    source: String,
    language: MultiCompilerLanguage,
    path: PathBuf,
}

#[derive(Serialize)]
struct ArtifactDataDump {
    pub source_map: Option<Vec<SourceElementDump>>,
    pub source_map_runtime: Option<Vec<SourceElementDump>>,
    pub pc_ic_map: Option<HashMap<usize, usize>>,
    pub pc_ic_map_runtime: Option<HashMap<usize, usize>>,
    pub build_id: String,
    pub file_id: u32,
}


impl ContractsDump {
    pub fn new(debugger_context: &DebuggerContext) -> Self {
        Self {
            identified_contracts: debugger_context
                .identified_contracts
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            sources: ContractsSourcesDump::new(&debugger_context.contracts_sources),
        }
    }
}

impl ContractsSourcesDump {
    pub fn new(contracts_sources: &ContractSources) -> Self {
        Self {
            sources_by_id: contracts_sources
                .sources_by_id
                .iter()
                .map(|(name, inner_map)| {
                    (
                        name.clone(),
                        inner_map
                            .iter()
                            .map(|(id, source_data)| {
                                (
                                    *id,
                                    SourceDataDump::new(source_data),
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
            artifacts_by_name: contracts_sources
                .artifacts_by_name
                .iter()
                .map(|(name, data)| {
                    (
                        name.clone(),
                        data.iter()
                            .map(ArtifactDataDump::new)
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl SourceDataDump {
    pub fn new(v: &SourceData) -> Self {
        Self {
            source: v.source.deref().clone(),
            language: v.language,
            path: v.path.clone(),
        }
    }
}

impl SourceElementDump {
    pub fn new(v: &SourceElement) -> Self {
        Self {
            offset: v.offset(),
            length: v.length(),
            index: v.index_i32(),
            jump: match v.jump() {
                Jump::In => 0,
                Jump::Out => 1,
                Jump::Regular => 2,
            },
            modifier_depth: v.modifier_depth(),
        }
    }
}

impl ArtifactDataDump {
    pub fn new(v: &ArtifactData) -> Self {
        Self {
            source_map: v.source_map.clone().map(|source_map| {
                source_map
                    .iter()
                    .map(SourceElementDump::new)
                    .collect()
            }),
            source_map_runtime: v.source_map_runtime.clone().map(
                |source_map| {
                    source_map
                        .iter()
                        .map(SourceElementDump::new)
                        .collect()
                },
            ),
            pc_ic_map: v
                .pc_ic_map
                .clone()
                .map(|v| v.inner.iter().map(|(k, v)| (*k, *v)).collect()),
            pc_ic_map_runtime: v
                .pc_ic_map_runtime
                .clone()
                .map(|v| v.inner.iter().map(|(k, v)| (*k, *v)).collect()),
            build_id: v.build_id.clone(),
            file_id: v.file_id,
        }
    }
}