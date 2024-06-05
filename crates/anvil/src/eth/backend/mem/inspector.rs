//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, revm::Database};
use alloy_primitives::{Address, Log};
use foundry_evm::{
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    revm::{
        interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter},
        primitives::U256,
        EvmContext,
    },
    traces::TracingInspectorConfig,
    InspectorExt,
};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct Inspector {
    pub tracer: Option<TracingInspector>,
    /// collects all `console.sol` logs
    pub log_collector: LogCollector,
}

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
    fn initialize_interp(&mut self, interp: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, ecx);
        });
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, ecx);
        });
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, ecx);
        });
    }

    #[inline]
    fn log(&mut self, ecx: &mut EvmContext<DB>, log: &Log) {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
            inspector.log(ecx, log);
        });
    }

    #[inline]
    fn call(&mut self, ecx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
            if let Some(outcome) = inspector.call(ecx, inputs) {
                return Some(outcome);
            }
        });

        None
    }

    #[inline]
    fn call_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        if let Some(tracer) = &mut self.tracer {
            return tracer.call_end(ecx, inputs, outcome);
        }

        outcome
    }

    #[inline]
    fn create(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.create(ecx, inputs) {
                return Some(out);
            }
        }
        None
    }

    #[inline]
    fn create_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        if let Some(tracer) = &mut self.tracer {
            return tracer.create_end(ecx, inputs, outcome);
        }

        outcome
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        if let Some(tracer) = &mut self.tracer {
            revm::Inspector::<DB>::selfdestruct(tracer, contract, target, value);
        }
    }
}

impl<DB: Database> InspectorExt<DB> for Inspector {}

/// Prints all the logs
#[inline]
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        node_info!("{}", log);
    }
}
