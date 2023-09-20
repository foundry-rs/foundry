use crate::Ui;
use foundry_common::{compile::ContractSources, evm::Breakpoints, get_contract_name};
use foundry_evm::{debug::DebugArena, trace::CallTraceDecoder};
use foundry_utils::types::ToEthers;
use tracing::trace;

use crate::{TUIExitReason, Tui};

/// Standardized way of firing up the debugger
pub struct DebuggerArgs<'a> {
    /// debug traces returned from the execution
    pub debug: Vec<DebugArena>,
    /// trace decoder
    pub decoder: &'a CallTraceDecoder,
    /// map of source files
    pub sources: ContractSources,
    /// map of the debugger breakpoints
    pub breakpoints: Breakpoints,
}

impl DebuggerArgs<'_> {
    pub fn run(&self) -> eyre::Result<TUIExitReason> {
        trace!(target: "debugger", "running debugger");

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

        let tui = Tui::new(
            flattened.into_iter().map(|i| (i.0.to_ethers(), i.1, i.2)).collect(),
            0,
            identified_contracts,
            self.sources.clone(),
            self.breakpoints.clone(),
        )?;

        tui.start()
    }
}
