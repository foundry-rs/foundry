//! Anvil specific [`evm2::Inspector`] implementation

use crate::eth::macros::node_info;
use alloy_primitives::{Address, B256, Log, LogData, U256};
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use evm2::{
    EvmTypes, Inspector,
    interpreter::{GasTracker, InstrStop, Interpreter, Message, MessageKind, MessageResult},
};
use evm2_inspectors::{
    tracing::{TracingInspector, TracingInspectorConfig, types::CallTraceNode},
    transfer::{TRANSFER_EVENT_TOPIC, TRANSFER_LOG_EMITTER, TransferInspector},
};
use foundry_common::{ErrorExt, fmt::ConsoleFmt};
use foundry_evm_core::{
    abi::console, constants::HARDHAT_CONSOLE_ADDRESS, decode::decode_console_logs,
};
use foundry_evm_traces::{CallTraceDecoder, SparsedTraceArena, render_trace_arena_inner};
use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
    sync::Arc,
};

/// The [`evm2::Inspector`] used when transacting in the evm
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
    /// Number of frames the tracer has entered.
    ///
    /// Tempo emits fee-token logs before the root frame starts; the tracer must not see hooks
    /// outside a frame.
    tracer_depth: usize,
}

/// Collects `LOG` opcode logs and Hardhat-style `console.sol` calls.
#[derive(Clone, Debug, Default)]
pub struct LogCollector {
    /// The collected logs.
    pub logs: Vec<Log>,
}

impl LogCollector {
    fn hardhat_log(&mut self, data: &[u8]) -> alloy_sol_types::Result<()> {
        let decoded = console::hh::ConsoleCalls::abi_decode(data)?;
        for line in decoded.fmt(Default::default()).lines() {
            self.logs.push(Log::new_unchecked(
                HARDHAT_CONSOLE_ADDRESS,
                vec![console::ds::log::SIGNATURE_HASH],
                line.abi_encode().into(),
            ));
        }
        Ok(())
    }
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
    /// Decoder for populating trace labels.
    pub call_trace_decoder: Arc<CallTraceDecoder>,
}

impl AnvilInspector {
    /// Finish a transaction: print traces/logs, drain the tracer, and reset for the next tx.
    ///
    /// Returns the collected call trace nodes from the finished transaction.
    pub fn finish_transaction(
        &mut self,
        config: &InspectorTxConfig,
        is_precompile: impl Fn(&Address) -> bool,
    ) -> Vec<CallTraceNode> {
        // Print before draining so the tracer is still populated.
        if config.print_traces {
            self.print_traces(config.call_trace_decoder.clone());
        }
        self.print_logs();

        let mut traces =
            self.tracer.take().map(|t| t.into_traces().into_nodes()).unwrap_or_default();
        // Record inclusion using the executing EVM's map, while retaining the full Geth graph.
        for node in &mut traces {
            if node.parent.is_some() && !node.trace.kind.is_any_create() {
                node.trace.maybe_precompile =
                    Some(node.trace.value.is_zero() && is_precompile(&node.trace.address));
            }
        }

        self.reset_transaction(config);

        traces
    }

    /// Discards a transaction's traces/logs and resets the inspector without printing them.
    pub fn discard_transaction(&mut self, config: &InspectorTxConfig) {
        self.reset_transaction(config);
    }

    /// Resets per-transaction collectors for the next transaction.
    fn reset_transaction(&mut self, config: &InspectorTxConfig) {
        // Reinstall tracer for next tx.
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));

        // Reset log collector for next tx.
        self.log_collector = config.print_logs.then(LogCollector::default);
    }

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

    /// Configures the `Tracer` [`evm2::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));
        self
    }

    /// Configures the `TracingInspector` [`evm2::Inspector`]
    pub fn with_tracing_config(mut self, config: TracingInspectorConfig) -> Self {
        self.tracer = Some(TracingInspector::new(config));
        self
    }

    /// Configures the `Tracer` [`evm2::Inspector`] with a log collector
    pub fn with_log_collector(mut self) -> Self {
        self.log_collector = Some(LogCollector::default());
        self
    }

    /// Configures the `Tracer` [`evm2::Inspector`] with a transfer event collector
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
        success: bool,
    ) -> Option<(Vec<(u64, Log)>, u64)> {
        self.simulation_logs.take().map(|mut collector| {
            if success {
                collector.append_remaining_canonical_logs(canonical_logs);
            } else {
                // A top-level revert can discard logs without producing an enclosing call frame
                // callback. Preserve the attempted count for subsequent log indices.
                collector.logs.clear();
            }
            (
                collector.logs.into_iter().map(|log| (log.index, log.log)).collect(),
                collector.next_index,
            )
        })
    }

    /// Configures the `Tracer` [`evm2::Inspector`] with a trace printer
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

    let traces = SparsedTraceArena {
        arena,
        ignored: Default::default(),
        diagnostics: Default::default(),
        debug_steps: None,
    };
    let trace = render_trace_arena_inner(&traces, false, true);
    node_info!(Traces = %format!("\n{}", trace));
}

impl<T: EvmTypes> Inspector<T> for AnvilInspector {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(interp.host().logs());
        }
        if let Some(tracer) = &mut self.tracer {
            Inspector::<T>::initialize_interp(tracer, interp);
        }
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(interp.host().logs());
        }
        if let Some(tracer) = self.tracer.as_mut().filter(|_| self.tracer_depth > 0) {
            Inspector::<T>::step(tracer, interp);
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if let Some(tracer) = self.tracer.as_mut().filter(|_| self.tracer_depth > 0) {
            Inspector::<T>::step_end(tracer, interp);
        }
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(interp.host().logs());
        }
    }

    fn log(&mut self, log: &Log, host: &mut T::Host<'_>) {
        if let Some(tracer) = self.tracer.as_mut().filter(|_| self.tracer_depth > 0) {
            Inspector::<T>::log(tracer, log, host);
        }
        if let Some(collector) = &mut self.log_collector {
            collector.logs.push(log.clone());
        }
        if let Some(collector) = &mut self.simulation_logs {
            // The hook runs before the log is recorded.
            collector.push_canonical_log(log.clone(), host.logs().len() + 1);
        }
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(interp.host().logs());
            collector.frame_start();
            // CALLCODE keeps the value in the calling account, so it is not a transfer.
            if message.kind == MessageKind::Call {
                collector.push_transfer(message.caller, message.destination, message.value);
            }
        }
        if let Some(transfer) = &mut self.transfer
            && let Some(result) = Inspector::<T>::call(transfer, interp, message)
        {
            return Some(result);
        }
        if let Some(tracer) = &mut self.tracer {
            self.tracer_depth += 1;
            if let Some(result) = Inspector::<T>::call(tracer, interp, message) {
                return Some(result);
            }
        }
        if let Some(collector) = &mut self.log_collector
            && message.destination == HARDHAT_CONSOLE_ADDRESS
            && let Err(err) = collector.hardhat_log(&message.input)
        {
            return Some(MessageResult::<T> {
                stop: InstrStop::Revert,
                gas: GasTracker::new_with_execution_gas_and_reservoir(
                    message.gas_limit,
                    message.reservoir,
                ),
                output: err.abi_encode_revert(),
                ..Default::default()
            });
        }
        None
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        if let Some(tracer) = &mut self.tracer {
            self.tracer_depth = self.tracer_depth.saturating_sub(1);
            Inspector::<T>::call_end(tracer, interp, message, result);
        }
        if let Some(collector) = &mut self.simulation_logs {
            let logs = interp.host().logs();
            collector.sync_journal_logs(logs);
            collector.frame_end(result.is_success(), logs.len());
        }
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        if let Some(collector) = &mut self.simulation_logs {
            collector.sync_journal_logs(interp.host().logs());
            collector.frame_start();
            collector.push_transfer(message.caller, message.destination, message.value);
        }
        if let Some(transfer) = &mut self.transfer
            && let Some(result) = Inspector::<T>::create(transfer, interp, message)
        {
            return Some(result);
        }
        if let Some(tracer) = &mut self.tracer {
            self.tracer_depth += 1;
            return Inspector::<T>::create(tracer, interp, message);
        }
        None
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        if let Some(tracer) = &mut self.tracer {
            self.tracer_depth = self.tracer_depth.saturating_sub(1);
            Inspector::<T>::create_end(tracer, interp, message, result);
        }
        if let Some(collector) = &mut self.simulation_logs {
            let logs = interp.host().logs();
            collector.sync_journal_logs(logs);
            collector
                .frame_end(result.is_success() && result.created_address.is_some(), logs.len());
        }
    }

    fn selfdestruct(
        &mut self,
        contract: &Address,
        target: &Address,
        value: &U256,
        host: &mut T::Host<'_>,
    ) {
        if let Some(tracer) = &mut self.tracer {
            Inspector::<T>::selfdestruct(tracer, contract, target, value, host);
        }
        if let Some(transfer) = &mut self.transfer {
            Inspector::<T>::selfdestruct(transfer, contract, target, value, host);
        }
        if let Some(collector) = &mut self.simulation_logs {
            collector.push_transfer(*contract, *target, *value);
        }
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        tracing::info!(target: crate::logging::EVM_CONSOLE_LOG_TARGET, "{}", log);
    }
}

/// An inspector shared between an EVM and its owner.
///
/// evm2 owns its inspector for the EVM's lifetime; sharing lets a block executor drain the
/// per-transaction traces between transactions.
#[derive(Debug, Default)]
pub struct SharedInspector<I>(Rc<RefCell<I>>);

impl<I> Clone for SharedInspector<I> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<I> SharedInspector<I> {
    /// Shares `inspector`.
    pub fn new(inspector: I) -> Self {
        Self(Rc::new(RefCell::new(inspector)))
    }

    /// Borrows the inspector mutably.
    pub fn borrow_mut(&self) -> RefMut<'_, I> {
        self.0.borrow_mut()
    }

    /// Returns the inspector, cloning it if it is still shared.
    pub fn into_inner(self) -> I
    where
        I: Clone,
    {
        Rc::try_unwrap(self.0).map(RefCell::into_inner).unwrap_or_else(|rc| rc.borrow().clone())
    }
}

impl<T: EvmTypes, I: Inspector<T>> Inspector<T> for SharedInspector<I> {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.borrow_mut().initialize_interp(interp);
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.borrow_mut().step(interp);
    }

    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.0.borrow_mut().step_end(interp);
    }

    fn log(&mut self, log: &Log, host: &mut T::Host<'_>) {
        self.0.borrow_mut().log(log, host);
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.0.borrow_mut().call(interp, message)
    }

    fn call_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        self.0.borrow_mut().call_end(interp, message, result);
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.0.borrow_mut().create(interp, message)
    }

    fn create_end(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        self.0.borrow_mut().create_end(interp, message, result);
    }

    fn selfdestruct(
        &mut self,
        contract: &Address,
        target: &Address,
        value: &U256,
        host: &mut T::Host<'_>,
    ) {
        self.0.borrow_mut().selfdestruct(contract, target, value, host);
    }
}
