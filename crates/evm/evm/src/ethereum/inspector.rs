//! Foundry inspectors composed for Ethereum execution.

use alloy_primitives::Log;
use evm2::{
    EvmTypesHost, Inspector,
    evm::{Database, inspector::CallAction},
    interpreter::{Interpreter, Message, MessageResult},
};
use foundry_cheatcodes::{
    CheatsConfig,
    ethereum::{CheatcodeAccessMode, EthereumCheatcodes},
};
use foundry_evm_core::ethereum::{FoundryEvmTypes, LocalState};
use foundry_evm_coverage::{HitMaps, NativeLineCoverageCollector};
use std::sync::Arc;

/// Inspector state and observations retained by the Ethereum executor.
#[derive(Clone, Debug)]
pub struct EthereumInspectorStack {
    cheatcodes: EthereumCheatcodes,
    logs: Vec<Log>,
    coverage: Option<NativeLineCoverageCollector>,
}

impl EthereumInspectorStack {
    /// Creates the inspector stack for a test or script execution.
    pub fn new(config: Arc<CheatsConfig>, access_mode: CheatcodeAccessMode) -> Self {
        Self {
            cheatcodes: EthereumCheatcodes::new(config, access_mode),
            logs: Vec::new(),
            coverage: None,
        }
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

    /// Starts collecting bytecode coverage for subsequent execution.
    pub fn enable_line_coverage(&mut self) {
        self.coverage = Some(NativeLineCoverageCollector::default());
    }

    /// Returns bytecode coverage collected since the last drain.
    pub fn take_line_coverage(&mut self) -> Option<HitMaps> {
        self.coverage.as_mut().map(NativeLineCoverageCollector::take)
    }
}

impl Inspector<FoundryEvmTypes> for EthereumInspectorStack {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        if let Some(coverage) = &mut self.coverage {
            coverage.initialize_interp(interp);
        }
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
        if let Some(coverage) = &mut self.coverage {
            coverage.step(interp);
        }
    }

    fn log(&mut self, log: &Log, _host: &mut <FoundryEvmTypes as EvmTypesHost>::Host<'_>) {
        self.logs.push(log.clone());
    }

    fn call_action(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> CallAction<FoundryEvmTypes> {
        self.cheatcodes.call_action(interp, message)
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        self.cheatcodes.call_end(interp, message, result);
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &mut Message<FoundryEvmTypes>,
    ) -> Option<MessageResult<FoundryEvmTypes>> {
        self.cheatcodes.create(interp, message)
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
    ) {
        if let Some(coverage) = &mut self.coverage {
            coverage.create_end(interp, message, result);
        }
        self.cheatcodes.create_end(interp, message, result);
    }
}
