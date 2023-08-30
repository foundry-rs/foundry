use crate::Ui;
use ethers::solc::artifacts::ContractBytecodeSome;
use foundry_common::{evm::Breakpoints, get_contract_name};
use foundry_evm::{debug::DebugArena, trace::CallTraceDecoder};
use std::collections::HashMap;
use tracing::trace;

use crate::{TUIExitReason, Tui};

/// Map over debugger contract sources name -> file_id -> (source, contract)
pub type ContractSources = HashMap<String, HashMap<u32, (String, ContractBytecodeSome)>>;

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

        let contract_sources = self.sources.clone();

        let tui = Tui::new(
            flattened,
            0,
            identified_contracts,
            contract_sources,
            self.breakpoints.clone(),
        )?;

        tui.start()
    }
}
