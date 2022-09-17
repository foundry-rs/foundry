//! Anvil specific [`revm::Inspector`] implementation

use crate::{
    eth::macros::node_info,
    revm::{CreateInputs, Database, Interpreter},
};
use bytes::Bytes;
use ethers::types::{Address, Log, H256};
use foundry_evm::{
    decode::decode_console_logs,
    executor::inspector::{LogCollector, Tracer},
    revm,
    revm::{CallInputs, EVMData, Gas, Return},
};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Debug, Clone, Default)]
pub struct Inspector {
    pub tracer: Option<Tracer>,
    /// collects all `console.sol` logs
    pub logs: LogCollector,
}

// === impl Inspector ===

impl Inspector {
    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        print_logs(&self.logs.logs)
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(Default::default());
        self
    }

    /// Enables steps recording for `Tracer`
    /// If `Tracer` wasn't configured before, configures it automatically
    pub fn with_steps_tracing(mut self) -> Self {
        if self.tracer.is_none() {
            self = self.with_tracing()
        }
        self.tracer = self.tracer.map(|tracer| tracer.with_steps_recording());

        self
    }
}

impl<DB: Database> revm::Inspector<DB> for Inspector {
    fn step(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.step(interp, data, is_static);
        }
        Return::Continue
    }

    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &Address,
        topics: &[H256],
        data: &Bytes,
    ) {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.log(evm_data, address, topics, data);
        }
        self.logs.log(evm_data, address, topics, data);
    }

    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        eval: Return,
    ) -> Return {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.step_end(interp, data, is_static, eval);
        }
        eval
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.call(data, call, is_static);
        }
        self.logs.call(data, call, is_static);

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CallInputs,
        remaining_gas: Gas,
        ret: Return,
        out: Bytes,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.call_end(data, inputs, remaining_gas, ret, out.clone(), is_static);
        }
        (ret, remaining_gas, out)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.create(data, call);
        }

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CreateInputs,
        status: Return,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        if let Some(tracer) = self.tracer.as_mut() {
            tracer.create_end(data, inputs, status, address, gas, retdata.clone());
        }
        (status, address, gas, retdata)
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        node_info!("{}", log);
    }
}
