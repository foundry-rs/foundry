use crate::{DebugNode, debugger::DebuggerContext};
use alloy_primitives::map::AddressMap;
use foundry_common::fs::write_json_file;
use foundry_compilers::{
    artifacts::sourcemap::{Jump, SourceElement},
    multi::MultiCompilerLanguage,
};
use foundry_evm_core::ic::PcIcMap;
use foundry_evm_traces::debug::{ArtifactData, ContractSources, SourceData};
use serde::Serialize;
use std::{collections::HashMap, path::Path};

/// Dumps debugger data to a JSON file.
pub(crate) fn dump(path: &Path, context: &DebuggerContext) -> eyre::Result<()> {
    write_json_file(path, &DebuggerDump::new(context))?;
    Ok(())
}

/// Holds info of debugger dump.
#[derive(Serialize)]
struct DebuggerDump<'a> {
    contracts: ContractsDump<'a>,
    debug_arena: &'a [DebugNode],
}

impl<'a> DebuggerDump<'a> {
    fn new(debugger_context: &'a DebuggerContext) -> Self {
        Self {
            contracts: ContractsDump::new(debugger_context),
            debug_arena: &debugger_context.debug_arena,
        }
    }
}

#[derive(Serialize)]
struct SourceElementDump {
    offset: u32,
    length: u32,
    index: i32,
    jump: u32,
    modifier_depth: u32,
}

impl SourceElementDump {
    fn new(v: &SourceElement) -> Self {
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

#[derive(Serialize)]
struct ContractsDump<'a> {
    identified_contracts: &'a AddressMap<String>,
    sources: ContractsSourcesDump<'a>,
}

impl<'a> ContractsDump<'a> {
    fn new(debugger_context: &'a DebuggerContext) -> Self {
        Self {
            identified_contracts: &debugger_context.identified_contracts,
            sources: ContractsSourcesDump::new(&debugger_context.contracts_sources),
        }
    }
}

#[derive(Serialize)]
struct ContractsSourcesDump<'a> {
    sources_by_id: HashMap<&'a str, HashMap<u32, SourceDataDump<'a>>>,
    artifacts_by_name: HashMap<&'a str, Vec<ArtifactDataDump<'a>>>,
}

impl<'a> ContractsSourcesDump<'a> {
    fn new(contracts_sources: &'a ContractSources) -> Self {
        Self {
            sources_by_id: contracts_sources
                .sources_by_id
                .iter()
                .map(|(name, inner_map)| {
                    (
                        name.as_str(),
                        inner_map
                            .iter()
                            .map(|(id, source_data)| (*id, SourceDataDump::new(source_data)))
                            .collect(),
                    )
                })
                .collect(),
            artifacts_by_name: contracts_sources
                .artifacts_by_name
                .iter()
                .map(|(name, data)| {
                    (name.as_str(), data.iter().map(ArtifactDataDump::new).collect())
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct SourceDataDump<'a> {
    source: &'a str,
    language: MultiCompilerLanguage,
    path: &'a Path,
}

impl<'a> SourceDataDump<'a> {
    fn new(v: &'a SourceData) -> Self {
        Self { source: &v.source, language: v.language, path: &v.path }
    }
}

#[derive(Serialize)]
struct ArtifactDataDump<'a> {
    source_map: Option<Vec<SourceElementDump>>,
    source_map_runtime: Option<Vec<SourceElementDump>>,
    pc_ic_map: Option<&'a PcIcMap>,
    pc_ic_map_runtime: Option<&'a PcIcMap>,
    build_id: &'a str,
    file_id: u32,
}

impl<'a> ArtifactDataDump<'a> {
    fn new(v: &'a ArtifactData) -> Self {
        Self {
            source_map: v
                .source_map
                .as_ref()
                .map(|source_map| source_map.iter().map(SourceElementDump::new).collect()),
            source_map_runtime: v
                .source_map_runtime
                .as_ref()
                .map(|source_map| source_map.iter().map(SourceElementDump::new).collect()),
            pc_ic_map: v.pc_ic_map.as_ref(),
            pc_ic_map_runtime: v.pc_ic_map_runtime.as_ref(),
            build_id: &v.build_id,
            file_id: v.file_id,
        }
    }
}
