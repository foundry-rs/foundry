//! Anvil specific [`revm::Inspector`] implementation

use alloy_primitives::{Address, Log, U256};
use foundry_evm::{
    backend::DatabaseError,
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    traces::{
        render_trace_arena_inner, CallTraceDecoder, SparsedTraceArena, TracingInspectorConfig,
    },
};
use revm::{
    interpreter::{
        interpreter::EthInterpreter, CallInputs, CallOutcome, CreateInputs, CreateOutcome,
        EOFCreateInputs, Interpreter,
    },
    Database, Inspector,
};

use crate::eth::{backend::executor::AnvilEvmContext, macros::node_info};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct AnvilInspector {
    /// Collects all traces
    pub tracer: Option<TracingInspector>,
    /// Collects all `console.sol` logs
    pub log_collector: Option<LogCollector>,
}

impl AnvilInspector {
    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        if let Some(collector) = &self.log_collector {
            print_logs(&collector.logs);
        }
    }

    /// Consumes the type and prints the traces.
    pub fn into_print_traces(mut self) {
        if let Some(a) = self.tracer.take() {
            print_traces(a)
        }
    }

    /// Called after the inspecting the evm
    /// This will log all traces
    pub fn print_traces(&self) {
        if let Some(a) = self.tracer.clone() {
            print_traces(a)
        }
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));
        self
    }

    pub fn with_tracing_config(mut self, config: TracingInspectorConfig) -> Self {
        self.tracer = Some(TracingInspector::new(config));
        self
    }

    /// Enables steps recording for `Tracer`.
    pub fn with_steps_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().with_state_diffs()));
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a log collector
    pub fn with_log_collector(mut self) -> Self {
        self.log_collector = Some(Default::default());
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a trace printer
    pub fn with_trace_printer(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().with_state_diffs()));
        self
    }
}

/// Prints the traces for the inspector
///
/// Caution: This blocks on call trace decoding
///
/// # Panics
///
/// If called outside tokio runtime
fn print_traces(tracer: TracingInspector) {
    let arena = tokio::task::block_in_place(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let mut arena = tracer.into_traces();
            let decoder = CallTraceDecoder::new();
            decoder.populate_traces(arena.nodes_mut()).await;
            arena
        })
    });

    let traces = SparsedTraceArena { arena, ignored: Default::default() };
    node_info!("Traces:");
    node_info!("{}", render_trace_arena_inner(&traces, false, true));
}

impl<D> Inspector<AnvilEvmContext<'_, D>, EthInterpreter> for AnvilInspector
where
    D: Database<Error = DatabaseError>,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter, ecx: &mut AnvilEvmContext<'_, D>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, ecx);
        });
    }

    fn step(&mut self, interp: &mut Interpreter, ecx: &mut AnvilEvmContext<'_, D>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, ecx);
        });
    }

    fn step_end(&mut self, interp: &mut Interpreter, ecx: &mut AnvilEvmContext<'_, D>) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, ecx);
        });
    }

    fn log(&mut self, interp: &mut Interpreter, ecx: &mut AnvilEvmContext<'_, D>, log: Log) {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log(interp, ecx, log);
        });
    }

    fn call(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| inspector
            .call(ecx, inputs)
            .map(Some),);
        None
    }

    fn call_end(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        if let Some(tracer) = &mut self.tracer {
            tracer.call_end(ecx, inputs, outcome);
        }
    }

    fn create(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.create(ecx, inputs) {
                return Some(out);
            }
        }
        None
    }

    fn create_end(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if let Some(tracer) = &mut self.tracer {
            tracer.create_end(ecx, inputs, outcome);
        }
    }

    #[inline]
    fn eofcreate(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &mut EOFCreateInputs,
    ) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.eofcreate(ecx, inputs) {
                return Some(out);
            }
        }
        None
    }

    #[inline]
    fn eofcreate_end(
        &mut self,
        ecx: &mut AnvilEvmContext<'_, D>,
        inputs: &EOFCreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if let Some(tracer) = &mut self.tracer {
            tracer.eofcreate_end(ecx, inputs, outcome);
        }
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        if let Some(tracer) = &mut self.tracer {
            Inspector::<AnvilEvmContext<'_, D>>::selfdestruct(tracer, contract, target, value);
        }
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        tracing::info!(target: crate::logging::EVM_CONSOLE_LOG_TARGET, "{}", log);
    }
}
