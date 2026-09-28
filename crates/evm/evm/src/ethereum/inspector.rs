//! Foundry inspectors composed for Ethereum execution.

use alloy_primitives::Log;
use evm2::{
    EvmTypesHost, Inspector,
    evm::Database,
    interpreter::{Interpreter, Message, MessageResult},
};
use foundry_cheatcodes::{
    CheatsConfig,
    ethereum::{CheatcodeAccessMode, EthereumCheatcodes},
};
use foundry_evm_core::ethereum::{FoundryEvmTypes, LocalState};
use std::sync::Arc;

/// Inspector state and observations retained by the Ethereum executor.
#[derive(Clone, Debug)]
pub struct EthereumInspectorStack {
    cheatcodes: EthereumCheatcodes,
    logs: Vec<Log>,
}

impl EthereumInspectorStack {
    /// Creates the inspector stack for a test or script execution.
    pub fn new(config: Arc<CheatsConfig>, access_mode: CheatcodeAccessMode) -> Self {
        Self { cheatcodes: EthereumCheatcodes::new(config, access_mode), logs: Vec::new() }
    }

    /// Installs contracts required by the inspectors.
    pub fn install<D: Database + Clone>(&self, state: &mut LocalState<D>) {
        self.cheatcodes.install(state);
    }

    /// Returns mutable cheatcode state, including fork caller authorization.
    pub const fn cheatcodes_mut(&mut self) -> &mut EthereumCheatcodes {
        &mut self.cheatcodes
    }

    /// Drains EVM logs collected in execution order.
    pub fn take_logs(&mut self) -> Vec<Log> {
        std::mem::take(&mut self.logs)
    }
}

impl Inspector<FoundryEvmTypes> for EthereumInspectorStack {
    fn log(&mut self, log: &Log, _host: &mut <FoundryEvmTypes as EvmTypesHost>::Host<'_>) {
        self.logs.push(log.clone());
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        self.cheatcodes.call(interp, message)
    }
}
