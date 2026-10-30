use super::{
    Cheatcodes, CheatsConfig, ChiselState, CmpOperands, CustomPrintTracer, DebugRecorder,
    EdgeCovConfig, EdgeCovInspector, EdgeCoverage, Fuzzer, LineCoverageCollector, LogCollector,
    RevertDiagnostic, ScriptExecutionInspector, TempoLabels, TracingInspector,
};
use alloy_consensus::transaction::Recovered;
use alloy_network::TransactionBuilder;
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, keccak256,
    map::{AddressHashMap, AddressMap, AddressSet, HashSet},
};
use evm2::{
    EvmFeatures, ExecutedTx, ExecutionError, Inspector, InspectorConfig, OpcodeSet,
    TxResultWithState,
    ethereum::intrinsic_and_floor_gas,
    evm::{DbResult, PendingState, PrewarmSet, State},
    interpreter::{GasTracker, Host, InstrStop, MessageKind, op},
    registry::HandlerError,
};
use eyre::WrapErr;
use foundry_cheatcodes::{
    CheatcodeAnalysis, CheatcodesBackend, CheatcodesExecutor, Wallets, message_result,
};
use foundry_common::{compile::Analysis, sh_warn};
use foundry_config::FuzzCorpusConfig;
use foundry_evm_core::{
    FoundryInspectorExt,
    backend::transaction_error,
    constants::{CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER_CODEHASH},
    evm::{
        BlockEnvFor, DestructedAccount, EthEvmNetwork, EvmEnvFor, EvmFactoryFor, ExecutedNested,
        FoundryEvmExt, FoundryEvmFactory, FoundryEvmFor, FoundryEvmNetwork, InterpreterFor,
        MessageFor, MessageResultFor, PersistentWrite, PreparedTx, SnapshotRestores,
        TransactionRequestFor, TxFor, TxResultFor, TypesFor, cool_loaded, destructed_accounts,
        evm_env, merge_pending_changes, merge_untracked_pending_changes_with_warmth,
        release_inspector, warm_accounts, warm_set,
    },
    precompiles::P256_VERIFY,
    utils::tx_warm_addresses,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_networks::arbitrum;
use foundry_evm_traces::{CallTrace, SparsedTraceArena, TraceRequirements};
use std::{
    mem,
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

/// Runs an end hook on `$result` and returns whether it changed the frame's outcome from `$stop`:
/// a different status or a revert with a different message.
macro_rules! changed {
    ($result:ident, $stop:ident, $call:expr) => {{
        let previous_output = $result.output.clone();
        $call;
        $result.stop != $stop
            || ($result.stop == InstrStop::Revert && $result.output != previous_output)
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

/// The root frame of a nested transaction that runs within a frame of the running transaction,
/// such as an isolated call, the transaction of `vm.executeTransaction` or a transaction replayed
/// by `vm.transact`.
///
/// A root frame that duplicates the surrounding frame skips the inspectors' call hooks, which the
/// surrounding frame already ran, and starts at its depth, so the frames it starts keep their depth
/// in the surrounding trace. An observed root frame is a new call one level below it.
#[derive(Clone, Copy, Debug)]
struct InnerRoot {
    /// Depth of the surrounding frame.
    depth: u16,
    /// Sender of the nested transaction.
    caller: Address,
    /// Origin the nested transaction reads instead of its sender, such as the running
    /// transaction's origin for an isolated call.
    origin: Option<Address>,
    /// Whether the root frame is a new call observed by the inspectors, such as a replayed
    /// transaction's, rather than a duplicate of the surrounding frame.
    observe: bool,
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
    pub debug_recorder: Option<Box<DebugRecorder>>,
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
    /// Active fork and cheatcode fork epoch when the running isolated call started.
    isolated_source_fork: Option<(Option<U256>, Option<u64>)>,
    /// Warmth of the running isolated call when it succeeded after switching forks, even back to
    /// the one it started on, captured before its transaction ended: its whole warm set, and the
    /// accounts and slots it accessed or fork switches carried, without its transaction's base.
    isolated_switch_warmth: Option<(PrewarmSet, AddressMap<HashSet<U256>>)>,
    /// Whether the frame that just ended ran on a frame EVM with an in-flight environment change,
    /// whose hooks already ran there. See `InspectorStackRefMut::env_handoff`.
    env_handoff_ended: bool,
    /// Accounts the running top-level transaction created, directly or through isolated calls.
    tx_created: AddressSet,
    /// Existing accounts the running `executeTransaction` transaction self-destructed, captured
    /// when its root frame ends, before the transaction destroys them.
    executed_destructs: Option<AddressMap<DestructedAccount>>,
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
        message: &mut MessageFor<FEN>,
    ) -> Result<MessageResultFor<FEN>, ExecutionError> {
        let previous = self.synthetic_create_depth;
        let create2_redirects = self.pending_create2_redirects.len();
        // A `deployCode` create called from the test contract duplicates a top-level deployment,
        // so isolate it as well.
        self.synthetic_create_depth = (message.depth == 2).then_some(2);
        let env = evm_env(evm);
        let result = self.execute_on_frame_evm(Some(cheats), backend, evm, env, message);
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
        let mut env = env.unwrap_or_else(|| evm_env(evm));
        env.version.initial_call_depth = depth;
        EvmFactoryFor::<FEN>::default().sync_transaction(evm);
        let mut child = backend.backend().nested_evm(evm, tx, Some(env));
        let previous = self.enter_inner_context(InnerRoot {
            depth,
            caller: tx.signer(),
            origin: None,
            observe: false,
        });
        // `ORIGIN` reads the nested transaction's signer, not the origin of an isolated call the
        // transaction runs in.
        let origin_override = cheats.origin_override.take();
        // The nested transaction has its own journal.
        let snapshot_restores = mem::take(&mut cheats.snapshot_restores);
        // The nested transaction's persistent writes refer to its own journal, so they are kept
        // apart. Its root frame skips the cheatcode hooks, so they are handled below.
        let outer_writes = mem::take(&mut cheats.persistent_writes);
        let frame_rollback = cheats.frame_rollback.take();
        let outer_destructs = self.executed_destructs.replace(AddressMap::default());
        // The nested transaction's calls do not apply access lists to the running one.
        let applied_access_list = cheats.applied_access_list.take();
        child.set_inspector(InspectorStackRefMut {
            cheatcodes: Some(&mut *cheats),
            inner: &mut *self,
            backend: Some(&mut *backend),
        });
        let executed = child.transact(tx).map(ExecutedTx::detach);
        // SAFETY: The nested EVM runs over the lifecycle's owned databases.
        let mut child = unsafe { release_inspector(child) };
        cheats.applied_access_list = applied_access_list;
        let destructed = mem::replace(&mut self.executed_destructs, outer_destructs);
        let result = match executed {
            Ok(TxResultWithState { result, pending_state, .. }) => {
                // A failed transaction keeps its nonce increment.
                merge_untracked_pending_changes_with_warmth(
                    evm.state_mut(),
                    &mut child,
                    pending_state,
                    destructed.unwrap_or_default(),
                )
                .map(|persistent| (result, persistent))
                .map_err(Into::into)
            }
            Err(err) => Err(transaction_error(err)),
        };
        drop(child);
        // The merged changes include the writes of a successful transaction; a failed one rolled
        // them back, so they are replayed over its merged changes.
        let writes = mem::replace(&mut cheats.persistent_writes, outer_writes);
        cheats.frame_rollback = frame_rollback;
        let result = result.and_then(|(result, mut persistent)| {
            if !result.is_success() {
                let merged = persistent.len();
                persistent.extend(writes.into_iter().map(|(.., write)| write));
                PersistentWrite::apply_all(&persistent[merged..], evm.state_mut())?;
            }
            cheats.record_persistent(usize::from(depth).saturating_sub(1), evm.state(), persistent);
            Ok(result)
        });
        // The nested transaction's changes are not the running transaction's execution.
        EvmFactoryFor::<FEN>::default().refresh_transaction(evm);
        cheats.snapshot_restores = snapshot_restores;
        cheats.origin_override = origin_override;
        self.exit_inner_context(previous);
        result
    }

    fn transact(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let (prepared, mut replay) = backend.backend().prepare_transact(
            fork_id,
            transaction,
            evm,
            depth + 1,
            cheats.isolation_env.as_ref(),
        )?;
        let fork_id = replay.backend().active_fork_id();
        let mut overrides = Default::default();
        cheats.env_overrides.swap(fork_id, &mut overrides);
        // The mined transaction starts with its own block's coinbase and its access list.
        let tx_warm_addresses = mem::replace(
            &mut cheats.tx_warm_addresses,
            tx_warm_addresses(&prepared.env(), prepared.access_list()),
        );
        let executed = self.replay(cheats, &mut replay, evm, depth, prepared, false);
        cheats.tx_warm_addresses = tx_warm_addresses;
        cheats.env_overrides.swap(fork_id, &mut overrides);
        drop(replay);
        let executed = executed.wrap_err("backend: failed committing transaction")?;
        Ok(backend.backend_mut().store_executed(evm, executed))
    }

    fn transact_from_tx(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let mut env = env.unwrap_or_else(|| evm_env(evm));
        env.version.initial_call_depth = depth + 1;
        let (prepared, mut replay) = backend.backend().prepare_nested(tx.clone(), evm, Some(env));
        let mut executed = self.replay(cheats, &mut replay, evm, depth, prepared, true)?;
        drop(replay);
        cheats.record_persistent(
            usize::from(depth).saturating_sub(1),
            evm.state(),
            executed.take_persistent(),
        );
        Ok(backend.backend_mut().store_executed(evm, executed))
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
        if requirements.debug_recording() {
            self.debug_recorder.get_or_insert_with(Default::default);
        } else {
            self.debug_recorder = None;
        }
        self.refresh_static_opcode_dispatch();
    }

    /// Replaces the tracer, such as with one taken before changing the trace requirements.
    /// Revert diagnostics only annotate trace nodes, so they follow the tracer.
    #[inline]
    pub fn set_tracer(&mut self, tracer: Option<Box<TracingInspector>>) {
        if tracer.is_none() {
            self.revert_diag = None;
        } else if self.revert_diag.is_none() {
            self.revert_diag = Some(Default::default());
        }
        self.tracer = tracer;
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
                    mut debug_recorder,
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

            SparsedTraceArena {
                arena,
                ignored,
                diagnostics: trace_diagnostics,
                debug_steps: debug_recorder.as_deref_mut().map(DebugRecorder::finish),
            }
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

    /// Whether a frame at `depth` runs as an isolated transaction: a top-level frame, or a
    /// synthetic `deployCode` create rewritten to a CREATE2 factory call.
    fn isolates(&self, depth: u16) -> bool {
        self.inner.enable_isolation
            && !self.inner.in_inner_context
            && self.backend.is_some()
            && (depth == 1 || self.inner.synthetic_create_depth == Some(depth))
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
        'inspectors: {
            if let Some(tracer) = &mut self.inner.tracer
                && changed!(
                    result,
                    stop,
                    Inspector::<TypesFor<FEN>>::call_end(&mut **tracer, interp, message, result)
                )
            {
                break 'inspectors;
            }
            if let Some(recorder) = &mut self.inner.debug_recorder {
                Inspector::<TypesFor<FEN>>::call_end(&mut **recorder, interp, message, result);
            }
            if let (Some(cheats), Some(backend)) = (cheats, backend)
                && changed!(result, stop, cheats.call_end_hook(interp, message, result, backend))
            {
                break 'inspectors;
            }
            if let Some(printer) = &mut self.inner.printer
                && changed!(
                    result,
                    stop,
                    Inspector::<TypesFor<FEN>>::call_end(&mut **printer, interp, message, result)
                )
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
        'inspectors: {
            if let Some(line_coverage) = &mut self.inner.line_coverage
                && changed!(
                    result,
                    stop,
                    Inspector::<TypesFor<FEN>>::create_end(
                        &mut **line_coverage,
                        interp,
                        message,
                        result
                    )
                )
            {
                break 'inspectors;
            }
            if let Some(tracer) = &mut self.inner.tracer
                && changed!(
                    result,
                    stop,
                    Inspector::<TypesFor<FEN>>::create_end(&mut **tracer, interp, message, result)
                )
            {
                break 'inspectors;
            }
            if let Some(recorder) = &mut self.inner.debug_recorder {
                Inspector::<TypesFor<FEN>>::create_end(&mut **recorder, interp, message, result);
            }
            if let (Some(cheats), Some(backend)) = (cheats, backend)
                && changed!(result, stop, cheats.create_end_hook(interp, message, result, backend))
            {
                break 'inspectors;
            }
            if let Some(printer) = &mut self.inner.printer {
                Inspector::<TypesFor<FEN>>::create_end(&mut **printer, interp, message, result);
            }
        }
    }

    /// Executes `message` on a frame EVM with the spec and chain ID requested while the
    /// transaction is in flight, if they differ from the running EVM's.
    ///
    /// evm2 fixes the spec and chain ID of a running EVM, so a change deferred by an in-flight
    /// operation, such as `vm.chainId` or a fork switch, or requested by `vm.setEvmVersion`,
    /// moves new frames onto a frame EVM built with it while the running frames keep theirs, so the
    /// change applies to the frames started after it. The message's hooks run on the frame EVM.
    ///
    /// Cheatcode calls run on the running EVM: they execute no code, and the cheatcodes that read
    /// or change the environment act on the running EVM and the in-flight request.
    fn env_handoff(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        if message.destination == CHEATCODE_ADDRESS {
            return None;
        }
        let backend = self.backend.as_deref_mut()?;
        let pending =
            backend.backend().lifecycle().pending_env().map(|env| (env.spec, env.chain_id()));
        let requested = self.cheatcodes.as_deref().and_then(|cheats| cheats.execution_evm_version);
        if pending.is_none() && requested.is_none() {
            return None;
        }
        let evm = interp.host();
        let running = evm_env(evm);
        // Same precedence as the next transaction's environment: `vm.setEvmVersion` overrides
        // the pending spec.
        let (pending_spec, chain_id) = pending.unwrap_or((running.spec, running.chain_id()));
        let spec = requested.unwrap_or(pending_spec);
        if spec == running.spec && chain_id == running.chain_id() {
            return None;
        }
        let mut env = EvmFactoryFor::<FEN>::default().env_with_spec(running, spec);
        env.version.chain_id = chain_id;
        let cheatcodes = self.cheatcodes.as_deref_mut();
        let result = self.inner.execute_on_frame_evm(cheatcodes, backend, evm, env, message);
        match result {
            Ok(result) => {
                // evm2 runs the end hooks right after this hook returns a result.
                self.inner.env_handoff_ended = true;
                Some(result)
            }
            Err(error) => {
                interp.fail(error);
                Some(message_result(message, InstrStop::Revert, Bytes::new()))
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
    ///
    /// `precharged_state` is the account-creation state gas the CALL or CREATE opcode that created
    /// `message` already charged. It is credited as far as the nested transaction charges it again
    /// at its top level, which can differ when a prank or broadcast changed the destination.
    fn transact_inner(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        precharged_state: u64,
    ) -> MessageResultFor<FEN> {
        let precharged_state = if precharged_state == 0 {
            0
        } else {
            precharged_state.min(account_creation_state_gas::<FEN>(interp, message))
        };
        let tx_origin = interp.host().tx_env().origin;
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
        // boundary, funding the precharged state gas from the reservoir. Raising the limit to an
        // EIP-7623 calldata floor above the frame's gas would give the frame more gas than it was
        // forwarded, so the nested transaction skips the floor, and the floor is charged to the
        // frame below.
        let mut env = evm_env(evm);
        let outer_env = env;
        let regular_limit = message.gas_limit.saturating_add(intrinsic);
        let mut gas_limit = regular_limit;
        if version.feature(EvmFeatures::EIP8037) {
            env.version.tx_gas_limit_cap = regular_limit;
            gas_limit =
                gas_limit.saturating_add(message.reservoir).saturating_add(precharged_state);
        }
        let skip_floor = floor > regular_limit;
        if skip_floor {
            env.set_feature(EvmFeatures::EIP7623, false);
        }
        if version.feature(EvmFeatures::BLOCK_GAS_LIMIT_CHECK) {
            gas_limit = gas_limit.min(env.block.gas_limit.saturating_to());
        }
        // The root keeps its depth in the surrounding trace, with a fresh call-depth budget.
        env.version.initial_call_depth = message.depth;
        let outer_basefee = env.block.basefee;
        env.block.basefee = U256::ZERO;
        env.set_feature(EvmFeatures::FEE_CHARGE, false);
        // A call without a fee charge skips the sender balance check, so an
        // underfunded value transfer fails in the call and keeps the nonce increment. An
        // underfunded create keeps the check, which leaves the nonce unchanged.
        if !is_create {
            env.set_feature(EvmFeatures::BALANCE_CHECK, false);
            env.set_feature(EvmFeatures::BALANCE_TOP_UP, false);
        }

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
        if let Some(access_list) = access_list.clone() {
            request.set_access_list(access_list);
        }
        // A transaction that did not execute leaves the frame's reservoir untouched.
        let not_executed = |output: Bytes| {
            let mut outcome = message_result(message, InstrStop::Revert, output);
            outcome.gas = GasTracker::new_with_execution_gas_and_reservoir(
                message.gas_limit,
                message.reservoir,
            );
            outcome
        };
        let tx = match FEN::tx_from_request(request) {
            Ok(tx) => tx,
            Err(err) => return not_executed(err.to_string().into_bytes().into()),
        };

        let Some(backend) = self.backend.as_deref_mut() else {
            return not_executed(Bytes::new());
        };
        let source_fork = backend.backend().active_fork_id();
        let epoch = self.cheatcodes.as_deref().map(|cheats| cheats.state_epoch);
        let fork_epoch = self.cheatcodes.as_deref().map(|cheats| cheats.fork_epoch);
        EvmFactoryFor::<FEN>::default().sync_transaction(evm);
        let mut child = backend.backend().nested_evm(evm, &tx, Some(env));
        // The nested transaction continues the running one's network extension state, such as
        // Monad's reserve balance context and tracker.
        *child.ext_mut() = evm.ext().clone();
        child.ext_mut().set_preserve_transaction_state(true);
        // Accounts the running transaction created stay created in the nested one, so that
        // EIP-6780 SELFDESTRUCT still deletes them, and keep the storage it holds for them.
        for address in &self.inner.tx_created {
            match child.state_mut().account(address) {
                Ok(mut account) => {
                    if account.exists() {
                        account.mark_created_keeping_storage();
                    }
                }
                Err(_) => return not_executed(Bytes::new()),
            }
        }

        // The nested transaction reads the running transaction's origin, cheatcodes route
        // environment changes through their opcode overrides while isolated, and fork revert
        // diagnostics compare against the nested transaction's target.
        let origin = self
            .cheatcodes
            .as_deref()
            .and_then(|cheats| cheats.origin_override)
            .unwrap_or(tx_origin);
        let previous_cheats = self.cheatcodes.as_deref_mut().map(|cheats| {
            let previous = cheats.origin_override.take();
            cheats.in_isolation_context = true;
            cheats.isolation_env = Some(outer_env);
            let root_call_target = mem::replace(
                &mut cheats.root_call_target,
                (!is_create).then_some(message.destination),
            );
            let tx_warm_addresses = mem::replace(
                &mut cheats.tx_warm_addresses,
                tx_warm_addresses(&env, access_list.as_ref()),
            );
            (previous, root_call_target, tx_warm_addresses)
        });
        let previous_inner = self.inner.enter_inner_context(InnerRoot {
            depth: message.depth,
            caller: message.caller,
            origin: (origin != message.caller).then_some(origin),
            observe: false,
        });
        let create2_redirects = self.inner.pending_create2_redirects.len();
        // The nested transaction has its own journal.
        let snapshot_restores =
            self.cheatcodes.as_deref_mut().map(|cheats| mem::take(&mut cheats.snapshot_restores));

        let previous_source_fork =
            self.inner.isolated_source_fork.replace((source_fork, fork_epoch));
        // Taking warmth needs a mutable backend, which clones a borrowed one, so only take it
        // when there is any.
        let persistent_warmth = if backend.backend().lifecycle().has_persistent_warmth() {
            backend.backend_mut().take_persistent_warmth()
        } else {
            Vec::new()
        };
        child.set_inspector(InspectorStackRefMut {
            cheatcodes: self.cheatcodes.as_deref_mut(),
            inner: &mut *self.inner,
            backend: Some(&mut *backend),
        });
        let executed = child.transact(&tx).map(|executed| executed.detach());
        // SAFETY: The nested EVM runs over the lifecycle's owned databases.
        let mut child = unsafe { release_inspector(child) };
        self.inner.isolated_source_fork = previous_source_fork;
        if !persistent_warmth.is_empty() {
            backend.backend_mut().restore_persistent_warmth(persistent_warmth);
        }
        let switch_warmth = self.inner.isolated_switch_warmth.take();

        self.inner.exit_inner_context(previous_inner);
        self.inner.pending_create2_redirects.truncate(create2_redirects);
        // A snapshot restoration that survived the nested transaction replaced its state.
        let restored = self.cheatcodes.as_deref_mut().zip(snapshot_restores).is_some_and(
            |(cheats, restores)| mem::replace(&mut cheats.snapshot_restores, restores).kept(),
        );
        if let (Some(cheats), Some((previous, root_call_target, tx_warm_addresses))) =
            (self.cheatcodes.as_deref_mut(), previous_cheats)
        {
            cheats.tx_warm_addresses = tx_warm_addresses;
            cheats.origin_override = previous;
            cheats.in_isolation_context = false;
            cheats.isolation_env = None;
            cheats.root_call_target = root_call_target;
        }

        let executed = match executed {
            Ok(executed) => executed,
            // An underfunded create fails in its frame, without a nonce increment,
            // after its transaction paid the intrinsic gas.
            Err(HandlerError::InsufficientFunds { .. }) if is_create => {
                let mut outcome = not_executed(Bytes::new());
                if outcome.gas.spend(intrinsic.max(floor)).is_err() {
                    outcome.gas.spend_all();
                }
                return outcome;
            }
            // Should we match, encode and propagate error as a revert reason?
            Err(_) => return not_executed(Bytes::new()),
        };
        let mut result = executed.result;
        if skip_floor {
            result.floor_gas = floor;
        }
        self.inner.tx_created.extend(
            executed
                .pending_state
                .account_changes()
                .filter(|change| change.created && change.current.is_some())
                .map(|change| change.address),
        );

        let fork_changed = backend.backend().active_fork_id() != source_fork;
        let replaced = restored
            || fork_changed
            || epoch != self.cheatcodes.as_deref().map(|cheats| cheats.state_epoch);
        // Accounts the running transaction holds cold, such as after `vm.cool` or a static call
        // cooled them, stay cold. Accounts it no longer holds, such as after a snapshot revert,
        // are imported warm.
        let state = evm.state_mut();
        let loaded = state.loaded_accounts().map(|(address, _)| *address).collect::<Vec<_>>();
        let cold = loaded
            .into_iter()
            .filter(|address| !state.account(address).is_ok_and(|account| account.is_warm()))
            .collect::<AddressSet>();
        let merged = if replaced {
            // Across another state replacement the running transaction keeps its warmth, and the
            // nested transaction's on success. As on master, a restored snapshot keeps the
            // running transaction's base warmth, a fork switch brings the nested transaction's
            // warmth, and a switch back to the starting fork keeps both.
            let warmth = if restored {
                Some(evm.state().prewarm_set().clone())
            } else if fork_changed {
                switch_warmth.map(|(warmth, _)| warmth)
            } else if let Some((_, accessed)) = switch_warmth {
                let mut warmth = warm_set(evm.state_mut());
                for (address, slots) in accessed {
                    warmth.warm_storage(&address, slots);
                }
                Some(warmth)
            } else {
                Some(warm_set(evm.state_mut()))
            };
            child.commit_source(&executed.pending_state);
            // SAFETY: The nested EVM runs over the lifecycle's owned databases.
            unsafe { backend.backend().end_frame_evm(evm, child) };
            if let Some(warmth) = warmth {
                *evm.state_mut().prewarm_set_mut() = warmth;
            } else if fork_changed && !restored {
                // Persistent slots the call had warmed when it switched forks stay warm after it
                // fails, without their accounts.
                let slots = backend.backend().lifecycle().switch_slots().clone();
                for (address, keys) in slots {
                    for key in keys {
                        let _ =
                            evm.state_mut().storage_slot(&address, key).map(|mut slot| slot.warm());
                    }
                }
            }
            // Unless the call switched forks, the running transaction's state stays in place: it
            // keeps the warmth it had, except for accounts it holds cold.
            let keeps_cold = restored || !fork_changed;
            if keeps_cold {
                for address in &cold {
                    evm.state_mut().set_account_warm(address, false);
                }
            }
            let cold = if keeps_cold { cold } else { AddressSet::default() };
            !result.status || warm_accessed(evm.state_mut(), &executed.pending_state, &cold).is_ok()
        } else {
            let merged = merge_pending_changes(evm.state_mut(), &executed.pending_state)
                .and_then(|()| {
                    if result.status {
                        warm_accessed(evm.state_mut(), &executed.pending_state, &cold)
                    } else {
                        warm_surviving(
                            evm.state_mut(),
                            child.state_mut(),
                            &executed.pending_state,
                            &cold,
                        )
                    }
                })
                .is_ok();
            // Keep cheatcode block changes and network extension state from the nested EVM.
            evm.set_block(*child.block());
            evm.ext_mut().replace_while_executing(child.ext().clone());
            merged
        };
        // The running transaction continues from the nested one's derived state, rebased onto
        // the merged state.
        evm.ext_mut().set_preserve_transaction_state(false);
        EvmFactoryFor::<FEN>::default().refresh_transaction(evm);
        let mut block = *evm.block();
        block.basefee = outer_basefee;
        evm.set_block(block);

        // A failed transaction refunds its account-creation state gas, as the outer opcode does.
        let precharged_state = precharged_state.min(result.state_gas_spent());
        if let Some(cheats) = self.cheatcodes.as_deref_mut() {
            cheats
                .gas_metering
                .set_isolated_snapshot_gas_used(result.tx_gas_used(), precharged_state);
        }

        // The transaction also pays its intrinsic gas and calldata floor, which can exceed the
        // gas forwarded to the frame. The frame then consumes its whole regular budget, as on an
        // exceptional halt.
        let mut gas =
            GasTracker::new_with_execution_gas_and_reservoir(message.gas_limit, message.reservoir);
        if gas.spend_state(result.state_gas_spent() - precharged_state).is_err()
            || gas.spend(result.execution_gas_spent()).is_err()
        {
            gas.spend_all();
        }
        let mut stop = result.stop;
        if !merged {
            stop = InstrStop::Revert;
        } else if result.status {
            gas.record_refund(result.final_refunded() as i64);
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
            // Transactions run from the test or script carry no access list.
            cheatcodes.tx_warm_addresses = tx_warm_addresses(&evm_env(interp.host()), None);
            cheatcodes.clear_storage_hook_mapping_slots();
            cheatcodes.snapshot_restores = SnapshotRestores::default();
            // A transaction that ended with an error did not drop its persistent writes.
            cheatcodes.persistent_writes.clear();
            cheatcodes.persistent_write_snapshots.clear();
        }
    }

    /// Captures the accounts an `executeTransaction` transaction self-destructed when its root
    /// frame ends, before the transaction destroys them. See [`destructed_accounts`].
    fn capture_executed_destructs(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        if let Some(destructs) = &mut self.inner.executed_destructs {
            match destructed_accounts(interp.host().state_mut()) {
                Ok(captured) => *destructs = captured,
                Err(err) => {
                    let _ = interp.fail(err);
                }
            }
        }
    }

    /// Captures the warmth of an isolated call whose root frame succeeded on another fork than it
    /// started on, before its transaction ends and clears it.
    fn capture_switch_warmth(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        result: &MessageResultFor<FEN>,
    ) {
        let Some(backend) = self.backend.as_deref() else { return };
        let switched = self.inner.isolated_source_fork.is_some_and(|(source, epoch)| {
            backend.backend().active_fork_id() != source
                || epoch != self.cheatcodes.as_deref().map(|cheats| cheats.fork_epoch)
        });
        self.inner.isolated_switch_warmth = (switched && result.is_success()).then(|| {
            let state = interp.host().state_mut();
            let mut accessed = backend.backend().lifecycle().active().carried_warmth().clone();
            for (address, slots) in warm_accounts(state) {
                accessed.entry(address).or_default().extend(slots);
            }
            (warm_set(state), accessed)
        });
    }

    /// Handles the root frame of a nested transaction, returning whether `message` is one that
    /// skips the inspectors' hooks.
    fn start_inner_root(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> bool {
        let Some(root) = self.inner.pending_inner_root.take_if(|root| {
            message.depth == root.depth + u16::from(root.observe) && root.caller == message.caller
        }) else {
            return false;
        };
        warm_arbitrum_system_contract::<FEN>(interp);
        if let Some(origin) = root.origin {
            interp.host().tx_env_mut().origin = origin;
        }
        if !root.observe
            && !matches!(message.kind, MessageKind::Create | MessageKind::Create2)
            && let Some(cheatcodes) = self.cheatcodes.as_deref()
        {
            // Transaction messages use the original target, so resolve function mocks here too.
            apply_mocked_function(cheatcodes, interp, message);
        }
        !root.observe
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
                        &mut inner.debug_recorder,
                    ],
                    |inspector| Inspector::<TypesFor<FEN>>::step(&mut **inspector, interp),
                );
            }
        }

        if let Some((cheats, backend, _)) = self.cheats_and_backend() {
            if cheats.has_opcode_overrides() {
                cheats.step_opcode_overrides(interp.opcode());
            }
            if cheats.has_step_hooks() {
                let opcode = interp.opcode();
                if !cheats.has_recording_accesses_only_step_hook()
                    || matches!(opcode, op::SLOAD | op::SSTORE)
                {
                    crate::utils::cold_path();
                    cheats.step_hook(interp, backend);
                }
            }
        }

        // evm2 skips `step_end` if an inspector halted the frame in `step`. A recorder makes the
        // dispatch general, so other dispatches skip the check.
        if matches!(self.inner.static_step_dispatch, OpcodeStepDispatch::General)
            && let Some(recorder) = self.inner.debug_recorder.as_deref_mut()
            && interp.result().is_err()
        {
            recorder.end_step_not_run(interp);
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
                    &mut inner.debug_recorder,
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

    /// Closes the snapshot restoration frame `message` ran in, if any, undoing the restorations
    /// made inside it if it failed.
    fn end_snapshot_frame(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &MessageResultFor<FEN>,
    ) {
        if let Some(cheats) = self.cheatcodes.as_deref_mut() {
            let evm = interp.host();
            let features = evm.version().features;
            let failed = !result.stop.is_success();
            let end = cheats.snapshot_restores.frame_end(
                evm.state_mut(),
                message.depth,
                failed,
                features,
            );
            // A frame ends here once per hook layer, and only the layer that opened it returns
            // its checkpoint.
            if let Some((checkpoint, ledger)) = end {
                // The reinstated state holds the persistent writes recorded over it.
                if let Some(ledger) = ledger {
                    cheats.persistent_writes = ledger;
                }
                cheats.frame_rollback =
                    Some((usize::from(message.depth), checkpoint.journal_len()));
            }
        }
    }
}

impl<FEN: FoundryEvmNetwork> Inspector<TypesFor<FEN>> for InspectorStackRefMut<'_, FEN> {
    #[inline]
    fn inspector_config(&self) -> InspectorConfig {
        let inner = &*self.inner;
        #[cfg(test)]
        if inner.early_exit_test_gate.is_some() {
            return InspectorConfig::new();
        }
        let every_opcode = inner.execution_cancellation.is_some()
            || inner.static_step_dispatch != OpcodeStepDispatch::None
            || inner.has_static_step_end_inspectors
            || inner.fuzzer.as_ref().is_some_and(|fuzzer| fuzzer.mapping_slots.is_some())
            || self.cheatcodes.as_deref().is_some_and(|cheats| {
                // A log that fails `expectEmit` reverts its frame in that `LOG`'s `step_end`.
                !cheats.expected_emits.is_empty()
                    || cheats.is_storage_hook_active()
                    || cheats.has_step_hooks()
                    || cheats.has_step_end_hooks()
            });
        let steps = if every_opcode {
            OpcodeSet::ALL
        } else if self.cheatcodes.as_deref().is_some_and(Cheatcodes::has_opcode_overrides) {
            // Origin and gas price overrides only rewrite what these instructions push.
            [op::ORIGIN, op::GASPRICE].into_iter().collect()
        } else {
            OpcodeSet::EMPTY
        };
        InspectorConfig::new().with_steps(steps)
    }

    fn initialize_interp(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let depth = interp.message().depth;
        if let Some(cheats) = self.cheatcodes.as_deref_mut()
            && let Some(checkpoint) = interp.host().frame_checkpoint()
        {
            cheats.snapshot_restores.frame_start(depth, checkpoint.clone());
        }

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
        // Network transaction hooks emit logs outside the root frame, when the tracer has no call
        // to attach them to: Tempo's 2D nonce increment before it starts and its fee settlement
        // after it ends. Only logs of running frames are traced, so they stay out of the trace.
        // Until the root frame starts, the arena's entry node is a placeholder.
        let tracer = inner.tracer.as_mut().filter(|tracer| {
            tracer.traces().nodes().first().is_none_or(|root| {
                root.trace.status.is_none() && root.trace != CallTrace::default()
            })
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
        if let Some(result) = self.env_handoff(interp, message) {
            return Some(result);
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
        if let Some(recorder) = self.inner.debug_recorder.as_deref_mut() {
            Inspector::<TypesFor<FEN>>::call(recorder, interp, message);
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
        let isolate = self.isolates(depth);
        let precharged_state = if isolate && depth == 1 && message.kind == MessageKind::Call {
            account_creation_state_gas::<FEN>(interp, message)
        } else {
            0
        };
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
                MessageKind::Call => {
                    return Some(self.transact_inner(interp, message, precharged_state));
                }
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
        // A frame handed off to a frame EVM already ran its end hooks there.
        if mem::take(&mut self.inner.env_handoff_ended) {
            return;
        }
        self.end_snapshot_frame(interp, message, result);

        // The root frame of a nested transaction duplicates a frame whose hooks run in the
        // surrounding transaction.
        if self.inner.is_inner_context_root(message.depth) {
            self.capture_switch_warmth(interp, result);
            self.capture_executed_destructs(interp);
            return;
        }

        if let Some(pending) = self.inner.pending_call_traces.pop()
            && let Some(tracer) = self.inner.tracer.as_deref_mut()
        {
            let trace = &mut tracer.traces_mut().nodes_mut()[pending.trace_idx].trace;
            if trace.address == P256_VERIFY {
                trace.maybe_precompile = Some(
                    pending.executed_address == Some(P256_VERIFY)
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
        if let Some(result) = self.env_handoff(interp, message) {
            return Some(result);
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
        if let Some(recorder) = self.inner.debug_recorder.as_deref_mut() {
            Inspector::<TypesFor<FEN>>::create(recorder, interp, message);
        }

        if let Some(line_coverage) = &mut self.inner.line_coverage
            && let Some(output) =
                Inspector::<TypesFor<FEN>>::create(&mut **line_coverage, interp, message)
        {
            return Some(output);
        }

        let trace_idx = self.inner.tracer.as_ref().map(|tracer| tracer.traces().nodes().len() - 1);
        // Pranks and broadcasts can change the creator and destination, so read the opcode's
        // charge first. A synthetic `deployCode` create carries the charge of the cheatcode.
        let precharged_state = if self.inner.synthetic_create_depth == Some(depth) {
            self.cheatcodes.as_deref().map_or(0, |cheats| cheats.deploy_code_create_state_gas)
        } else if depth == 1
            && message.kind == MessageKind::Create
            && self.inner.enable_isolation
            && !self.inner.in_inner_context
        {
            account_creation_state_gas::<FEN>(interp, message)
        } else {
            0
        };
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

        if message.kind != MessageKind::Create2 && self.isolates(depth) {
            return Some(self.transact_inner(interp, message, precharged_state));
        }

        None
    }

    fn create_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        // A frame handed off to a frame EVM already ran its end hooks there.
        if mem::take(&mut self.inner.env_handoff_ended) {
            return;
        }
        // A redirected CREATE ends as the call to the factory, then becomes the CREATE result.
        if message.kind == MessageKind::Call {
            self.call_end(interp, message, result);
            self.finish_create2_redirect(message, result);
            return;
        }

        self.end_snapshot_frame(interp, message, result);

        if self.inner.is_inner_context_root(message.depth) {
            self.capture_switch_warmth(interp, result);
            self.capture_executed_destructs(interp);
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
        // The calling frame is the transaction's root, so its address is the recipient.
        let recipient = interp.message().destination;
        let is_precompile = interp.host().precompiles().contains(&recipient);
        let state = interp.host().state_mut();
        // Warmth carried across fork switches cools too.
        if let Some(prewarm) =
            self.backend.as_deref().and_then(|backend| backend.backend().lifecycle().tx_prewarm())
        {
            *state.prewarm_set_mut() = prewarm.clone();
        }
        // The recipient cools like any other account the transaction loaded, unlike the addresses
        // `vm.cool` keeps warm.
        if !is_precompile
            && !self
                .cheatcodes
                .as_deref()
                .is_some_and(|cheats| cheats.tx_warm_addresses.contains(&recipient))
            && let Some(backend) = self.backend.as_deref_mut()
        {
            // Only take a mutable backend, which clones a borrowed one, if the lifecycle tracks
            // the recipient's warmth.
            if backend.backend().lifecycle().tracks_prewarm(&recipient) {
                backend.backend_mut().remove_prewarm(state, &recipient);
            } else {
                state.prewarm_set_mut().remove(&recipient);
            }
        }
        // Accounts with arbitrary storage keep their warm slots, and stay warm if they have any.
        let arbitrary = self
            .cheatcodes
            .as_deref()
            .map(|cheats| {
                let loaded = state
                    .loaded_accounts()
                    .filter(|(address, _)| cheats.has_arbitrary_storage(address))
                    .map(|(address, slots)| (*address, slots.copied().collect::<Vec<_>>()))
                    .collect::<Vec<_>>();
                loaded
                    .into_iter()
                    .filter_map(|(address, mut slots)| {
                        let storage = state.storage(&address).ok()?;
                        slots.retain(|key| storage.is_warm(key));
                        (!slots.is_empty()).then_some((address, slots))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        cool_loaded(state);
        for (address, slots) in arbitrary {
            state.set_account_warm(&address, true);
            for key in slots {
                state.set_storage_warm(&address, key, true);
            }
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
            && self.debug_recorder.is_none()
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
            || self.tracer.is_some()
            || self.debug_recorder.is_some();
    }

    /// Derive the next `--batch` CREATE2 salt and advance the per-batch counter.
    /// The per-inspector random seed is lazily initialized on first use.
    fn next_batch_create_salt(&mut self, chain_id: u64, nonce: u64) -> U256 {
        let process_salt = *self.batch_rewrite_process_salt.get_or_insert_with(rand::random);
        let counter = self.batch_create_counter;
        self.batch_create_counter = counter.wrapping_add(1);
        compute_batch_create_salt(process_salt, chain_id, nonce, counter)
    }

    /// Executes `message` of the transaction `evm` is running on a separate frame EVM with `env`,
    /// observed by the whole stack, such as for `deployCode` or an in-flight spec change.
    fn execute_on_frame_evm<FEN: FoundryEvmNetwork>(
        &mut self,
        cheatcodes: Option<&mut Cheatcodes<FEN>>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        env: EvmEnvFor<FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Result<MessageResultFor<FEN>, ExecutionError> {
        // SAFETY: `evm` runs over the lifecycle's owned databases, and so does the frame EVM,
        // which only lives for this call; its inspector is released before its state moves back.
        let mut frame = unsafe { backend.backend().begin_frame_evm(evm, env) };
        frame.set_inspector(InspectorStackRefMut {
            cheatcodes,
            inner: self,
            backend: Some(backend),
        });
        let result = Host::execute_message(&mut frame, message);
        // SAFETY: See above.
        let frame = unsafe { release_inspector(frame) };
        // SAFETY: See above.
        unsafe { backend.backend().end_frame_evm(evm, frame) };
        result
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

    /// Executes a transaction replayed by the cheatcode call at `depth`, such as for
    /// `vm.transact`, observed by the whole stack with `cheats` active over the throwaway
    /// `backend` it was prepared with. Its root frame is a call below the cheatcode call.
    fn replay<FEN: FoundryEvmNetwork>(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        prepared: PreparedTx<TypesFor<FEN>>,
        untracked: bool,
    ) -> eyre::Result<ExecutedNested<TypesFor<FEN>>> {
        let caller = prepared.tx().signer();
        let previous =
            self.enter_inner_context(InnerRoot { depth, caller, origin: None, observe: true });
        // The replay reads its own transaction environment, not the surrounding execution's.
        let origin_override = cheats.origin_override.take();
        let gas_price = cheats.gas_price.take();
        // The replayed transaction has its own journal.
        let snapshot_restores = mem::take(&mut cheats.snapshot_restores);
        let fork_id = backend.backend().active_fork_id();
        let (mut gas_price_override, mut blob_hashes_override) = (None, None);
        cheats.env_overrides.swap_transaction_overrides(
            fork_id,
            &mut gas_price_override,
            &mut blob_hashes_override,
        );
        let in_replay = mem::replace(&mut cheats.in_replay, true);
        let mut inspector = InspectorStackRefMut {
            cheatcodes: Some(&mut *cheats),
            inner: &mut *self,
            backend: Some(backend),
        };
        let executed = prepared.execute(evm, Some(&mut inspector), untracked);
        cheats.snapshot_restores = snapshot_restores;
        cheats.in_replay = in_replay;
        cheats.env_overrides.swap_transaction_overrides(
            fork_id,
            &mut gas_price_override,
            &mut blob_hashes_override,
        );
        cheats.origin_override = origin_override;
        cheats.gas_price = gas_price;
        self.exit_inner_context(previous);
        executed
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

/// Warms in `state` the accounts a successful nested transaction loaded and the storage slots it
/// accessed, as accesses across an isolated call boundary stay warm for the calling frame.
/// Accounts in `cold` stay cold.
fn warm_accessed(state: &mut State<'_>, pending: &PendingState, cold: &AddressSet) -> DbResult<()> {
    for change in pending.account_changes() {
        if !cold.contains(&change.address) {
            state.account(&change.address)?.warm();
        }
        for (key, slot) in &change.storage.slots {
            if slot.is_warm {
                state.storage_slot(&change.address, *key)?.warm();
            }
        }
    }
    Ok(())
}

/// Warms in `state` the accounts and storage slots still warm at the end of a failed nested
/// transaction detached into `pending`, which reattaching it to `child` exposes. Warmth no revert
/// undid survives the failure, such as of the accounts a `vm.executeTransaction`
/// within it touched, while accesses of its reverted frames do not. Accounts in `cold` stay cold,
/// but not their slots.
fn warm_surviving(
    state: &mut State<'_>,
    child: &mut State<'_>,
    pending: &PendingState,
    cold: &AddressSet,
) -> DbResult<()> {
    child.set_pending_state(pending.clone());
    let warm = warm_accounts(child);
    child.clear_transaction_state();
    // The running transaction holds every account the nested one loaded, cold unless warmed
    // below, so that a later failure leaves them cold.
    for change in pending.account_changes() {
        state.account(&change.address)?;
    }
    for (address, slots) in warm {
        if !cold.contains(&address) {
            state.account(&address)?.warm();
        }
        for key in slots {
            state.storage_slot(&address, key)?.warm();
        }
    }
    Ok(())
}

/// Returns the EIP-8037 account-creation state gas charged before running `message`, by its CALL
/// or CREATE opcode or at the top level of a transaction: a CREATE, or a value transfer, to an
/// empty account.
fn account_creation_state_gas<FEN: FoundryEvmNetwork>(
    interp: &mut InterpreterFor<'_, '_, FEN>,
    message: &MessageFor<FEN>,
) -> u64 {
    let evm = interp.host();
    let features = evm.version().features;
    let is_create = message.kind == MessageKind::Create;
    if !features.contains(EvmFeatures::EIP8037)
        || !(is_create || !message.value.is_zero())
        || !evm
            .target_is_empty_for_new_account_gas(&message.destination, features)
            .unwrap_or_default()
    {
        return 0;
    }
    let gas_params = &evm.version().gas_params;
    if is_create { gas_params.create_state_gas() } else { gas_params.new_account_state_gas() }
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
