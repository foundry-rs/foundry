use crate::DebugNode;
use alloy_primitives::map::AddressHashMap;
use foundry_common::evm::Breakpoints;
use foundry_evm_traces::debug::ContractSources;

pub struct DebuggerContext {
    pub debug_arena: Vec<DebugNode>,
    pub identified_contracts: AddressHashMap<String>,
    /// Source map of contract sources
    pub contracts_sources: ContractSources,
    pub breakpoints: Breakpoints,
}
