//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, revm::Database};
use bytes::Bytes;
use ethers::types::Log;
use forge::revm::primitives::{B160, B256};
use foundry_evm::{
    call_inspectors,
    decode::{decode_console_log, decode_console_logs},
    executor::inspector::{EvmEventLogger, OnLog, Tracer},
    revm,
    revm::{
        inspectors::GasInspector,
        interpreter::{CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
        EVMData,
    },
};
use std::{cell::RefCell, fmt::Debug, rc::Rc};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Debug, Clone, Default)]
pub struct Inspector {
    pub gas: Option<Rc<RefCell<GasInspector>>>,
    pub tracer: Option<Tracer>,
    /// collects all `console.sol` logs
    pub logger: EvmEventLogger<InlineLogs>,
}

/*
impl<ONLOG: OnLog> std::fmt::Debug for Inspector<ONLOG> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inspector")
            .field("gas", &self.gas)
            .field("tracer", &self.tracer)
            .field("logger", &self.logger)
            .finish()
    }
}
*/

// === impl Inspector ===

#[derive(Debug, Clone, Default)]
pub struct InlineLogs;

impl OnLog for InlineLogs {
    type OnLogState = bool;
    fn on_log(inline_logs_enabled: &mut Self::OnLogState, log_entry: &Log) {
        if *inline_logs_enabled {
            node_info!("{}", decode_console_log(log_entry).unwrap_or(format!("{:?}", log_entry)));
        }
    }
}

impl Inspector {
    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        print_logs(&self.logger.logs)
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(Default::default());
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_inline_logs(mut self) -> Self {
        self.logger.on_log_state = true;
        self
    }

    /// Enables steps recording for `Tracer` and attaches `GasInspector` to it
    /// If `Tracer` wasn't configured before, configures it automatically
    pub fn with_steps_tracing(mut self) -> Self {
        if self.tracer.is_none() {
            self = self.with_tracing()
        }
        let gas_inspector = Rc::new(RefCell::new(GasInspector::default()));
        self.gas = Some(gas_inspector.clone());
        self.tracer = self.tracer.map(|tracer| tracer.with_steps_recording(gas_inspector));

        self
    }
}

impl<DB: Database> revm::Inspector<DB> for Inspector {
    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            { inspector.initialize_interp(interp, data, is_static) }
        );
        InstructionResult::Continue
    }

    fn step(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            {
                inspector.step(interp, data, is_static);
            }
        );
        InstructionResult::Continue
    }

    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &B160,
        topics: &[B256],
        data: &Bytes,
    ) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.tracer,
                Some(&mut self.logger)
            ],
            {
                inspector.log(evm_data, address, topics, data);
            }
        );
    }

    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        eval: InstructionResult,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            {
                inspector.step_end(interp, data, is_static, eval);
            }
        );
        eval
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.tracer,
                Some(&mut self.logger)
            ],
            {
                inspector.call(data, call, is_static);
            }
        );

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CallInputs,
        remaining_gas: Gas,
        ret: InstructionResult,
        out: Bytes,
        is_static: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            {
                inspector.call_end(data, inputs, remaining_gas, ret, out.clone(), is_static);
            }
        );
        (ret, remaining_gas, out)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            {
                inspector.create(data, call);
            }
        );

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CreateInputs,
        status: InstructionResult,
        address: Option<B160>,
        gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [&mut self.gas.as_deref().map(|gas| gas.borrow_mut()), &mut self.tracer],
            {
                inspector.create_end(data, inputs, status, address, gas, retdata.clone());
            }
        );
        (status, address, gas, retdata)
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        node_info!("{}", log);
    }
}
