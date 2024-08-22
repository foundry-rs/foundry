//! The file dumper implementation

use crate::{context::DebuggerContext, DebugNode};
use alloy_primitives::Address;
use eyre::Result;
use foundry_common::fs::write_json_file;
use foundry_compilers::{artifacts::sourcemap::Jump, multi::MultiCompilerLanguage};
use foundry_evm_traces::debug::ContractSources;
use serde::Serialize;
use std::{collections::HashMap, ops::Deref, path::PathBuf};

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
            contracts: to_contracts_dump(debugger_context),
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

fn to_contracts_dump(debugger_context: &DebuggerContext) -> ContractsDump {
    ContractsDump {
        identified_contracts: debugger_context
            .identified_contracts
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect(),
        sources: to_contracts_sources_dump(&debugger_context.contracts_sources),
    }
}

fn to_contracts_sources_dump(contracts_sources: &ContractSources) -> ContractsSourcesDump {
    ContractsSourcesDump {
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
                                SourceDataDump {
                                    source: source_data.source.deref().clone(),
                                    language: source_data.language,
                                    path: source_data.path.clone(),
                                },
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
                        .map(|artifact_data| ArtifactDataDump {
                            source_map: artifact_data.source_map.clone().map(|source_map| {
                                source_map
                                    .iter()
                                    .map(|v| SourceElementDump {
                                        offset: v.offset(),
                                        length: v.length(),
                                        index: v.index_i32(),
                                        jump: match v.jump() {
                                            Jump::In => 0,
                                            Jump::Out => 1,
                                            Jump::Regular => 2,
                                        },
                                        modifier_depth: v.modifier_depth(),
                                    })
                                    .collect()
                            }),
                            source_map_runtime: artifact_data.source_map_runtime.clone().map(
                                |source_map| {
                                    source_map
                                        .iter()
                                        .map(|v| SourceElementDump {
                                            offset: v.offset(),
                                            length: v.length(),
                                            index: v.index_i32(),
                                            jump: match v.jump() {
                                                Jump::In => 0,
                                                Jump::Out => 1,
                                                Jump::Regular => 2,
                                            },
                                            modifier_depth: v.modifier_depth(),
                                        })
                                        .collect()
                                },
                            ),
                            pc_ic_map: artifact_data
                                .pc_ic_map
                                .clone()
                                .map(|v| v.inner.iter().map(|(k, v)| (*k, *v)).collect()),
                            pc_ic_map_runtime: artifact_data
                                .pc_ic_map_runtime
                                .clone()
                                .map(|v| v.inner.iter().map(|(k, v)| (*k, *v)).collect()),
                            build_id: artifact_data.build_id.clone(),
                            file_id: artifact_data.file_id,
                        })
                        .collect(),
                )
            })
            .collect(),
    }
}
