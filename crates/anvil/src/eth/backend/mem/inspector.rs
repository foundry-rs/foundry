//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, revm::Database};
use alloy_rpc_types::Log;
use foundry_evm::{
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    revm,
    revm::{
        interpreter::{CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
        primitives::{Address, Bytes, B256},
        EVMData,
    },
    traces::TracingInspectorConfig,
};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct Inspector {
    pub tracer: Option<TracingInspector>,
    /// collects all `console.sol` logs
    pub log_collector: LogCollector,
}

// === impl Inspector ===

impl Inspector {
    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        print_logs(&self.log_collector.logs)
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));
        self
    }

    /// Enables steps recording for `Tracer`.
    pub fn with_steps_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all()));
        self
    }
}

impl<DB: Database> revm::Inspector<DB> for Inspector {
    #[inline]
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, data);
        });
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, data);
        });
    }

    #[inline]
    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &Address,
        topics: &[B256],
        data: &Bytes,
    ) {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
            inspector.log(evm_data, address, topics, data);
        });
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, data);
        });
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
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
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
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
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
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
