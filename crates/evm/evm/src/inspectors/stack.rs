use super::{
    Cheatcodes, CheatsConfig, ChiselState, CmpOperands, CustomPrintTracer, EdgeCovConfig,
    EdgeCovInspector, EdgeCoverage, Fuzzer, LineCoverageCollector, LogCollector, RevertDiagnostic,
    ScriptExecutionInspector, TempoLabels, TracingInspector,
};
use alloy_consensus::transaction::Recovered;
use alloy_network::TransactionBuilder;
use alloy_primitives::{
    Address, B256, Bytes, KECCAK256_EMPTY, Log, TxKind, U256, keccak256,
    map::{AddressHashMap, AddressSet},
};
use evm2::{
    EvmFeatures, ExecutionError, Inspector,
    bytecode::Bytecode,
    ethereum::intrinsic_and_floor_gas,
    evm::{DbResult, JournalEntry, PendingState, State},
    interpreter::{GasTracker, Host, InstrStop, MessageKind, op},
};
use foundry_cheatcodes::{
    CheatcodeAnalysis, CheatcodesBackend, CheatcodesExecutor, Wallets, message_result,
};
use foundry_common::{compile::Analysis, sh_warn};
use foundry_config::FuzzCorpusConfig;
use foundry_evm_core::{
    FoundryInspectorExt,
    backend::{Backend, transaction_error},
    constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
    evm::{
        BlockEnvFor, EthEvmNetwork, EvmEnvFor, FoundryEvmExt, FoundryEvmFor, FoundryEvmNetwork,
        InterpreterFor, MessageFor, MessageResultFor, TransactionRequestFor, TxEnvFor, TxFor,
        TxResultFor, TypesFor, evm_env, merge_nested_changes, merge_pending_changes,
        release_inspector,
    },
    precompiles::P256_VERIFY,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_networks::{NetworkConfigs, arbitrum};
use foundry_evm_traces::{SparsedTraceArena, TraceRequirements};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use crate::executors::{EarlyExit, EvmExecutionCancellation};

#[derive(Clone, Debug)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct InspectorStackBuilder<BLOCK: Clone> {
    /// Solar compiler instance, to grant syntactic and semantic analysis capabilities.
    pub analysis: Option<Analysis>,
    /// The block environment.
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: Option<BLOCK>,
    /// The gas price.
    ///
    /// Used in the cheatcode handler to overwrite the gas price separately from the gas price
    /// in the execution environment.
    pub gas_price: Option<u128>,
    /// The cheatcodes config.
    pub cheatcodes: Option<Arc<CheatsConfig>>,
    /// The fuzzer inspector and its state, if it exists.
    pub fuzzer: Option<Fuzzer>,
    /// Whether to enable tracing and revert diagnostics.
    pub trace_requirements: TraceRequirements,
    /// Whether logs should be collected.
    /// - None for no log collection.
    /// - Some(true) for realtime console.log-ing.
    /// - Some(false) for log collection.
    pub logs: Option<bool>,
    /// Whether line coverage info should be collected.
    pub line_coverage: Option<bool>,
    /// Whether to print all opcode traces into the console. Useful for debugging the EVM.
    pub print: Option<bool>,
    /// The chisel state inspector.
    pub chisel_state: Option<usize>,
    /// Whether to enable call isolation.
    /// In isolation mode all top-level calls are executed as a separate transaction in a separate
    /// EVM context, enabling more precise gas accounting and transaction state changes.
    pub enable_isolation: bool,
    /// Configuration retained for Celo precompile support.
    // TODO(celo-execution-owner): Replace this residual with concrete Celo precompile
    // configuration. This is independent of the Monad lifecycle migration.
    pub networks: NetworkConfigs,
    /// Concrete Tempo label inspector selected by the Tempo executor builder.
    tempo_labels: Option<Box<TempoLabels>>,
    /// Explicitly resolved additional cheatcode addresses.
    pub extra_cheatcode_addresses: &'static [Address],
    /// The wallets to set in the cheatcodes context.
    pub wallets: Option<Wallets>,
    /// The CREATE2 deployer address.
    pub create2_deployer: Address,
}

impl<BLOCK: Clone> Default for InspectorStackBuilder<BLOCK> {
    fn default() -> Self {
        Self {
            analysis: None,
            block: None,
            gas_price: None,
            cheatcodes: None,
            fuzzer: None,
            trace_requirements: TraceRequirements::none(),
            logs: None,
            line_coverage: None,
            print: None,
            chisel_state: None,
            enable_isolation: false,
            networks: NetworkConfigs::default(),
            tempo_labels: None,
            extra_cheatcode_addresses: &[],
            wallets: None,
            create2_deployer: Default::default(),
        }
    }
}

impl<BLOCK: Clone> InspectorStackBuilder<BLOCK> {
    /// Create a new inspector stack builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the solar compiler instance that grants syntactic and semantic analysis capabilities
    #[inline]
    pub fn set_analysis(mut self, analysis: Analysis) -> Self {
        self.analysis = Some(analysis);
        self
    }

    /// Set the block environment.
    #[inline]
    pub fn block(mut self, block: BLOCK) -> Self {
        self.block = Some(block);
        self
    }

    /// Set the gas price.
    #[inline]
    pub const fn gas_price(mut self, gas_price: u128) -> Self {
        self.gas_price = Some(gas_price);
        self
    }

    /// Enable cheatcodes with the given config.
    #[inline]
    pub fn cheatcodes(mut self, config: Arc<CheatsConfig>) -> Self {
        self.cheatcodes = Some(config);
        self
    }

    /// Set the wallets.
    #[inline]
    pub fn wallets(mut self, wallets: Wallets) -> Self {
        self.wallets = Some(wallets);
        self
    }

    /// Set the fuzzer inspector.
    #[inline]
    pub fn fuzzer(mut self, fuzzer: Fuzzer) -> Self {
        self.fuzzer = Some(fuzzer);
        self
    }

    /// Set the Chisel inspector.
    #[inline]
    pub const fn chisel_state(mut self, final_pc: usize) -> Self {
        self.chisel_state = Some(final_pc);
        self
    }

    /// Set the log collector, and whether to print the logs directly to stdout.
    #[inline]
    pub const fn logs(mut self, live_logs: bool) -> Self {
        self.logs = Some(live_logs);
        self
    }

    /// Set whether to collect line coverage information.
    #[inline]
    pub const fn line_coverage(mut self, yes: bool) -> Self {
        self.line_coverage = Some(yes);
        self
    }

    /// Set whether to enable the trace printer.
    #[inline]
    pub const fn print(mut self, yes: bool) -> Self {
        self.print = Some(yes);
        self
    }

    /// Set trace data requirements.
    #[inline]
    pub const fn trace_requirements(mut self, requirements: TraceRequirements) -> Self {
        self.trace_requirements = self.trace_requirements.merge(requirements);
        self
    }

    /// Set whether to enable the call isolation.
    /// For description of call isolation, see [`InspectorStack::enable_isolation`].
    #[inline]
    pub const fn enable_isolation(mut self, yes: bool) -> Self {
        self.enable_isolation = yes;
        self
    }

    /// Sets networks when building an inspector stack directly.
    ///
    /// [`ExecutorBuilder::build`](crate::executors::ExecutorBuilder::build) overrides this with its
    /// explicit network configuration so the executor, backend, and inspector remain in sync.
    #[inline]
    pub const fn networks(mut self, networks: NetworkConfigs) -> Self {
        self.networks = networks;
        self
    }

    /// Installs the Tempo label inspector.
    #[inline]
    pub(crate) fn tempo_labels(mut self, inspector: TempoLabels) -> Self {
        self.tempo_labels = Some(Box::new(inspector));
        self
    }

    /// Sets explicitly resolved additional cheatcode addresses.
    #[inline]
    pub const fn extra_cheatcode_addresses(mut self, addresses: &'static [Address]) -> Self {
        self.extra_cheatcode_addresses = addresses;
        self
    }

    #[inline]
    pub const fn create2_deployer(mut self, create2_deployer: Address) -> Self {
        self.create2_deployer = create2_deployer;
        self
    }

    /// Builds the stack of inspectors to use when transacting/committing on the EVM.
    pub fn build<FEN: FoundryEvmNetwork>(self) -> InspectorStack<FEN>
    where
        BLOCK: Into<BlockEnvFor<FEN>>,
    {
        let Self {
            analysis,
            block,
            gas_price,
            cheatcodes,
            fuzzer,
            trace_requirements,
            logs,
            line_coverage,
            print,
            chisel_state,
            enable_isolation,
            networks,
            tempo_labels,
            extra_cheatcode_addresses,
            wallets,
            create2_deployer,
        } = self;
        let mut stack = InspectorStack::new();
        // inspectors
        if let Some(config) = cheatcodes {
            let mut cheatcodes = Cheatcodes::new(config);
            cheatcodes.set_extra_cheatcode_addresses(extra_cheatcode_addresses);
            // Set analysis capabilities if they are provided
            if let Some(analysis) = analysis {
                stack.set_analysis(analysis.clone());
                cheatcodes.core.set_analysis(CheatcodeAnalysis::new(analysis));
            }
            // Set wallets if they are provided
            if let Some(wallets) = wallets {
                cheatcodes.core.set_wallets(wallets);
            }
            stack.set_cheatcodes(cheatcodes);
        }

        if let Some(fuzzer) = fuzzer {
            stack.set_fuzzer(fuzzer);
        }
        if let Some(chisel_state) = chisel_state {
            stack.set_chisel(chisel_state);
        }
        stack.collect_line_coverage(line_coverage.unwrap_or(false));
        stack.collect_logs(logs);
        stack.print(print.unwrap_or(false));
        stack.tracing_requirements(trace_requirements);

        stack.enable_isolation(enable_isolation);
        stack.networks(networks);
        stack.inner.tempo_labels = tempo_labels;
        stack.set_extra_cheatcode_addresses(extra_cheatcode_addresses);
        stack.set_create2_deployer(create2_deployer);

        // environment, must come after all of the inspectors
        if let Some(block) = block {
            stack.set_block(block.into());
        }
        if let Some(gas_price) = gas_price {
            stack.set_gas_price(gas_price);
        }

        stack
    }
}

/// Helper macro to call the same method on multiple inspectors without resorting to dynamic
/// dispatch.
#[macro_export]
macro_rules! call_inspectors {
    ([$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $body:expr $(,)?) => {
        $(
            if let Some($id) = $inspector {
                $crate::utils::cold_path();
                $body;
            }
        )+
    };
    (#[ret] [$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $body:expr $(,)?) => {{
        $(
            if let Some($id) = $inspector {
                $crate::utils::cold_path();
                if let Some(result) = $body {
                    return result;
                }
            }
        )+
    }};
}

/// The collected results of [`InspectorStack`].
pub struct InspectorData<FEN: FoundryEvmNetwork> {
    pub logs: Vec<Log>,
    pub labels: AddressHashMap<String>,
    pub traces: Option<SparsedTraceArena>,
    pub line_coverage: Option<HitMaps>,
    pub edge_coverage: Option<EdgeCoverage>,
    pub evm_cmp_values: Option<Vec<CmpOperands>>,
    pub cheatcodes: Option<Box<Cheatcodes<FEN>>>,
    pub chisel_state: Option<(Vec<U256>, Vec<u8>)>,
    pub reverter: Option<Address>,
}

/// The root frame of a nested transaction that duplicates a frame of the running transaction,
/// such as an isolated call or the transaction of `vm.executeTransaction`.
///
/// The duplicated frame already ran the inspectors' call hooks in the running transaction, so the
/// root frame skips them. Its depth is set to the duplicated frame's, so the frames it starts keep
/// their depth in the surrounding trace.
#[derive(Clone, Copy, Debug)]
struct InnerRoot {
    /// Depth of the duplicated frame.
    depth: u16,
    /// Sender of the nested transaction, or `None` if not known in advance.
    caller: Option<Address>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OpcodeStepDispatch {
    #[default]
    None,
    FuzzerOnly,
    General,
}

/// An inspector that calls multiple inspectors in sequence.
///
/// If a call to an inspector returns a value (indicating a stop or revert) the remaining inspectors
/// are not called.
///
/// Stack is divided into [Cheatcodes] and `InspectorStackInner`. This is done to allow assembling
/// [`InspectorStackRefMut`] inside [Cheatcodes] to allow usage of it as an evm2 [`Inspector`].
/// This gives us ability to create and execute separate EVM frames from inside cheatcodes while
/// still having access to entire stack of inspectors and correctly handling traces, logs,
/// debugging info collection, etc.
///
/// The stack observes an execution through [`InspectorStack::as_inspector`], which pairs it with
/// the backend the execution runs over.
#[derive(Clone, Debug)]
pub struct InspectorStack<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    pub cheatcodes: Option<Box<Cheatcodes<FEN>>>,
    pub inner: InspectorStackInner,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct EarlyExitTestGate {
    entered: std::sync::mpsc::Sender<()>,
    release: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    target_pc: usize,
    notified: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Copy, Debug)]
struct PendingCallTrace {
    trace_idx: usize,
    executed_address: Option<Address>,
}

/// All used inspectors besides [Cheatcodes].
///
/// See [`InspectorStack`].
#[derive(Default, Clone, Debug)]
pub struct InspectorStackInner {
    /// Solar compiler instance, to grant syntactic and semantic analysis capabilities.
    pub analysis: Option<Analysis>,

    // Inspectors.
    // These are boxed to reduce the size of the struct and slightly improve performance of the
    // `if let Some` checks.
    pub chisel_state: Option<Box<ChiselState>>,
    pub edge_coverage: Option<Box<EdgeCovInspector>>,
    pub fuzzer: Option<Box<Fuzzer>>,
    pub line_coverage: Option<Box<LineCoverageCollector>>,
    pub log_collector: Option<Box<LogCollector>>,
    pub printer: Option<Box<CustomPrintTracer>>,
    pub revert_diag: Option<Box<RevertDiagnostic>>,
    pub script_execution_inspector: Option<Box<ScriptExecutionInspector>>,
    pub tempo_labels: Option<Box<TempoLabels>>,
    pub tracer: Option<Box<TracingInspector>>,

    // FoundryInspectorExt and other internal data.
    /// Whether to collect sancov edge coverage from instrumented native crates.
    pub sancov_edges: bool,
    /// Whether to capture sancov trace-cmp operands for dictionary injection.
    pub sancov_trace_cmp: bool,
    pub enable_isolation: bool,
    pub networks: NetworkConfigs,
    /// Additional addresses installed and recognized as cheatcode contracts.
    pub extra_cheatcode_addresses: &'static [Address],
    pub create2_deployer: Address,
    /// Flag marking if we are in a nested transaction duplicating a frame of the running one.
    pub in_inner_context: bool,
    /// Depth of the frame the running nested transaction duplicates.
    inner_root_depth: Option<u16>,
    /// The root frame of the nested transaction about to start.
    pending_inner_root: Option<InnerRoot>,
    /// Synthetic CREATE depth corresponding to an outer top-level deployment.
    synthetic_create_depth: Option<u16>,
    /// Accounts the running top-level transaction created, directly or through isolated calls.
    tx_created: AddressSet,
    /// Address that reverted the call, if any.
    pub reverter: Option<Address>,
    /// LIFO stack of the depths of CREATE frames that were redirected to the CREATE2 factory.
    pending_create2_redirects: Vec<u16>,
    /// LIFO stack tracking the effective address of traced calls delegated to the EVM provider.
    pending_call_traces: Vec<PendingCallTrace>,
    /// Pending CREATE2 deployer validation error, returned after the create hooks of the other
    /// inspectors so the trace stays balanced.
    pending_create2_error: Option<Bytes>,
    /// Counter for CREATE2 salt in `--batch` CREATE rewrites.
    pub batch_create_counter: u64,
    /// Whether the one-shot `--batch` rewrite warning has already been emitted.
    pub batch_rewrite_warned: bool,
    /// Per-inspector random seed mixed into `--batch` CREATE2 salts, ensuring re-runs
    /// at identical on-chain state still produce distinct salts. Lazily initialized.
    pub batch_rewrite_process_salt: Option<u64>,
    /// Shared cancellation state for interruptible EVM execution.
    execution_cancellation: Option<EvmExecutionCancellation>,
    /// Amortizes deadline checks in the opcode hot path.
    cancellation_poll_counter: u8,
    /// Whether this inspector halted the current execution due to cancellation.
    execution_cancelled: bool,
    #[cfg(test)]
    early_exit_test_gate: Option<EarlyExitTestGate>,
    static_step_dispatch: OpcodeStepDispatch,
    has_static_step_end_inspectors: bool,
}

/// Mutable references to both parts of [InspectorStack] and the backend the execution runs over,
/// implementing the evm2 [`Inspector`]. Obtained via [`InspectorStack::as_inspector`].
pub struct InspectorStackRefMut<'a, FEN: FoundryEvmNetwork = EthEvmNetwork> {
    pub cheatcodes: Option<&'a mut Cheatcodes<FEN>>,
    pub inner: &'a mut InspectorStackInner,
    /// The backend the execution runs over.
    ///
    /// Cheatcodes and call isolation are only active with a backend.
    pub backend: Option<&'a mut dyn CheatcodesBackend<FEN>>,
}

impl<FEN: FoundryEvmNetwork> CheatcodesExecutor<FEN> for InspectorStackInner {
    fn exec_message(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        tx_env: &TxEnvFor<FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Result<MessageResultFor<FEN>, ExecutionError> {
        let previous = self.synthetic_create_depth;
        let create2_redirects = self.pending_create2_redirects.len();
        // A `deployCode` create called from the test contract duplicates a top-level deployment,
        // so isolate it as well.
        self.synthetic_create_depth = (message.depth == 2).then_some(2);
        let mut frame = backend.backend().begin_frame_evm(evm);
        frame.set_inspector(InspectorStackRefMut {
            cheatcodes: Some(cheats),
            inner: &mut *self,
            backend: Some(&mut *backend),
        });
        let result = Host::execute_message(&mut frame, tx_env, message);
        Backend::<FEN>::end_frame_evm(evm, frame);
        self.synthetic_create_depth = previous;
        // Drop redirects left behind if nested execution aborted before their frames ended.
        self.pending_create2_redirects.truncate(create2_redirects);
        result
    }

    fn nested_transaction(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let mut child = backend.backend().nested_evm(evm, tx, env);
        let previous = self.enter_inner_context(InnerRoot { depth, caller: Some(tx.signer()) });
        // `ORIGIN` reads the nested transaction's signer, not the origin of an isolated call the
        // transaction runs in.
        let origin_override = cheats.origin_override.take();
        child.set_inspector(InspectorStackRefMut {
            cheatcodes: Some(&mut *cheats),
            inner: &mut *self,
            backend: Some(&mut *backend),
        });
        let result = match child.transact(tx) {
            Ok(executed) => {
                // A failed transaction keeps its nonce increment.
                merge_nested_changes(evm.state_mut(), &executed)?;
                Ok(executed.result().clone())
            }
            Err(err) => Err(transaction_error(err)),
        };
        drop(release_inspector(child));
        cheats.origin_override = origin_override;
        self.exit_inner_context(previous);
        result
    }

    fn transact(
        &mut self,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let previous = self.enter_inner_context(InnerRoot { depth, caller: None });
        let mut inspector =
            InspectorStackRefMut::<FEN> { cheatcodes: None, inner: &mut *self, backend: None };
        let result =
            backend.backend_mut().transact(fork_id, transaction, evm, Some(&mut inspector));
        self.exit_inner_context(previous);
        result
    }

    fn transact_from_tx(
        &mut self,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let previous = self.enter_inner_context(InnerRoot { depth, caller: Some(tx.signer()) });
        let mut inspector =
            InspectorStackRefMut::<FEN> { cheatcodes: None, inner: &mut *self, backend: None };
        let result = backend.backend_mut().transact_from_tx(tx, evm, env, Some(&mut inspector));
        self.exit_inner_context(previous);
        result
    }

    fn console_log(&mut self, msg: &str) {
        if let Some(collector) = &mut self.log_collector {
            collector.console_log(msg);
        }
    }

    fn tracing_inspector(&mut self) -> Option<&mut TracingInspector> {
        self.tracer.as_deref_mut()
    }
}

impl<FEN: FoundryEvmNetwork> Default for InspectorStack<FEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<FEN: FoundryEvmNetwork> InspectorStack<FEN> {
    /// Creates a new inspector stack.
    ///
    /// Note that the stack is empty by default, and you must add inspectors to it.
    /// This is done by calling the `set_*` methods on the stack directly, or by building the stack
    /// with [`InspectorStack`].
    #[inline]
    pub fn new() -> Self {
        Self { cheatcodes: None, inner: InspectorStackInner::default() }
    }

    /// Set the solar compiler instance.
    #[inline]
    pub fn set_analysis(&mut self, analysis: Analysis) {
        self.analysis = Some(analysis);
    }

    /// Set the cancellation state checked during EVM execution.
    #[inline]
    pub(crate) fn set_early_exit(&mut self, early_exit: EarlyExit) {
        self.set_execution_cancellation(EvmExecutionCancellation::early_exit(early_exit));
    }

    /// Set the complete cancellation state checked during EVM execution.
    #[inline]
    pub(crate) fn set_execution_cancellation(&mut self, cancellation: EvmExecutionCancellation) {
        self.execution_cancellation = Some(cancellation);
    }

    /// Returns whether this inspector halted execution due to cancellation.
    #[inline]
    pub(crate) const fn execution_cancelled(&self) -> bool {
        self.inner.execution_cancelled
    }

    #[cfg(test)]
    pub(crate) fn set_early_exit_test_gate(
        &mut self,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
        target_pc: usize,
    ) {
        self.early_exit_test_gate = Some(EarlyExitTestGate {
            entered,
            release: Arc::new(std::sync::Mutex::new(release)),
            target_pc,
            notified: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
    }

    /// Sets the block for the relevant inspectors.
    #[inline]
    pub fn set_block(&mut self, block: BlockEnvFor<FEN>) {
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.block = Some(block);
        }
    }

    /// Sets the gas price for the relevant inspectors.
    #[inline]
    pub fn set_gas_price(&mut self, gas_price: u128) {
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.gas_price = Some(gas_price);
        }
    }

    /// Set the cheatcodes inspector.
    #[inline]
    pub fn set_cheatcodes(&mut self, cheatcodes: Cheatcodes<FEN>) {
        self.cheatcodes = Some(cheatcodes.into());
    }

    /// Set the fuzzer inspector.
    #[inline]
    pub fn set_fuzzer(&mut self, fuzzer: Fuzzer) {
        self.fuzzer = Some(fuzzer.into());
        self.refresh_static_step_dispatch();
    }

    /// Set the Chisel inspector.
    #[inline]
    pub fn set_chisel(&mut self, final_pc: usize) {
        self.chisel_state = Some(ChiselState::new(final_pc).into());
        self.refresh_static_step_end_dispatch();
    }

    /// Set whether to enable the line coverage collector.
    #[inline]
    pub fn collect_line_coverage(&mut self, yes: bool) {
        self.line_coverage = yes.then(Default::default);
        self.refresh_static_step_dispatch();
    }

    /// Set whether to enable the edge coverage collector with default config.
    #[inline]
    pub fn collect_edge_coverage(&mut self, yes: bool) {
        self.edge_coverage = yes.then(|| EdgeCovInspector::default().into());
        self.refresh_static_step_dispatch();
    }

    /// Enable edge coverage with an explicit configuration.
    #[inline]
    pub fn collect_edge_coverage_with_edge_config(&mut self, config: EdgeCovConfig) {
        self.edge_coverage = Some(EdgeCovInspector::with_config(config).into());
        self.refresh_static_step_dispatch();
    }

    /// Configure the edge coverage collector from a [`FuzzCorpusConfig`].
    ///
    /// Derives both the on/off gate and [`EdgeCovConfig`] from `corpus`.
    #[inline]
    pub fn collect_edge_coverage_with_config(&mut self, corpus: &FuzzCorpusConfig) {
        self.edge_coverage = corpus
            .collect_evm_edge_coverage()
            .then(|| EdgeCovInspector::with_config(corpus.into()).into());
        self.refresh_static_step_dispatch();
    }

    /// Set whether to collect EVM comparison operands.
    #[inline]
    pub fn collect_evm_cmp_log(&mut self, yes: bool) {
        if yes {
            self.edge_coverage
                .get_or_insert_with(|| EdgeCovInspector::with_cmp_log_only().into())
                .enable_cmp_log(true);
        } else if let Some(edge_coverage) = &mut self.edge_coverage {
            edge_coverage.enable_cmp_log(false);
        }
        self.refresh_static_step_dispatch();
    }

    /// Set whether to collect sancov edge coverage from instrumented native crates.
    #[inline]
    pub const fn collect_sancov_edges(&mut self, yes: bool) {
        self.inner.sancov_edges = yes;
    }

    /// Set whether to capture sancov trace-cmp operands for dictionary injection.
    #[inline]
    pub const fn collect_sancov_trace_cmp(&mut self, yes: bool) {
        self.inner.sancov_trace_cmp = yes;
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub const fn enable_isolation(&mut self, yes: bool) {
        self.inner.enable_isolation = yes;
    }

    /// Set networks with enabled features.
    #[inline]
    pub const fn networks(&mut self, networks: NetworkConfigs) {
        self.inner.networks = networks;
    }

    /// Returns additional addresses installed and recognized as cheatcode contracts.
    #[inline]
    pub const fn extra_cheatcode_addresses(&self) -> &'static [Address] {
        self.inner.extra_cheatcode_addresses
    }

    /// Sets additional addresses installed and recognized as cheatcode contracts.
    #[inline]
    pub const fn set_extra_cheatcode_addresses(&mut self, addresses: &'static [Address]) {
        self.inner.extra_cheatcode_addresses = addresses;
    }

    /// Set the CREATE2 deployer address.
    #[inline]
    pub fn set_create2_deployer(&mut self, deployer: Address) {
        self.create2_deployer = deployer;
    }

    /// Set whether to enable the log collector.
    /// - None for no log collection.
    /// - Some(true) for realtime console.log-ing.
    /// - Some(false) for log collection.
    #[inline]
    pub fn collect_logs(&mut self, live_logs: Option<bool>) {
        self.log_collector = live_logs.map(|live_logs| {
            Box::new(if live_logs {
                LogCollector::LiveLogs
            } else {
                LogCollector::Capture { logs: Vec::new() }
            })
        });
    }

    /// Set whether to enable the trace printer.
    #[inline]
    pub fn print(&mut self, yes: bool) {
        self.printer = yes.then(Default::default);
        self.refresh_static_opcode_dispatch();
    }

    /// Set trace data requirements.
    #[inline]
    pub fn tracing_requirements(&mut self, requirements: TraceRequirements) {
        let config = requirements.into_config();
        self.revert_diag = config.is_some().then(RevertDiagnostic::default).map(Into::into);

        if let Some(config) = config {
            match &mut self.tracer {
                Some(tracer) => *tracer.config_mut() = config,
                None => self.tracer = Some(Box::new(TracingInspector::new(config))),
            }
        } else {
            self.tracer = None;
        }
        self.refresh_static_opcode_dispatch();
    }

    /// Set whether to enable script execution inspector.
    #[inline]
    pub fn script(&mut self, script_address: Address) {
        self.script_execution_inspector.get_or_insert_with(Default::default).script_address =
            script_address;
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.script_address = Some(script_address);
        }
        self.refresh_static_step_dispatch();
    }

    /// Returns the stack as an evm2 inspector of an execution over `backend`.
    #[inline(always)]
    pub fn as_inspector<'a>(
        &'a mut self,
        backend: &'a mut dyn CheatcodesBackend<FEN>,
    ) -> InspectorStackRefMut<'a, FEN> {
        InspectorStackRefMut {
            cheatcodes: self.cheatcodes.as_deref_mut(),
            inner: &mut self.inner,
            backend: Some(backend),
        }
    }

    /// Collects all the data gathered during inspection into a single struct.
    pub fn collect(self) -> InspectorData<FEN> {
        let Self {
            mut cheatcodes,
            inner:
                InspectorStackInner {
                    chisel_state,
                    line_coverage,
                    edge_coverage,
                    log_collector,
                    tempo_labels,
                    tracer,
                    revert_diag,
                    reverter,
                    ..
                },
        } = self;

        let trace_diagnostics =
            revert_diag.map(|revert_diag| revert_diag.into_diagnostics()).unwrap_or_default();

        let traces = tracer.map(|tracer| tracer.into_traces()).map(|arena| {
            let ignored = cheatcodes
                .as_mut()
                .map(|cheatcodes| {
                    let mut ignored = std::mem::take(&mut cheatcodes.ignored_traces.ignored);

                    // If the last pause call was not resumed, ignore the rest of the trace
                    if let Some(last_pause_call) = cheatcodes.ignored_traces.last_pause_call {
                        ignored.insert(last_pause_call, (arena.nodes().len(), 0));
                    }

                    ignored
                })
                .unwrap_or_default();

            SparsedTraceArena { arena, ignored, diagnostics: trace_diagnostics }
        });

        let (edge_coverage, evm_cmp_values) = edge_coverage
            .map(|edge_coverage| {
                let (hitcount, cmp_values) = edge_coverage.into_parts();
                (Some(hitcount), (!cmp_values.is_empty()).then_some(cmp_values))
            })
            .unwrap_or_default();

        InspectorData {
            logs: log_collector.and_then(|logs| logs.into_captured_logs()).unwrap_or_default(),
            labels: {
                let mut labels =
                    cheatcodes.as_ref().map(|c| c.core.labels.clone()).unwrap_or_default();
                if let Some(tempo_labels) = tempo_labels {
                    labels.extend(tempo_labels.labels);
                }
                labels
            },
            traces,
            line_coverage: line_coverage.map(|line_coverage| line_coverage.finish()),
            edge_coverage,
            evm_cmp_values,
            cheatcodes,
            chisel_state: chisel_state.and_then(|state| state.state),
            reverter,
        }
    }
}

impl<FEN: FoundryEvmNetwork> InspectorStackRefMut<'_, FEN> {
    /// Whether the cheatcodes are active: installed, with a backend to run over.
    #[inline]
    fn cheats_and_backend(
        &mut self,
    ) -> Option<(&mut Cheatcodes<FEN>, &mut dyn CheatcodesBackend<FEN>, &mut InspectorStackInner)>
    {
        match (self.cheatcodes.as_deref_mut(), self.backend.as_deref_mut()) {
            (Some(cheats), Some(backend)) => Some((cheats, backend, &mut *self.inner)),
            _ => None,
        }
    }

    /// Converts the result of a CREATE frame redirected to the CREATE2 factory, if `message` is
    /// one, into the result of the CREATE.
    fn finish_create2_redirect(
        &mut self,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        if message.kind != MessageKind::Call
            || self.inner.pending_create2_redirects.last() != Some(&message.depth)
        {
            return;
        }
        self.inner.pending_create2_redirects.pop();

        if result.stop.is_success() {
            match Address::try_from(result.output.as_ref()) {
                Ok(address) => {
                    result.created_address = Some(address);
                    result.output = Bytes::new();
                }
                Err(_) => {
                    result.stop = InstrStop::Revert;
                    result.output = "invalid CREATE2 factory output".into();
                    result.gas = GasTracker::new_with_execution_gas_and_reservoir(
                        message.gas_limit,
                        message.reservoir,
                    );
                    result.created_address = None;
                }
            }
        } else {
            result.created_address = None;
        }
    }

    fn do_call_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        let storage_hook_active =
            self.cheatcodes.as_deref().is_some_and(Cheatcodes::is_storage_hook_active);
        if !storage_hook_active && let Some(fuzzer) = &mut self.inner.fuzzer {
            Inspector::<TypesFor<FEN>>::call_end(&mut **fuzzer, interp, message, result);
        }

        let stop = result.stop;
        let backend = self.backend.as_deref_mut();
        let cheats = self.cheatcodes.as_deref_mut().filter(|_| backend.is_some());
        macro_rules! changed {
            ($call:expr) => {{
                let previous_output = result.output.clone();
                $call;
                // If the inspector returns a different status or a revert with a non-empty
                // message, we assume it wants to tell us something.
                result.stop != stop
                    || (result.stop == InstrStop::Revert && result.output != previous_output)
            }};
        }
        'inspectors: {
            if let Some(tracer) = &mut self.inner.tracer
                && changed!(Inspector::<TypesFor<FEN>>::call_end(
                    &mut **tracer,
                    interp,
                    message,
                    result
                ))
            {
                break 'inspectors;
            }
            if let (Some(cheats), Some(backend)) = (cheats, backend)
                && changed!(cheats.call_end_hook(interp, message, result, backend))
            {
                break 'inspectors;
            }
            if let Some(printer) = &mut self.inner.printer
                && changed!(Inspector::<TypesFor<FEN>>::call_end(
                    &mut **printer,
                    interp,
                    message,
                    result
                ))
            {
                break 'inspectors;
            }
            if let Some(revert_diag) = &mut self.inner.revert_diag {
                Inspector::<TypesFor<FEN>>::call_end(&mut **revert_diag, interp, message, result);
            }
        }

        // Record first address that reverted the call.
        if stop.is_revert() && self.inner.reverter.is_none() {
            self.inner.reverter = Some(message.destination);
        }
    }

    fn do_create_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        let stop = result.stop;
        if !self.inner.in_inner_context
            && stop.is_success()
            && let Some(address) = result.created_address
        {
            self.inner.tx_created.insert(address);
        }
        let backend = self.backend.as_deref_mut();
        let cheats = self.cheatcodes.as_deref_mut().filter(|_| backend.is_some());
        macro_rules! changed {
            ($call:expr) => {{
                let previous_output = result.output.clone();
                $call;
                result.stop != stop
                    || (result.stop == InstrStop::Revert && result.output != previous_output)
            }};
        }
        'inspectors: {
            if let Some(line_coverage) = &mut self.inner.line_coverage
                && changed!(Inspector::<TypesFor<FEN>>::create_end(
                    &mut **line_coverage,
                    interp,
                    message,
                    result
                ))
            {
                break 'inspectors;
            }
            if let Some(tracer) = &mut self.inner.tracer
                && changed!(Inspector::<TypesFor<FEN>>::create_end(
                    &mut **tracer,
                    interp,
                    message,
                    result
                ))
            {
                break 'inspectors;
            }
            if let (Some(cheats), Some(backend)) = (cheats, backend)
                && changed!(cheats.create_end_hook(interp, message, result, backend))
            {
                break 'inspectors;
            }
            if let Some(printer) = &mut self.inner.printer {
                Inspector::<TypesFor<FEN>>::create_end(&mut **printer, interp, message, result);
            }
        }
    }

    /// Executes `message`, a top-level call or create of the running transaction, as a separate
    /// nested transaction, for precise gas accounting and transaction state changes.
    ///
    /// The nested transaction runs with a zero base fee and without fee charges, with the
    /// message's gas plus the transaction's intrinsic gas. Its state changes are merged into the
    /// running transaction as journaled writes, unless a cheatcode replaced the running state
    /// inside it, such as a state snapshot restoration or a fork switch, in which case the
    /// running transaction adopts the nested EVM's whole state.
    fn transact_inner(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
    ) -> MessageResultFor<FEN> {
        let tx_origin = interp.tx_env().origin;
        let is_create = matches!(message.kind, MessageKind::Create | MessageKind::Create2);
        let kind = if is_create { TxKind::Create } else { TxKind::Call(message.destination) };
        let evm = interp.host();
        let version = *evm.version();

        // Access lists do not exist before Berlin, including in isolated transactions.
        let access_list = self
            .cheatcodes
            .as_deref()
            .filter(|_| evm.spec_id() >= evm2::SpecId::BERLIN)
            .and_then(|cheats| cheats.core.access_list.clone());
        let (al_accounts, al_keys) = access_list.as_ref().map_or((0, 0), |list| {
            (list.len() as u64, list.iter().map(|item| item.storage_keys.len() as u64).sum())
        });
        let (intrinsic, floor) = intrinsic_and_floor_gas(
            &version,
            message.caller,
            kind,
            &message.input,
            al_accounts,
            al_keys,
            message.value,
        );

        // Preserve the frame's regular gas and reservoir across the synthetic transaction
        // boundary. evm2 rejects transactions whose gas limit is below the EIP-7623 calldata
        // floor, so the limit is raised to the floor; the floor is charged to the frame below.
        let mut env = evm_env(evm);
        let outer_env = env;
        let regular_limit = message.gas_limit.saturating_add(intrinsic);
        let mut gas_limit = regular_limit;
        if version.feature(EvmFeatures::EIP8037) {
            env.version.tx_gas_limit_cap = regular_limit;
            gas_limit = gas_limit.saturating_add(message.reservoir);
        }
        gas_limit = gas_limit.max(floor);
        if version.feature(EvmFeatures::BLOCK_GAS_LIMIT_CHECK) {
            gas_limit = gas_limit.min(env.block.gas_limit.saturating_to());
        }
        let outer_basefee = env.block.basefee;
        env.block.basefee = U256::ZERO;
        env.set_feature(EvmFeatures::FEE_CHARGE, false);

        let nonce = evm
            .state_mut()
            .account_info_untracked(&message.caller)
            .ok()
            .flatten()
            .map_or(0, |account| account.nonce);
        let mut request = TransactionRequestFor::<FEN>::default()
            .with_from(message.caller)
            .with_kind(kind)
            .with_input(message.input.clone())
            .with_value(message.value)
            .with_nonce(nonce)
            .with_chain_id(env.chain_id())
            .with_gas_limit(gas_limit);
        if evm.spec_id() < evm2::SpecId::LONDON {
            request.set_gas_price(0);
        } else {
            request.set_max_fee_per_gas(0);
            request.set_max_priority_fee_per_gas(0);
        }
        if let Some(access_list) = access_list {
            request.set_access_list(access_list);
        }
        let not_executed = |output: Bytes| message_result(message, InstrStop::Revert, output);
        let tx = match FEN::tx_from_request(request) {
            Ok(tx) => tx,
            Err(err) => return not_executed(err.to_string().into_bytes().into()),
        };

        let Some(backend) = self.backend.as_deref_mut() else {
            return not_executed(Bytes::new());
        };
        let source_fork = backend.backend().active_fork_id();
        let epoch = self.cheatcodes.as_deref().map(|cheats| cheats.state_epoch);
        let mut child = backend.backend().nested_evm(evm, &tx, Some(env));
        // The nested transaction continues the running one's network extension state, such as
        // Monad's reserve balance context.
        *child.ext_mut() = evm.ext().clone();
        // Accounts the running transaction created stay created in the nested one, so that
        // EIP-6780 SELFDESTRUCT still deletes them.
        let features = child.version().features;
        for address in &self.inner.tx_created {
            if mark_created(child.state_mut(), address, features).is_err() {
                return not_executed(Bytes::new());
            }
        }

        // Cheatcodes route environment changes through their opcode overrides while isolated,
        // and the root frame reads the running transaction's origin.
        let previous_origin = self.cheatcodes.as_deref_mut().map(|cheats| {
            let previous = cheats.origin_override;
            let origin = previous.unwrap_or(tx_origin);
            cheats.origin_override = (origin != message.caller).then_some(origin);
            cheats.in_isolation_context = true;
            cheats.isolation_env = Some(outer_env);
            previous
        });
        let previous_inner = self
            .inner
            .enter_inner_context(InnerRoot { depth: message.depth, caller: Some(message.caller) });
        let create2_redirects = self.inner.pending_create2_redirects.len();

        child.set_inspector(InspectorStackRefMut {
            cheatcodes: self.cheatcodes.as_deref_mut(),
            inner: &mut *self.inner,
            backend: Some(&mut *backend),
        });
        let executed = child.transact(&tx).ok().map(|executed| executed.detach());
        let mut child = release_inspector(child);

        self.inner.exit_inner_context(previous_inner);
        self.inner.pending_create2_redirects.truncate(create2_redirects);
        if let (Some(cheats), Some(previous)) = (self.cheatcodes.as_deref_mut(), previous_origin) {
            cheats.origin_override = previous;
            cheats.in_isolation_context = false;
            cheats.isolation_env = None;
        }

        let Some(executed) = executed else {
            // Should we match, encode and propagate error as a revert reason?
            return not_executed(Bytes::new());
        };
        let result = executed.result;
        self.inner.tx_created.extend(
            executed
                .pending_state
                .account_changes()
                .filter(|change| change.created && change.current.is_some())
                .map(|change| change.address),
        );

        let replaced = epoch != self.cheatcodes.as_deref().map(|cheats| cheats.state_epoch)
            || self.backend.as_deref().is_some_and(|b| b.backend().active_fork_id() != source_fork);
        let merged = if replaced {
            child.commit_source(&executed.pending_state);
            Backend::<FEN>::end_frame_evm(evm, child);
            true
        } else {
            let merged = merge_pending_changes(evm.state_mut(), &executed.pending_state).is_ok()
                && (!result.status
                    || warm_accessed(evm.state_mut(), &executed.pending_state).is_ok());
            // Keep cheatcode block changes and network extension state from the nested EVM.
            evm.set_block(*child.block());
            evm.ext_mut().replace_while_executing(child.ext().clone());
            merged
        };
        let mut block = *evm.block();
        block.basefee = outer_basefee;
        evm.set_block(block);

        if let Some(cheats) = self.cheatcodes.as_deref_mut() {
            cheats.gas_metering.set_isolated_snapshot_gas_used(result.tx_gas_used(), 0);
        }

        // The transaction also pays its intrinsic gas and calldata floor, which can exceed the
        // gas forwarded to the frame. The frame then consumes its whole regular budget, as on an
        // exceptional halt.
        let mut gas =
            GasTracker::new_with_execution_gas_and_reservoir(message.gas_limit, message.reservoir);
        if gas.spend_state(result.state_gas_spent()).is_err()
            || gas.spend(result.execution_gas_spent()).is_err()
        {
            gas.spend_all();
        }
        let mut stop = result.stop;
        if !merged {
            stop = InstrStop::Revert;
        } else if result.status {
            gas.record_refund(result.refunded as i64);
        }

        // Pre-computed address for `expectRevert` reverter checks on reverted creates.
        let created_address = result
            .created_address
            .or_else(|| (is_create && stop.is_revert()).then_some(message.destination));
        let mut outcome = message_result::<_, _>(message, stop, result.output);
        outcome.gas = gas;
        outcome.created_address = created_address;
        outcome
    }

    /// Invoked at the beginning of a new top-level (0 depth) frame.
    fn top_level_frame_start(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        warm_arbitrum_system_contract::<FEN>(interp);
        self.inner.tx_created.clear();
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.clear_storage_hook_mapping_slots();
        }
    }

    /// Handles the root frame of a nested transaction, returning whether `message` is one.
    fn start_inner_root(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> bool {
        let Some(root) = self.inner.pending_inner_root.take_if(|root| {
            message.depth == 0 && root.caller.is_none_or(|caller| caller == message.caller)
        }) else {
            return false;
        };
        message.depth = root.depth;
        warm_arbitrum_system_contract::<FEN>(interp);
        if matches!(message.kind, MessageKind::Create | MessageKind::Create2) {
            // The transaction already bumped the creator's nonce, and a create below the top
            // level bumps it again.
            if let Ok(mut account) = interp.host().state_mut().account(&message.caller) {
                let nonce = account.nonce();
                account.set_nonce(nonce.saturating_sub(1));
            }
        } else if let Some(cheatcodes) = self.cheatcodes.as_deref() {
            // Transaction messages use the original target, so resolve function mocks here too.
            apply_mocked_function(cheatcodes, interp, message);
        }
        true
    }

    // We take extra care in optimizing `step` and `step_end`, as they're are likely the most
    // hot functions in all of Foundry.

    #[inline(always)]
    fn step_inlined(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let storage_hook_active =
            self.cheatcodes.as_deref().is_some_and(Cheatcodes::is_storage_hook_active);

        #[cfg(test)]
        if interp.opcode() == op::JUMPDEST
            && let Some(gate) = &self.inner.early_exit_test_gate
            && interp.pc() == gate.target_pc
            && !gate.notified.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let _ = gate.entered.send(());
            let _ = gate
                .release
                .lock()
                .expect("early-exit test gate lock poisoned")
                .recv_timeout(std::time::Duration::from_secs(1));
        }

        if let Some(cancellation) = &self.inner.execution_cancellation {
            let poll_deadline = self.inner.cancellation_poll_counter == 0;
            self.inner.cancellation_poll_counter =
                self.inner.cancellation_poll_counter.wrapping_add(1);
            if cancellation.should_stop(poll_deadline) {
                self.inner.execution_cancelled = true;
                interp.set_stop(InstrStop::Stop);
                return;
            }
        }

        let inner = &mut *self.inner;
        match inner.static_step_dispatch {
            OpcodeStepDispatch::None => {}
            OpcodeStepDispatch::FuzzerOnly => {
                if !storage_hook_active && let Some(inspector) = &mut inner.fuzzer {
                    Inspector::<TypesFor<FEN>>::step(&mut **inspector, interp);
                }
            }
            OpcodeStepDispatch::General => {
                if !storage_hook_active {
                    call_inspectors!(
                        [
                            // These are sorted in definition order.
                            &mut inner.edge_coverage,
                            &mut inner.fuzzer,
                            &mut inner.line_coverage,
                        ],
                        |inspector| Inspector::<TypesFor<FEN>>::step(&mut **inspector, interp),
                    );
                }
                call_inspectors!(
                    [
                        // These are sorted in definition order.
                        &mut inner.chisel_state,
                        &mut inner.printer,
                        &mut inner.revert_diag,
                        &mut inner.script_execution_inspector,
                        &mut inner.tracer,
                    ],
                    |inspector| Inspector::<TypesFor<FEN>>::step(&mut **inspector, interp),
                );
            }
        }

        if let Some((cheats, backend, _)) = self.cheats_and_backend()
            && cheats.has_step_hooks()
        {
            let opcode = interp.opcode();
            if !cheats.has_recording_accesses_only_step_hook()
                || matches!(opcode, op::SLOAD | op::SSTORE)
            {
                crate::utils::cold_path();
                cheats.step_hook(interp, backend);
            }
        }
    }

    #[inline(always)]
    fn step_end_inlined(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let inner = &mut *self.inner;
        if inner.has_static_step_end_inspectors {
            call_inspectors!(
                [
                    // These are sorted in definition order.
                    &mut inner.chisel_state,
                    &mut inner.printer,
                    &mut inner.revert_diag,
                    &mut inner.tracer,
                ],
                |inspector| Inspector::<TypesFor<FEN>>::step_end(&mut **inspector, interp),
            );
        }

        if let Some(fuzzer) = &mut inner.fuzzer
            && fuzzer.mapping_slots.is_some()
        {
            Inspector::<TypesFor<FEN>>::step_end(&mut **fuzzer, interp);
        }

        if let Some((cheats, backend, inner)) = self.cheats_and_backend()
            && cheats.has_step_end_hooks()
        {
            crate::utils::cold_path();
            cheats.step_end_hook(interp, backend, inner);
        }
    }

    /// Rewrites a CREATE frame into a call to the CREATE2 factory if the cheatcodes request it.
    ///
    /// Returns whether the frame was rewritten. If the factory is missing or invalid, the error is
    /// returned by the create hook after the other inspectors observed the frame.
    fn redirect_create2(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> bool {
        let depth = usize::from(message.depth);
        if !self
            .cheatcodes
            .as_deref_mut()
            .is_some_and(|cheats| cheats.should_use_create2_factory(depth, message))
        {
            return false;
        }

        // The default factory runtime forwards CALLVALUE into CREATE2, so payable deploys are
        // supported on both the explicit `new C{salt, value}` path and the `--batch` path.
        let salt = match message.kind {
            MessageKind::Create2 => message.salt.into(),
            // --batch: process_salt (random per run) + counter make each deploy unique.
            // The nonce is from the EVM frame caller (script contract), not the EOA.
            MessageKind::Create => {
                if !self.inner.batch_rewrite_warned {
                    let _ = sh_warn!(
                        "--batch rewrites CREATE → CREATE2 via the Arachnid factory; \
                         deployed addresses follow the CREATE2 formula and constructor \
                         msg.sender is the factory, not the EOA."
                    );
                    self.inner.batch_rewrite_warned = true;
                }
                let chain_id = interp.host().version().chain_id;
                let Ok(Some(account)) =
                    interp.host().state_mut().account_info_untracked(&message.caller)
                else {
                    return false;
                };
                self.inner.next_batch_create_salt(chain_id, account.nonce)
            }
            _ => return false,
        };

        let create2_deployer = self.inner.create2_deployer;
        let state = interp.host().state_mut();
        let Ok(mut account) = state.account(&create2_deployer) else { return false };
        let code_hash = account.code_hash();
        if code_hash == alloy_primitives::KECCAK256_EMPTY {
            self.inner.pending_create2_error =
                Some(format!("missing CREATE2 deployer: {create2_deployer}").into_bytes().into());
            return false;
        } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
            self.inner.pending_create2_error = Some("invalid CREATE2 deployer bytecode".into());
            return false;
        }
        let Ok(code) = account.load_code() else { return false };
        account.warm();

        let mut input = Vec::with_capacity(32 + message.input.len());
        input.extend_from_slice(&B256::from(salt).0);
        input.extend_from_slice(&message.input);

        message.kind = MessageKind::Call;
        message.destination = create2_deployer;
        message.call_target = create2_deployer;
        message.code_address = create2_deployer;
        message.code = code;
        message.input = input.into();
        message.salt = B256::ZERO;
        self.inner.pending_create2_redirects.push(message.depth);
        true
    }
}

impl<FEN: FoundryEvmNetwork> Inspector<TypesFor<FEN>> for InspectorStackRefMut<'_, FEN> {
    fn initialize_interp(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let inner = &mut *self.inner;
        call_inspectors!([&mut inner.line_coverage, &mut inner.tracer], |inspector| {
            Inspector::<TypesFor<FEN>>::initialize_interp(&mut **inspector, interp)
        },);
        if let Some(cheats) = self.cheatcodes.as_deref_mut() {
            cheats.initialize_interp_hook(interp);
        }
        let inner = &mut *self.inner;
        call_inspectors!(
            [&mut inner.script_execution_inspector, &mut inner.printer],
            |inspector| Inspector::<TypesFor<FEN>>::initialize_interp(&mut **inspector, interp),
        );
    }

    fn step(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.step_inlined(interp);
    }

    fn step_end(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.step_end_inlined(interp);
    }

    fn log(&mut self, log: &Log, host: &mut FoundryEvmFor<'_, FEN>) {
        let inner = &mut *self.inner;
        // Network post-transaction hooks, such as Tempo's fee settlement, emit logs after the root
        // frame has ended, when the tracer has no call to attach them to.
        let tracer = inner.tracer.as_mut().filter(|tracer| {
            tracer.traces().nodes().first().is_none_or(|root| root.trace.status.is_none())
        });
        call_inspectors!([tracer, &mut inner.log_collector], |inspector| {
            Inspector::<TypesFor<FEN>>::log(&mut **inspector, log, host)
        });
        if let Some(cheats) = self.cheatcodes.as_deref_mut()
            && cheats.has_log_hooks()
        {
            crate::utils::cold_path();
            cheats.log_hook(log);
        }
        call_inspectors!([&mut self.inner.printer], |inspector| {
            Inspector::<TypesFor<FEN>>::log(&mut **inspector, log, host)
        });
    }

    fn call(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        if self.start_inner_root(interp, message) {
            return None;
        }

        let depth = message.depth;
        if depth == 0 && !self.inner.in_inner_context {
            self.top_level_frame_start(interp);
        }

        if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
            revert_diag.frame_start();
        }

        let storage_hook_callback = self
            .cheatcodes
            .as_deref()
            .is_some_and(|cheatcodes| cheatcodes.is_storage_hook_callback(message));
        let storage_hook_active =
            self.cheatcodes.as_deref().is_some_and(Cheatcodes::is_storage_hook_active);

        if !storage_hook_active
            && let Some(fuzzer) = &mut self.inner.fuzzer
            && let Some(output) = Inspector::<TypesFor<FEN>>::call(&mut **fuzzer, interp, message)
        {
            return Some(output);
        }

        if let Some(tracer) = self.inner.tracer.as_deref_mut() {
            crate::utils::cold_path();
            let output = Inspector::<TypesFor<FEN>>::call(tracer, interp, message);
            let trace_idx = tracer.traces().nodes().len() - 1;
            self.inner
                .pending_call_traces
                .push(PendingCallTrace { trace_idx, executed_address: None });
            if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
                revert_diag.set_trace_node(trace_idx);
            }
            if output.is_some() {
                return output;
            }
        }

        let inner = &mut *self.inner;
        call_inspectors!(
            #[ret]
            [
                &mut inner.log_collector,
                &mut inner.printer,
                &mut inner.revert_diag,
                &mut inner.tempo_labels
            ],
            |inspector| Inspector::<TypesFor<FEN>>::call(&mut **inspector, interp, message)
                .map(Some),
        );

        // Storage hook callbacks are instrumentation frames, not user calls. Let evm2 execute the
        // callback after tracing it, but do not apply mocks, pranks, broadcasts, or isolation.
        if storage_hook_callback {
            if let Some(pending) = self.inner.pending_call_traces.last_mut() {
                pending.executed_address = Some(message.code_address);
            }
            return None;
        }

        // The tracer records call inputs before cheatcodes apply caller overrides such as pranks
        // and broadcasts. Keep the trace lifecycle ordering, but remember the node so its caller
        // can be synchronized with the inputs that are actually executed.
        let trace_idx = self.inner.tracer.as_ref().map(|tracer| tracer.traces().nodes().len() - 1);
        // Also isolate a synthetic `deployCode` create that was rewritten to a CREATE2 factory
        // call.
        let isolate = self.inner.enable_isolation
            && !self.inner.in_inner_context
            && self.backend.is_some()
            && (depth == 1 || self.inner.synthetic_create_depth == Some(depth));
        let mut cheatcode_outcome = None;
        if let Some((cheats, backend, inner)) = self.cheats_and_backend() {
            apply_mocked_function(cheats, interp, message);
            cheatcode_outcome = cheats.call_with_executor(
                interp,
                message,
                backend,
                inner,
                isolate && message.kind == MessageKind::Call,
            );
        }

        if let Some(trace_idx) = trace_idx
            && let Some(tracer) = self.inner.tracer.as_deref_mut()
        {
            let caller = match message.kind {
                MessageKind::DelegateCall | MessageKind::CallCode => message.destination,
                _ => message.caller,
            };
            let node = &mut tracer.traces_mut().nodes_mut()[trace_idx];
            node.trace.caller = caller;
        }

        if let Some(output) = cheatcode_outcome {
            return Some(output);
        }

        if let Some(outcome) = handle_arbitrum_system_call::<FEN>(
            interp,
            message,
            self.backend
                .as_deref()
                .and_then(|backend| backend.backend().active_fork_block_number()),
        ) {
            return Some(outcome);
        }

        if let Some(pending) = self.inner.pending_call_traces.last_mut() {
            pending.executed_address = Some(message.code_address);
        }

        if isolate {
            match message.kind {
                // Isolate CALLs.
                MessageKind::Call => return Some(self.transact_inner(interp, message)),
                // Mark accounts and storage cold before STATICCALLs.
                MessageKind::StaticCall => self.cool_for_static_call(interp),
                // Process other variants as usual.
                _ => {}
            }
        }

        None
    }

    fn call_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        // The root frame of a nested transaction duplicates a frame whose hooks run in the
        // surrounding transaction.
        if self.inner.is_inner_context_root(message.depth) {
            return;
        }

        if let Some(pending) = self.inner.pending_call_traces.pop()
            && let Some(tracer) = self.inner.tracer.as_deref_mut()
        {
            let trace = &mut tracer.traces_mut().nodes_mut()[pending.trace_idx].trace;
            if trace.address == P256_VERIFY {
                trace.maybe_precompile = Some(
                    pending.executed_address == Some(P256_VERIFY)
                        && !message.disable_precompiles
                        && interp.host().precompiles().contains(&P256_VERIFY),
                );
            }
        }

        self.do_call_end(interp, message, result);

        if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
            revert_diag.frame_end();
        }
    }

    fn create(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        if self.start_inner_root(interp, message) {
            return None;
        }

        // A CREATE redirected to the CREATE2 factory is observed as a call to the factory.
        if self.redirect_create2(interp, message) {
            return self.call(interp, message);
        }

        let depth = message.depth;
        if depth == 0 && !self.inner.in_inner_context {
            self.top_level_frame_start(interp);
        }

        if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
            revert_diag.frame_start();
        }

        if let Some(tracer) = self.inner.tracer.as_deref_mut() {
            crate::utils::cold_path();
            let output = Inspector::<TypesFor<FEN>>::create(tracer, interp, message);
            let trace_idx = tracer.traces().nodes().len() - 1;
            if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
                revert_diag.set_trace_node(trace_idx);
            }
            if output.is_some() {
                return output;
            }
        }

        if let Some(line_coverage) = &mut self.inner.line_coverage
            && let Some(output) =
                Inspector::<TypesFor<FEN>>::create(&mut **line_coverage, interp, message)
        {
            return Some(output);
        }

        let trace_idx = self.inner.tracer.as_ref().map(|tracer| tracer.traces().nodes().len() - 1);
        let mut cheatcode_outcome = None;
        if let Some((cheats, backend, _)) = self.cheats_and_backend() {
            cheatcode_outcome = cheats.create_hook(interp, message, backend);
        }

        if let Some(trace_idx) = trace_idx
            && let Some(tracer) = self.inner.tracer.as_deref_mut()
        {
            tracer.traces_mut().nodes_mut()[trace_idx].trace.caller = message.caller;
        }

        if let Some(output) = cheatcode_outcome {
            return Some(output);
        }

        // If the CREATE2 deployer is invalid, return the error here, after the other inspectors
        // were notified, so tracing stays balanced.
        if let Some(error) = self.inner.pending_create2_error.take() {
            return Some(message_result(message, InstrStop::Revert, error));
        }

        if message.kind != MessageKind::Create2
            && self.inner.enable_isolation
            && !self.inner.in_inner_context
            && self.backend.is_some()
            && (depth == 1 || self.inner.synthetic_create_depth == Some(depth))
        {
            return Some(self.transact_inner(interp, message));
        }

        None
    }

    fn create_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        // A redirected CREATE ends as the call to the factory, then becomes the CREATE result.
        if message.kind == MessageKind::Call {
            self.call_end(interp, message, result);
            self.finish_create2_redirect(message, result);
            return;
        }

        if self.inner.is_inner_context_root(message.depth) {
            return;
        }

        self.do_create_end(interp, message, result);

        if let Some(revert_diag) = self.inner.revert_diag.as_deref_mut() {
            revert_diag.frame_end();
        }
    }

    fn selfdestruct(
        &mut self,
        contract: &Address,
        target: &Address,
        value: &U256,
        host: &mut FoundryEvmFor<'_, FEN>,
    ) {
        call_inspectors!([&mut self.inner.printer], |inspector| {
            Inspector::<TypesFor<FEN>>::selfdestruct(
                &mut **inspector,
                contract,
                target,
                value,
                host,
            )
        });
    }
}

impl<FEN: FoundryEvmNetwork> FoundryInspectorExt<TypesFor<FEN>> for InspectorStackRefMut<'_, FEN> {
    fn should_use_create2_factory(&mut self, depth: usize, message: &MessageFor<FEN>) -> bool {
        self.cheatcodes
            .as_deref_mut()
            .is_some_and(|cheats| cheats.should_use_create2_factory(depth, message))
    }

    fn console_log(&mut self, msg: &str) {
        if let Some(collector) = &mut self.inner.log_collector {
            collector.console_log(msg);
        }
    }

    fn get_networks(&self) -> NetworkConfigs {
        self.inner.networks
    }

    fn create2_deployer(&self) -> Address {
        self.inner.create2_deployer
    }
}

impl<FEN: FoundryEvmNetwork> InspectorStackRefMut<'_, FEN> {
    /// Cools the running transaction's loaded accounts and storage before an isolated STATICCALL,
    /// as for a separate transaction.
    ///
    /// Storage of accounts with arbitrary storage stays warm, so its generated values are kept.
    fn cool_for_static_call(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let state = interp.host().state_mut();
        let arbitrary = self
            .cheatcodes
            .as_deref()
            .map(|cheats| {
                state
                    .journal()
                    .iter()
                    .filter_map(|entry| match entry {
                        JournalEntry::StorageWarmed { address, key }
                            if cheats.has_arbitrary_storage(address) =>
                        {
                            Some((*address, *key))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        state.cool_loaded_state();
        let addresses = arbitrary.iter().map(|(address, _)| *address).collect::<AddressSet>();
        for address in addresses {
            if let Ok(mut account) = state.account(&address) {
                account.warm();
            }
        }
        for (address, key) in arbitrary {
            state.set_storage_warm(&address, key, true);
        }
    }
}

impl<FEN: FoundryEvmNetwork> Deref for InspectorStack<FEN> {
    type Target = InspectorStackInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<FEN: FoundryEvmNetwork> DerefMut for InspectorStack<FEN> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl InspectorStackInner {
    #[inline]
    const fn refresh_static_opcode_dispatch(&mut self) {
        self.refresh_static_step_dispatch();
        self.refresh_static_step_end_dispatch();
    }

    #[inline]
    const fn refresh_static_step_dispatch(&mut self) {
        self.static_step_dispatch = if self.chisel_state.is_none()
            && self.edge_coverage.is_none()
            && self.line_coverage.is_none()
            && self.printer.is_none()
            && self.revert_diag.is_none()
            && self.script_execution_inspector.is_none()
            && self.tracer.is_none()
        {
            if self.fuzzer.is_some() {
                OpcodeStepDispatch::FuzzerOnly
            } else {
                OpcodeStepDispatch::None
            }
        } else {
            OpcodeStepDispatch::General
        };
    }

    #[inline]
    const fn refresh_static_step_end_dispatch(&mut self) {
        self.has_static_step_end_inspectors = self.chisel_state.is_some()
            || self.printer.is_some()
            || self.revert_diag.is_some()
            || self.tracer.is_some();
    }

    /// Derive the next `--batch` CREATE2 salt and advance the per-batch counter.
    /// The per-inspector random seed is lazily initialized on first use.
    fn next_batch_create_salt(&mut self, chain_id: u64, nonce: u64) -> U256 {
        let process_salt = *self.batch_rewrite_process_salt.get_or_insert_with(rand::random);
        let counter = self.batch_create_counter;
        self.batch_create_counter = counter.wrapping_add(1);
        compute_batch_create_salt(process_salt, chain_id, nonce, counter)
    }

    /// Whether this frame is the root of a nested transaction, duplicating a surrounding frame.
    fn is_inner_context_root(&self, depth: u16) -> bool {
        self.in_inner_context && self.inner_root_depth == Some(depth)
    }

    /// Enters a nested transaction whose root frame duplicates a surrounding frame, returning the
    /// previous context to restore with [`Self::exit_inner_context`].
    const fn enter_inner_context(
        &mut self,
        root: InnerRoot,
    ) -> (bool, Option<u16>, Option<InnerRoot>) {
        let previous = (self.in_inner_context, self.inner_root_depth, self.pending_inner_root);
        self.in_inner_context = true;
        self.inner_root_depth = Some(root.depth);
        self.pending_inner_root = Some(root);
        previous
    }

    /// Restores the context saved by [`Self::enter_inner_context`].
    const fn exit_inner_context(&mut self, previous: (bool, Option<u16>, Option<InnerRoot>)) {
        (self.in_inner_context, self.inner_root_depth, self.pending_inner_root) = previous;
    }
}

/// Derive the CREATE2 salt used by `--batch` CREATE rewrites.
///
/// Mixes a per-inspector random seed, the chain id, the caller nonce, and a per-batch counter
/// so that two simulations launched at the same on-chain state produce distinct salts and
/// do not collide at the Arachnid factory.
fn compute_batch_create_salt(process_salt: u64, chain_id: u64, nonce: u64, counter: u64) -> U256 {
    let mut buf = [0u8; 32];
    buf[0..8].copy_from_slice(&process_salt.to_be_bytes());
    buf[8..16].copy_from_slice(&chain_id.to_be_bytes());
    buf[16..24].copy_from_slice(&nonce.to_be_bytes());
    buf[24..32].copy_from_slice(&counter.to_be_bytes());
    keccak256(buf).into()
}

/// Redirects `message` to the implementation set with `vm.mockFunction`, if one matches.
fn apply_mocked_function<FEN: FoundryEvmNetwork>(
    cheatcodes: &Cheatcodes<FEN>,
    interp: &mut InterpreterFor<'_, '_, FEN>,
    message: &mut MessageFor<FEN>,
) {
    if let Some(mocks) = cheatcodes.core.mocked_functions.get(&message.code_address) {
        // Check if any mock function set for call data or if catch-all mock function set
        // for selector.
        if let Some(target) = mocks
            .get(&message.input)
            .or_else(|| message.input.get(..4).and_then(|selector| mocks.get(selector)))
        {
            let target = *target;
            let code = interp
                .host()
                .state_mut()
                .account(&target)
                .and_then(|mut account| account.load_code())
                .expect("failed to load account");
            // Like the callee address, so cheatcodes such as `expectCall` observe the target.
            message.call_target = target;
            message.code_address = target;
            message.code = code;
        }
    }
}

/// Marks the existing `address` as created in the running transaction of `state`, keeping its
/// account fields.
///
/// evm2 only marks accounts created by `CREATE` and `CREATE2`, so this recreates the account
/// over an empty one and restores its nonce and code.
fn mark_created(state: &mut State<'_>, address: &Address, features: EvmFeatures) -> DbResult<()> {
    let (nonce, code_hash, code) = {
        let mut account = state.account(address)?;
        if account.is_created() || !account.exists() {
            return Ok(());
        }
        let (nonce, code_hash, code) = (account.nonce(), account.code_hash(), account.load_code()?);
        account.set_nonce(0);
        account.set_code(KECCAK256_EMPTY, Bytecode::default());
        (nonce, code_hash, code)
    };
    // The account is empty and the endowment is zero, so the creation cannot fail.
    let _ = state.create_account(address, address, &U256::ZERO, features)?;
    let mut account = state.account(address)?;
    account.set_nonce(nonce);
    account.set_code(code_hash, code);
    Ok(())
}

/// Warms in `state` the accounts a successful nested transaction loaded and the storage slots it
/// accessed, as accesses across an isolated call boundary stay warm for the calling frame.
fn warm_accessed(state: &mut State<'_>, pending: &PendingState) -> DbResult<()> {
    for change in pending.account_changes() {
        state.account(&change.address)?.warm();
        for (key, slot) in &change.storage.slots {
            if slot.is_warm {
                state.storage_slot(&change.address, *key)?.warm();
            }
        }
    }
    Ok(())
}

/// Warms Arbitrum's ArbSys like a precompile, as Nitro does, on Arbitrum chains.
fn warm_arbitrum_system_contract<FEN: FoundryEvmNetwork>(interp: &mut InterpreterFor<'_, '_, FEN>) {
    let evm = interp.host();
    if !arbitrum::is_arbitrum_chain(evm.version().chain_id) {
        return;
    }
    // Other selectors can still execute code at the address.
    evm.state_mut().prewarm(&arbitrum::ARB_SYS_ADDRESS);
}

/// Answers `ArbSys.arbBlockNumber()` on Arbitrum chains.
fn handle_arbitrum_system_call<FEN: FoundryEvmNetwork>(
    interp: &mut InterpreterFor<'_, '_, FEN>,
    message: &MessageFor<FEN>,
    fork_block_number: Option<u64>,
) -> Option<MessageResultFor<FEN>> {
    let evm = interp.host();
    if message.destination != arbitrum::ARB_SYS_ADDRESS
        || message.code_address != arbitrum::ARB_SYS_ADDRESS
        || !arbitrum::is_arbitrum_chain(evm.version().chain_id)
    {
        return None;
    }

    if message.input.get(..4) != Some(&arbitrum::ARB_BLOCK_NUMBER_SELECTOR) {
        return None;
    }

    // Forks report their L2 block, which the block env lacks on Arbitrum because `NUMBER` returns
    // the L1 block there. Without a fork the block env is read here to follow `vm.roll`.
    let block_number = fork_block_number.unwrap_or_else(|| evm.block().number.saturating_to());
    let Some((gas_cost, output)) = arbitrum::arb_block_number_call(message.gas_limit, block_number)
    else {
        let mut result = message_result(message, InstrStop::PrecompileOOG, Bytes::new());
        result.gas.spend_all();
        return Some(result);
    };

    let mut result = message_result(message, InstrStop::Return, output);
    let _ = result.gas.spend(gas_cost);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_dispatch_defaults_to_no_static_inspectors() {
        let stack = InspectorStackInner::default();

        assert_eq!(stack.static_step_dispatch, OpcodeStepDispatch::None);
        assert!(!stack.has_static_step_end_inspectors);
    }

    #[test]
    fn opcode_dispatch_uses_fuzzer_fast_path_when_fuzzer_is_only_static_step_inspector() {
        let mut stack = InspectorStack::<EthEvmNetwork>::new();
        stack.set_fuzzer(Fuzzer::new(16, None));

        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::FuzzerOnly);
        assert!(!stack.inner.has_static_step_end_inspectors);

        stack.collect_line_coverage(true);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::General);

        stack.collect_line_coverage(false);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::FuzzerOnly);
    }

    #[test]
    fn opcode_dispatch_tracks_general_step_and_step_end_inspectors() {
        let mut stack = InspectorStack::<EthEvmNetwork>::new();

        stack.print(true);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::General);
        assert!(stack.inner.has_static_step_end_inspectors);

        stack.print(false);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::None);
        assert!(!stack.inner.has_static_step_end_inspectors);

        stack.tracing_requirements(TraceRequirements::none().with_calls(true));
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::General);
        assert!(stack.inner.has_static_step_end_inspectors);

        stack.tracing_requirements(TraceRequirements::none());
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::None);
        assert!(!stack.inner.has_static_step_end_inspectors);

        stack.set_chisel(0);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::None);
        assert!(stack.inner.has_static_step_end_inspectors);
    }

    #[test]
    fn opcode_dispatch_tracks_script_and_edge_coverage_inspectors() {
        let mut stack = InspectorStack::<EthEvmNetwork>::new();

        stack.script(Address::with_last_byte(1));
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::General);
        assert!(!stack.inner.has_static_step_end_inspectors);

        let mut stack = InspectorStack::<EthEvmNetwork>::new();
        stack.collect_edge_coverage(true);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::General);
        stack.collect_edge_coverage(false);
        assert_eq!(stack.inner.static_step_dispatch, OpcodeStepDispatch::None);
    }

    #[test]
    fn revert_diagnostic_frames_remain_balanced() {
        let mut inspector = RevertDiagnostic::default();
        inspector.frame_start();
        inspector.set_trace_node(0);
        inspector.frame_start();
        inspector.set_trace_node(1);
        inspector.frame_end();
        inspector.frame_end();

        assert!(inspector.into_diagnostics().is_empty());
    }

    #[test]
    fn distinct_salts_across_simulations_at_same_nonce() {
        // Two "simulations" with different per-inspector seeds but identical on-chain state.
        let a = compute_batch_create_salt(0xabcd_ef01_2345_6789, 1, 5, 0);
        let b = compute_batch_create_salt(0x1122_3344_5566_7788, 1, 5, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn counter_changes_salt() {
        let a = compute_batch_create_salt(1, 1, 5, 0);
        let b = compute_batch_create_salt(1, 1, 5, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn chain_id_changes_salt() {
        let a = compute_batch_create_salt(1, 1, 5, 0);
        let b = compute_batch_create_salt(1, 2, 5, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let a = compute_batch_create_salt(42, 1, 5, 7);
        let b = compute_batch_create_salt(42, 1, 5, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_create2_addresses_across_inspector_instances_at_same_onchain_state() {
        // Two fresh inspectors fed identical (chain_id, nonce) must produce CREATE2 addresses
        // that differ when routed through the same factory with the same init code.
        let factory = Address::with_last_byte(0x42);
        let init_code = b"\x60\x80\x60\x40".as_slice();

        let mut a = InspectorStackInner::default();
        let mut b = InspectorStackInner::default();
        let salt_a = a.next_batch_create_salt(1, 5).to_be_bytes::<32>();
        let salt_b = b.next_batch_create_salt(1, 5).to_be_bytes::<32>();

        let addr_a = factory.create2_from_code(salt_a, init_code);
        let addr_b = factory.create2_from_code(salt_b, init_code);
        assert_ne!(addr_a, addr_b);
    }
}
