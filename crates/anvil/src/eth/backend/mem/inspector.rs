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
        CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, CreateScheme,
        Interpreter, interpreter::EthInterpreter,
    },
};
use revm_inspectors::transfer::{TRANSFER_EVENT_TOPIC, TRANSFER_LOG_EMITTER, TransferInspector};
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
    /// Collects canonical and synthetic transfer logs for an `eth_simulateV1` response.
    simulation_logs: Option<SimulationLogCollector>,
}

#[derive(Clone, Debug)]
struct SimulationLog {
    log: Log,
    index: u64,
    canonical: bool,
}

/// Collects simulation response logs without inserting synthetic logs into EVM state.
#[derive(Clone, Debug, Default)]
struct SimulationLogCollector {
    logs: Vec<SimulationLog>,
    checkpoints: Vec<usize>,
    next_index: u64,
    trace_transfers: bool,
    journal_log_count: usize,
}

impl SimulationLogCollector {
    fn push_log(&mut self, log: Log, canonical: bool) {
        self.logs.push(SimulationLog { log, index: self.next_index, canonical });
        self.next_index += 1;
    }

    fn push_canonical_log(&mut self, log: Log, journal_log_count: usize) {
        self.push_log(log, true);
        self.journal_log_count = journal_log_count;
    }

    fn sync_journal_logs(&mut self, logs: &[Log]) {
        self.journal_log_count = self.journal_log_count.min(logs.len());
        for log in &logs[self.journal_log_count..] {
            self.push_log(log.clone(), true);
        }
        self.journal_log_count = logs.len();
    }

    fn push_transfer(&mut self, from: Address, to: Address, value: U256) {
        if !self.trace_transfers || value.is_zero() {
            return;
        }
        self.push_log(
            Log {
                address: TRANSFER_LOG_EMITTER,
                data: LogData::new_unchecked(
                    vec![
                        TRANSFER_EVENT_TOPIC,
                        B256::from_slice(&from.abi_encode()),
                        B256::from_slice(&to.abi_encode()),
                    ],
                    value.abi_encode().into(),
                ),
            },
            false,
        );
    }

    fn frame_start(&mut self) {
        self.checkpoints.push(self.logs.len());
    }

    fn frame_end(&mut self, success: bool, journal_log_count: usize) {
        let checkpoint = self.checkpoints.pop().expect("execution frame checkpoint exists");
        if !success {
            self.logs.truncate(checkpoint);
        }
        self.journal_log_count = journal_log_count;
    }

    fn append_remaining_canonical_logs(&mut self, canonical_logs: &[Log]) {
        let mut canonical_logs = canonical_logs.iter();
        for collected in self.logs.iter().filter(|log| log.canonical) {
            let canonical =
                canonical_logs.next().expect("collected canonical log exists in result");
            assert_eq!(&collected.log, canonical, "collected canonical logs preserve ordering");
        }
        for log in canonical_logs {
            self.push_log(log.clone(), true);
        }
    }
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
        self.transfer = Some(TransferInspector::new(false).with_logs(true));
        self
    }

    /// Collects canonical and synthetic transfer logs for an `eth_simulateV1` response.
    pub fn with_simulation_logs(mut self, trace_transfers: bool) -> Self {
        self.simulation_logs =
            Some(SimulationLogCollector { trace_transfers, ..Default::default() });
        self
    }

    /// Takes the collected `eth_simulateV1` response logs and attempted log count.
    pub fn take_simulation_logs(
        &mut self,
        canonical_logs: &[Log],
    ) -> Option<(Vec<(u64, Log)>, u64)> {
        self.simulation_logs.take().map(|mut collector| {
            collector.append_remaining_canonical_logs(canonical_logs);
            (
                collector.logs.into_iter().map(|log| (log.index, log.log)).collect(),
                collector.next_index,
            )
        })
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

    let traces =
        SparsedTraceArena { arena, ignored: Default::default(), diagnostics: Default::default() };
    let trace = render_trace_arena_inner(&traces, false, true);
    node_info!(Traces = %format!("\n{}", trace));
}

impl<CTX> Inspector<CTX, EthInterpreter> for AnvilInspector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
        }
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, ecx);
        });
    }

    fn step(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
        }
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, ecx);
        });
    }

    fn step_end(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, ecx);
        });
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
        }
    }

    #[allow(clippy::redundant_clone)]
    fn log(&mut self, ecx: &mut CTX, log: Log) {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log(ecx, log.clone());
        });
        if let Some(collector) = &mut self.simulation_logs {
            collector.push_canonical_log(log, ecx.journal().logs().len());
        }
    }

    #[allow(clippy::redundant_clone)]
    fn log_full(&mut self, interp: &mut Interpreter, ecx: &mut CTX, log: Log) {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            inspector.log_full(interp, ecx, log.clone());
        });
        if let Some(collector) = &mut self.simulation_logs {
            collector.push_canonical_log(log, ecx.journal().logs().len());
        }
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
            collector.frame_start();
            if matches!(inputs.scheme, CallScheme::Call)
                && let Some(value) = inputs.transfer_value()
            {
                collector.push_transfer(inputs.transfer_from(), inputs.transfer_to(), value);
            }
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
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
            collector.frame_end(outcome.instruction_result().is_ok(), ecx.journal().logs().len());
        }
    }

    fn create(&mut self, ecx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
            collector.frame_start();
            if matches!(inputs.scheme(), CreateScheme::Create | CreateScheme::Create2 { .. })
                && let Ok(account) = ecx.journal_mut().load_account(inputs.caller())
            {
                let address = inputs.created_address(account.data.info.nonce);
                collector.push_transfer(inputs.caller(), address, inputs.value());
            }
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
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(ecx.journal().logs());
            collector.frame_end(
                outcome.instruction_result().is_ok() && outcome.address.is_some(),
                ecx.journal().logs().len(),
            );
        }
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        call_inspectors!([&mut self.tracer, &mut self.transfer], |inspector| {
            Inspector::<CTX, EthInterpreter>::selfdestruct(inspector, contract, target, value)
        });
        if let Some(collector) = &mut self.simulation_logs {
            collector.push_transfer(contract, target, value);
        }
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        tracing::info!(target: crate::logging::EVM_CONSOLE_LOG_TARGET, "{}", log);
    }
}
