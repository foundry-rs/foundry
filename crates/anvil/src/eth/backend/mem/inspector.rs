//! Anvil specific [`revm::Inspector`] implementation

use crate::eth::macros::node_info;
use alloy_primitives::{Address, B256, Log, LogData, U256};
use alloy_sol_types::SolValue;
use foundry_evm::{
    call_inspectors,
    decode::decode_console_logs,
    inspectors::{LogCollector, TracingInspector},
    traces::{
        CallTraceDecoder, CallTraceNode, SparsedTraceArena, TracingInspectorConfig,
        render_trace_arena_inner,
    },
};
use revm::{
    Inspector,
    context::{ContextTr, JournalTr},
    inspector::JournalExt,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, CreateScheme, Interpreter,
        interpreter::EthInterpreter,
    },
};
use revm_inspectors::transfer::{TRANSFER_EVENT_TOPIC, TRANSFER_LOG_EMITTER, TransferInspector};
use std::sync::Arc;

/// A log emitted while simulating a transaction and its attempted execution-order index.
#[derive(Clone, Debug)]
pub struct SimulationLog {
    /// The emitted log.
    pub log: Log,
    /// The log's index among all attempted logs in the transaction.
    pub index: usize,
}

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct AnvilInspector {
    /// Collects all traces
    pub tracer: Option<TracingInspector>,
    /// Collects all `console.sol` logs
    pub log_collector: Option<LogCollector>,
    /// Collects all internal ETH transfers as ERC20 transfer events.
    pub transfer: Option<TransferInspector>,
    /// Canonical and synthetic logs retained from successful execution frames.
    simulation_logs: Vec<SimulationLog>,
    /// The retained-log length at the start of each execution frame.
    simulation_log_checkpoints: Vec<usize>,
    /// Counts every attempted canonical and synthetic log, including reverted frames.
    attempted_simulation_log_count: usize,
}

/// Configuration for per-transaction inspector lifecycle.
#[derive(Clone, Debug)]
pub struct InspectorTxConfig {
    /// Whether to print traces to stdout.
    pub print_traces: bool,
    /// Whether to print logs to stdout.
    pub print_logs: bool,
    /// Whether to enable step-level tracing (with state diffs).
    pub enable_steps_tracing: bool,
    /// Decoder for populating trace labels.
    pub call_trace_decoder: Arc<CallTraceDecoder>,
}

impl AnvilInspector {
    /// Returns simulation response logs in execution order.
    pub fn simulation_logs(&self) -> &[SimulationLog] {
        &self.simulation_logs
    }

    /// Returns the number of logs attempted during execution, including reverted frames.
    pub const fn attempted_simulation_log_count(&self) -> usize {
        self.attempted_simulation_log_count
    }

    /// Finish a transaction: print traces/logs, drain the tracer, and reset for the next tx.
    ///
    /// Returns the collected call trace nodes from the finished transaction.
    pub fn finish_transaction(&mut self, config: &InspectorTxConfig) -> Vec<CallTraceNode> {
        // Print before draining so the tracer is still populated.
        if config.print_traces {
            self.print_traces(config.call_trace_decoder.clone());
        }
        self.print_logs();

        let traces = self.tracer.take().map(|t| t.into_traces().into_nodes()).unwrap_or_default();
        self.simulation_logs.clear();
        self.simulation_log_checkpoints.clear();
        self.attempted_simulation_log_count = 0;

        // Reinstall tracer for next tx.
        let tracing_config = if config.enable_steps_tracing {
            TracingInspectorConfig::all().with_state_diffs()
        } else {
            TracingInspectorConfig::all().set_steps(false)
        };
        self.tracer = Some(TracingInspector::new(tracing_config));

        // Reset log collector for next tx.
        if config.print_logs {
            self.log_collector = Some(LogCollector::Capture { logs: Vec::new() });
        }

        traces
    }

    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        if let Some(LogCollector::Capture { logs }) = &self.log_collector {
            print_logs(logs);
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
        self.log_collector = Some(LogCollector::Capture { logs: Vec::new() });
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a transfer event collector
    pub fn with_transfers(mut self) -> Self {
        self.transfer = Some(TransferInspector::new(false));
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a trace printer
    pub fn with_trace_printer(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().with_state_diffs()));
        self
    }

    fn record_simulation_log(&mut self, log: Log) {
        let index = self.attempted_simulation_log_count;
        self.attempted_simulation_log_count += 1;
        self.simulation_logs.push(SimulationLog { log, index });
    }

    fn record_transfer(&mut self, from: Address, to: Address, value: U256) {
        if self.transfer.is_none() || value.is_zero() {
            return;
        }
        let from = B256::from_slice(&from.abi_encode());
        let to = B256::from_slice(&to.abi_encode());
        let data = value.abi_encode();
        self.record_simulation_log(Log {
            address: TRANSFER_LOG_EMITTER,
            data: LogData::new_unchecked(vec![TRANSFER_EVENT_TOPIC, from, to], data.into()),
        });
    }

    fn start_simulation_frame(&mut self) {
        self.simulation_log_checkpoints.push(self.simulation_logs.len());
    }

    fn end_simulation_frame(&mut self, success: bool) {
        if let Some(checkpoint) = self.simulation_log_checkpoints.pop()
            && !success
        {
            self.simulation_logs.truncate(checkpoint);
        }
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
    let trace = render_trace_arena_inner(&traces, false, true);
    node_info!(Traces = %format!("\n{}", trace));
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
    fn log(&mut self, ecx: &mut CTX, log: Log) {
        self.record_simulation_log(log.clone());
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log(ecx, log.clone());
        });
    }

    #[allow(clippy::redundant_clone)]
    fn log_full(&mut self, interp: &mut Interpreter, ecx: &mut CTX, log: Log) {
        self.record_simulation_log(log.clone());
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log_full(interp, ecx, log.clone());
        });
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.start_simulation_frame();
        if let Some(value) = inputs.transfer_value() {
            self.record_transfer(inputs.transfer_from(), inputs.transfer_to(), value);
        }
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
        self.end_simulation_frame(outcome.result.result.is_ok());
    }

    fn create(&mut self, ecx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.start_simulation_frame();
        if !matches!(inputs.scheme(), CreateScheme::Custom { .. })
            && let Ok(account) = ecx.journal_mut().load_account(inputs.caller())
        {
            self.record_transfer(
                inputs.caller(),
                inputs.created_address(account.data.info.nonce),
                inputs.value(),
            );
        }
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
        self.end_simulation_frame(outcome.result.result.is_ok());
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        self.record_transfer(contract, target, value);
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
