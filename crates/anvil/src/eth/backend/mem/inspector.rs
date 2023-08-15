//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, revm::Database};
use bytes::Bytes;
use ethers::types::Log;
use foundry_evm::{
    call_inspectors,
    decode::{decode_console_log, decode_console_logs},
    executor::{EvmEventLogger, OnLog, Tracer},
    revm,
    revm::{
        interpreter::{CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
        primitives::{B160, B256},
        EVMData,
    },
};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Debug, Clone, Default)]
pub struct Inspector {
    pub tracer: Option<Tracer>,
    /// collects all `console.sol` logs
    pub logger: EvmEventLogger<InlineLogs>,
}

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

    /// Configures the `Executor` to emit logs and events as they are executed
    pub fn with_inline_logs(mut self) -> Self {
        self.logger.on_log_state = true;
        self
    }

    /// Enables steps recording for `Tracer` and attaches `GasInspector` to it
    /// If `Tracer` wasn't configured before, configures it automatically

    /// Enables steps recording for `Tracer`.
    pub fn with_steps_tracing(mut self) -> Self {
        let tracer = self.tracer.get_or_insert_with(Default::default);
        tracer.record_steps();
        self
    }
}

impl<DB: Database> revm::Inspector<DB> for Inspector {
    #[inline]
    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, data);
        });
        InstructionResult::Continue
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter, data: &mut EVMData<'_, DB>) -> InstructionResult {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, data);
        });
        InstructionResult::Continue
    }

    #[inline]
    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &B160,
        topics: &[B256],
        data: &Bytes,
    ) {
        call_inspectors!([&mut self.tracer, Some(&mut self.logger)], |inspector| {
            inspector.log(evm_data, address, topics, data);
        });
    }

    #[inline]
    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        eval: InstructionResult,
    ) -> InstructionResult {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, data, eval);
        });
        eval
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!([&mut self.tracer, Some(&mut self.logger)], |inspector| {
            inspector.call(data, call);
        });

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    #[inline]
    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CallInputs,
        remaining_gas: Gas,
        ret: InstructionResult,
        out: Bytes,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.call_end(data, inputs, remaining_gas, ret, out.clone());
        });
        (ret, remaining_gas, out)
    }

    #[inline]
    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.create(data, call);
        });

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    #[inline]
    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CreateInputs,
        status: InstructionResult,
        address: Option<B160>,
        gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.create_end(data, inputs, status, address, gas, retdata.clone());
        });
        (status, address, gas, retdata)
    }
}

/// Prints all the logs
#[inline]
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        node_info!("{}", log);
    }
}
