use alloy_primitives::Address;
use foundry_common::{compile::ContractSources, evm::Breakpoints};
use foundry_evm_core::{debug::DebugNodeFlat, utils::PcIcMap};
use std::collections::{BTreeMap, HashMap};

pub struct DebuggerContext {
    pub debug_arena: Vec<DebugNodeFlat>,
    pub identified_contracts: HashMap<Address, String>,
    /// Source map of contract sources
    pub contracts_sources: ContractSources,
    /// A mapping of source -> (PC -> IC map for deploy code, PC -> IC map for runtime code)
    pub pc_ic_maps: BTreeMap<String, (PcIcMap, PcIcMap)>,
    pub breakpoints: Breakpoints,
}
