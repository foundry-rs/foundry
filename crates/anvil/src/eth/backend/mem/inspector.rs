//! Anvil specific [`revm::Inspector`] implementation

use crate::eth::macros::node_info;
use alloy_primitives::{Address, Log, U256};
use foundry_evm::{
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    traces::{
        CallTraceDecoder, SparsedTraceArena, TracingInspectorConfig, render_trace_arena_inner,
    },
};
use revm::{
    Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter,
        interpreter::EthInterpreter,
    },
};
use revm_inspectors::transfer::TransferInspector;
use std::sync::Arc;

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct AnvilInspector {
    /// Collects all traces
    pub tracer: Option<TracingInspector>,
    /// Collects all `console.sol` logs
    pub log_collector: Option<LogCollector>,
    /// Collects all internal ETH transfers as ERC20 transfer events.
    pub transfer: Option<TransferInspector>,
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
    pub fn into_print_traces(mut self, decoder: Arc<CallTraceDecoder>) {
        if let Some(a) = self.tracer.take() {
            print_traces(a, decoder);
        }
    }

    /// Called after the inspecting the evm
    /// This will log all traces
    pub fn print_traces(&self, decoder: Arc<CallTraceDecoder>) {
        if let Some(a) = self.tracer.clone() {
            print_traces(a, decoder);
        }
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));
        self
    }

    /// Configures the `TracingInspector` [`revm::Inspector`]
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

    /// Configures the `Tracer` [`revm::Inspector`] with a transfer event collector
    pub fn with_transfers(mut self) -> Self {
        self.transfer = Some(TransferInspector::new(false).with_logs(true));
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
fn print_traces(tracer: TracingInspector, decoder: Arc<CallTraceDecoder>) {
    let arena = tokio::task::block_in_place(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let mut arena = tracer.into_traces();
            decoder.populate_traces(arena.nodes_mut()).await;
            arena
        })
    });

    let traces = SparsedTraceArena { arena, ignored: Default::default() };
    node_info!("Traces:");
    node_info!("{}", render_trace_arena_inner(&traces, false, true));
}

impl<CTX> Inspector<CTX, EthInterpreter> for AnvilInspector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, ecx);
        });
    }

    fn step(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, ecx);
        });
    }

    fn step_end(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, ecx);
        });
    }

    #[allow(clippy::redundant_clone)]
    fn log(&mut self, interp: &mut Interpreter, ecx: &mut CTX, log: Log) {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log(interp, ecx, log.clone());
        });
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        call_inspectors!(
            #[ret]
            [&mut self.tracer, &mut self.log_collector, &mut self.transfer],
            |inspector| inspector.call(ecx, inputs).map(Some),
        );
        None
    }

    fn call_end(&mut self, ecx: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        if let Some(tracer) = &mut self.tracer {
            tracer.call_end(ecx, inputs, outcome);
        }
    }

    fn create(&mut self, ecx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        call_inspectors!(
            #[ret]
            [&mut self.tracer, &mut self.transfer],
            |inspector| inspector.create(ecx, inputs).map(Some),
        );
        None
    }

    fn create_end(&mut self, ecx: &mut CTX, inputs: &CreateInputs, outcome: &mut CreateOutcome) {
        if let Some(tracer) = &mut self.tracer {
            tracer.create_end(ecx, inputs, outcome);
        }
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        call_inspectors!([&mut self.tracer, &mut self.transfer], |inspector| {
            Inspector::<CTX, EthInterpreter>::selfdestruct(inspector, contract, target, value)
        });
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        tracing::info!(target: crate::logging::EVM_CONSOLE_LOG_TARGET, "{}", log);
    }
}
