//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, revm::Database};
use alloy_primitives::Log;
use foundry_evm::{
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    revm,
    revm::{
        interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter},
        EvmContext,
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
    fn initialize_interp(&mut self, interp: &mut Interpreter, context: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, context);
        });
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter, context: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, context);
        });
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter, context: &mut EvmContext<DB>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, context);
        });
    }

    #[inline]
    fn log(&mut self, context: &mut EvmContext<DB>, log: &Log) {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
            inspector.log(context, log);
        });
    }

    #[inline]
    fn call(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        call_inspectors!([&mut self.tracer, Some(&mut self.log_collector)], |inspector| {
            if let Some(outcome) = inspector.call(context, inputs) {
                return Some(outcome);
            }
        });

        None
    }

    #[inline]
    fn call_end(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        if let Some(tracer) = &mut self.tracer {
            return tracer.call_end(context, inputs, outcome);
        }

        outcome
    }

    #[inline]
    fn create(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.create(context, inputs) {
                return Some(out);
            }
        }
        None
    }

    #[inline]
    fn create_end(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        if let Some(tracer) = &mut self.tracer {
            return tracer.create_end(context, inputs, outcome);
        }

        outcome
    }
}

/// Prints all the logs
#[inline]
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        node_info!("{}", log);
    }
}
