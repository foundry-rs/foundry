use super::{
    Cheatcodes, CheatsConfig, ChiselState, CustomPrintTracer, Fuzzer, LineCoverageCollector,
    LogCollector, RevertDiagnostic, ScriptExecutionInspector, TracingInspector,
};
use alloy_evm::{Evm, eth::EthEvmContext};
use alloy_primitives::{
    Address, Bytes, Log, TxKind, U256,
    map::{AddressHashMap, HashMap},
};
use foundry_cheatcodes::{CheatcodesExecutor, Wallets};
use foundry_evm_core::{
    ContextExt, Env, InspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::new_evm_with_inspector,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_traces::{SparsedTraceArena, TraceMode};
use revm::{
    Inspector,
    context::{
        BlockEnv,
        result::{ExecutionResult, Output},
    },
    context_interface::CreateScheme,
    interpreter::{
        CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, Gas, InstructionResult,
        Interpreter, InterpreterResult,
    },
    state::{Account, AccountStatus},
};
use revm_inspectors::edge_cov::EdgeCovInspector;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct InspectorStackBuilder {
    /// The block environment.
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: Option<BlockEnv>,
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
    pub trace_mode: TraceMode,
    /// Whether logs should be collected.
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
    /// Whether to enable Odyssey features.
    pub odyssey: bool,
    /// The wallets to set in the cheatcodes context.
    pub wallets: Option<Wallets>,
    /// The CREATE2 deployer address.
    pub create2_deployer: Address,
}

impl InspectorStackBuilder {
    /// Create a new inspector stack builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the block environment.
    #[inline]
    pub fn block(mut self, block: BlockEnv) -> Self {
        self.block = Some(block);
        self
    }

    /// Set the gas price.
    #[inline]
    pub fn gas_price(mut self, gas_price: u128) -> Self {
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
    pub fn chisel_state(mut self, final_pc: usize) -> Self {
        self.chisel_state = Some(final_pc);
        self
    }

    /// Set whether to collect logs.
    #[inline]
    pub fn logs(mut self, yes: bool) -> Self {
        self.logs = Some(yes);
        self
    }

    /// Set whether to collect line coverage information.
    #[inline]
    pub fn line_coverage(mut self, yes: bool) -> Self {
        self.line_coverage = Some(yes);
        self
    }

    /// Set whether to enable the trace printer.
    #[inline]
    pub fn print(mut self, yes: bool) -> Self {
        self.print = Some(yes);
        self
    }

    /// Set whether to enable the tracer.
    /// Revert diagnostic inspector is activated when `mode != TraceMode::None`
    #[inline]
    pub fn trace_mode(mut self, mode: TraceMode) -> Self {
        if self.trace_mode < mode {
            self.trace_mode = mode
        }
        self
    }

    /// Set whether to enable the call isolation.
    /// For description of call isolation, see [`InspectorStack::enable_isolation`].
    #[inline]
    pub fn enable_isolation(mut self, yes: bool) -> Self {
        self.enable_isolation = yes;
        self
    }

    /// Set whether to enable Odyssey features.
    /// For description of call isolation, see [`InspectorStack::enable_isolation`].
    #[inline]
    pub fn odyssey(mut self, yes: bool) -> Self {
        self.odyssey = yes;
        self
    }

    #[inline]
    pub fn create2_deployer(mut self, create2_deployer: Address) -> Self {
        self.create2_deployer = create2_deployer;
        self
    }

    /// Builds the stack of inspectors to use when transacting/committing on the EVM.
    pub fn build(self) -> InspectorStack {
        let Self {
            block,
            gas_price,
            cheatcodes,
            fuzzer,
            trace_mode,
            logs,
            line_coverage,
            print,
            chisel_state,
            enable_isolation,
            odyssey,
            wallets,
            create2_deployer,
        } = self;
        let mut stack = InspectorStack::new();

        // inspectors
        if let Some(config) = cheatcodes {
            let mut cheatcodes = Cheatcodes::new(config);
            // Set wallets if they are provided
            if let Some(wallets) = wallets {
                cheatcodes.set_wallets(wallets);
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
        stack.collect_logs(logs.unwrap_or(true));
        stack.print(print.unwrap_or(false));
        stack.tracing(trace_mode);

        stack.enable_isolation(enable_isolation);
        stack.odyssey(odyssey);
        stack.set_create2_deployer(create2_deployer);

        // environment, must come after all of the inspectors
        if let Some(block) = block {
            stack.set_block(&block);
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
pub struct InspectorData {
    pub logs: Vec<Log>,
    pub labels: AddressHashMap<String>,
    pub traces: Option<SparsedTraceArena>,
    pub line_coverage: Option<HitMaps>,
    pub edge_coverage: Option<Vec<u8>>,
    pub cheatcodes: Option<Box<Cheatcodes>>,
    pub chisel_state: Option<(Vec<U256>, Vec<u8>, Option<InstructionResult>)>,
    pub reverter: Option<Address>,
}

/// Contains data about the state of outer/main EVM which created and invoked the inner EVM context.
/// Used to adjust EVM state while in inner context.
///
/// We need this to avoid breaking changes due to EVM behavior differences in isolated vs
/// non-isolated mode. For descriptions and workarounds for those changes see: <https://github.com/foundry-rs/foundry/pull/7186#issuecomment-1959102195>
#[derive(Debug, Clone)]
pub struct InnerContextData {
    /// Origin of the transaction in the outer EVM context.
    original_origin: Address,
}

/// An inspector that calls multiple inspectors in sequence.
///
/// If a call to an inspector returns a value (indicating a stop or revert) the remaining inspectors
/// are not called.
///
/// Stack is divided into [Cheatcodes] and `InspectorStackInner`. This is done to allow assembling
/// `InspectorStackRefMut` inside [Cheatcodes] to allow usage of it as [revm::Inspector]. This gives
/// us ability to create and execute separate EVM frames from inside cheatcodes while still having
/// access to entire stack of inspectors and correctly handling traces, logs, debugging info
/// collection, etc.
#[derive(Clone, Debug, Default)]
pub struct InspectorStack {
    pub cheatcodes: Option<Box<Cheatcodes>>,
    pub inner: InspectorStackInner,
}

/// All used inpectors besides [Cheatcodes].
///
/// See [`InspectorStack`].
#[derive(Default, Clone, Debug)]
pub struct InspectorStackInner {
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
    pub tracer: Option<Box<TracingInspector>>,

    // InspectorExt and other internal data.
    pub enable_isolation: bool,
    pub odyssey: bool,
    pub create2_deployer: Address,
    /// Flag marking if we are in the inner EVM context.
    pub in_inner_context: bool,
    pub inner_context_data: Option<InnerContextData>,
    pub top_frame_journal: HashMap<Address, Account>,
    /// Address that reverted the call, if any.
    pub reverter: Option<Address>,
}

/// Struct keeping mutable references to both parts of [InspectorStack] and implementing
/// [revm::Inspector]. This struct can be obtained via [InspectorStack::as_mut] or via
/// [CheatcodesExecutor::get_inspector] method implemented for [InspectorStackInner].
pub struct InspectorStackRefMut<'a> {
    pub cheatcodes: Option<&'a mut Cheatcodes>,
    pub inner: &'a mut InspectorStackInner,
}

impl CheatcodesExecutor for InspectorStackInner {
    fn get_inspector<'a>(&'a mut self, cheats: &'a mut Cheatcodes) -> Box<dyn InspectorExt + 'a> {
        Box::new(InspectorStackRefMut { cheatcodes: Some(cheats), inner: self })
    }

    fn tracing_inspector(&mut self) -> Option<&mut TracingInspector> {
        self.tracer.as_deref_mut()
    }
}

impl InspectorStack {
    /// Creates a new inspector stack.
    ///
    /// Note that the stack is empty by default, and you must add inspectors to it.
    /// This is done by calling the `set_*` methods on the stack directly, or by building the stack
    /// with [`InspectorStack`].
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Logs the status of the inspectors.
    pub fn log_status(&self) {
        trace!(enabled=%{
            let mut enabled = Vec::with_capacity(16);
            macro_rules! push {
                ($($id:ident),* $(,)?) => {
                    $(
                        if self.$id.is_some() {
                            enabled.push(stringify!($id));
                        }
                    )*
                };
            }
            push!(cheatcodes, chisel_state, line_coverage, fuzzer, log_collector, printer, tracer);
            if self.enable_isolation {
                enabled.push("isolation");
            }
            format!("[{}]", enabled.join(", "))
        });
    }

    /// Set variables from an environment for the relevant inspectors.
    #[inline]
    pub fn set_env(&mut self, env: &Env) {
        self.set_block(&env.evm_env.block_env);
        self.set_gas_price(env.tx.gas_price);
    }

    /// Sets the block for the relevant inspectors.
    #[inline]
    pub fn set_block(&mut self, block: &BlockEnv) {
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.block = Some(block.clone());
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
    pub fn set_cheatcodes(&mut self, cheatcodes: Cheatcodes) {
        self.cheatcodes = Some(cheatcodes.into());
    }

    /// Set the fuzzer inspector.
    #[inline]
    pub fn set_fuzzer(&mut self, fuzzer: Fuzzer) {
        self.fuzzer = Some(fuzzer.into());
    }

    /// Set the Chisel inspector.
    #[inline]
    pub fn set_chisel(&mut self, final_pc: usize) {
        self.chisel_state = Some(ChiselState::new(final_pc).into());
    }

    /// Set whether to enable the line coverage collector.
    #[inline]
    pub fn collect_line_coverage(&mut self, yes: bool) {
        self.line_coverage = yes.then(Default::default);
    }

    /// Set whether to enable the edge coverage collector.
    #[inline]
    pub fn collect_edge_coverage(&mut self, yes: bool) {
        // TODO: configurable edge size?
        self.edge_coverage = yes.then(EdgeCovInspector::new).map(Into::into);
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub fn enable_isolation(&mut self, yes: bool) {
        self.enable_isolation = yes;
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub fn odyssey(&mut self, yes: bool) {
        self.odyssey = yes;
    }

    /// Set the CREATE2 deployer address.
    #[inline]
    pub fn set_create2_deployer(&mut self, deployer: Address) {
        self.create2_deployer = deployer;
    }

    /// Set whether to enable the log collector.
    #[inline]
    pub fn collect_logs(&mut self, yes: bool) {
        self.log_collector = yes.then(Default::default);
    }

    /// Set whether to enable the trace printer.
    #[inline]
    pub fn print(&mut self, yes: bool) {
        self.printer = yes.then(Default::default);
    }

    /// Set whether to enable the tracer.
    /// Revert diagnostic inspector is activated when `mode != TraceMode::None`
    #[inline]
    pub fn tracing(&mut self, mode: TraceMode) {
        self.revert_diag = (!mode.is_none()).then(RevertDiagnostic::default).map(Into::into);

        if let Some(config) = mode.into_config() {
            *self.tracer.get_or_insert_with(Default::default).config_mut() = config;
        } else {
            self.tracer = None;
        }
    }

    /// Set whether to enable script execution inspector.
    #[inline]
    pub fn script(&mut self, script_address: Address) {
        self.script_execution_inspector.get_or_insert_with(Default::default).script_address =
            script_address;
    }

    #[inline(always)]
    fn as_mut(&mut self) -> InspectorStackRefMut<'_> {
        InspectorStackRefMut { cheatcodes: self.cheatcodes.as_deref_mut(), inner: &mut self.inner }
    }

    /// Returns an [`InspectorExt`] using this stack's inspectors.
    #[inline]
    pub fn as_inspector(&mut self) -> impl InspectorExt + '_ {
        self
    }

    /// Collects all the data gathered during inspection into a single struct.
    pub fn collect(self) -> InspectorData {
        let Self {
            mut cheatcodes,
            inner:
                InspectorStackInner {
                    chisel_state,
                    line_coverage,
                    edge_coverage,
                    log_collector,
                    tracer,
                    reverter,
                    ..
                },
        } = self;

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

            SparsedTraceArena { arena, ignored }
        });

        InspectorData {
            logs: log_collector.map(|logs| logs.logs).unwrap_or_default(),
            labels: cheatcodes
                .as_ref()
                .map(|cheatcodes| cheatcodes.labels.clone())
                .unwrap_or_default(),
            traces,
            line_coverage: line_coverage.map(|line_coverage| line_coverage.finish()),
            edge_coverage: edge_coverage.map(|edge_coverage| edge_coverage.into_hitcount()),
            cheatcodes,
            chisel_state: chisel_state.and_then(|state| state.state),
            reverter,
        }
    }
}

impl InspectorStackRefMut<'_> {
    /// Adjusts the EVM data for the inner EVM context.
    /// Should be called on the top-level call of inner context (depth == 0 &&
    /// self.in_inner_context) Decreases sender nonce for CALLs to keep backwards compatibility
    /// Updates tx.origin to the value before entering inner context
    fn adjust_evm_data_for_inner_context(&mut self, ecx: &mut EthEvmContext<&mut dyn DatabaseExt>) {
        let inner_context_data =
            self.inner_context_data.as_ref().expect("should be called in inner context");
        ecx.tx.caller = inner_context_data.original_origin;
    }

    fn do_call_end(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) -> CallOutcome {
        let result = outcome.result.result;
        call_inspectors!(
            #[ret]
            [
                &mut self.fuzzer,
                &mut self.tracer,
                &mut self.cheatcodes,
                &mut self.printer,
                &mut self.revert_diag
            ],
            |inspector| {
                let previous_outcome = outcome.clone();
                inspector.call_end(ecx, inputs, outcome);

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                let different = outcome.result.result != result
                    || (outcome.result.result == InstructionResult::Revert
                        && outcome.output() != previous_outcome.output());
                different.then_some(outcome.clone())
            },
        );

        // Record first address that reverted the call.
        if result.is_revert() && self.reverter.is_none() {
            self.reverter = Some(inputs.target_address);
        }

        outcome.clone()
    }

    fn do_create_end(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        call: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) -> CreateOutcome {
        let result = outcome.result.result;
        call_inspectors!(
            #[ret]
            [&mut self.tracer, &mut self.cheatcodes, &mut self.printer],
            |inspector| {
                let previous_outcome = outcome.clone();
                inspector.create_end(ecx, call, outcome);

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                let different = outcome.result.result != result
                    || (outcome.result.result == InstructionResult::Revert
                        && outcome.output() != previous_outcome.output());
                different.then_some(outcome.clone())
            },
        );

        outcome.clone()
    }

    fn transact_inner(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        kind: TxKind,
        caller: Address,
        input: Bytes,
        gas_limit: u64,
        value: U256,
    ) -> (InterpreterResult, Option<Address>) {
        let cached_env = Env::from(ecx.cfg.clone(), ecx.block.clone(), ecx.tx.clone());

        ecx.block.basefee = 0;
        ecx.tx.chain_id = Some(ecx.cfg.chain_id);
        ecx.tx.caller = caller;
        ecx.tx.kind = kind;
        ecx.tx.data = input;
        ecx.tx.value = value;
        // Add 21000 to the gas limit to account for the base cost of transaction.
        ecx.tx.gas_limit = gas_limit + 21000;

        // If we haven't disabled gas limit checks, ensure that transaction gas limit will not
        // exceed block gas limit.
        if !ecx.cfg.disable_block_gas_limit {
            ecx.tx.gas_limit = std::cmp::min(ecx.tx.gas_limit, ecx.block.gas_limit);
        }
        ecx.tx.gas_price = 0;

        self.inner_context_data = Some(InnerContextData { original_origin: cached_env.tx.caller });
        self.in_inner_context = true;

        let res = self.with_inspector(|inspector| {
            let (db, journal, env) = ecx.as_db_env_and_journal();
            let mut evm = new_evm_with_inspector(db, env.to_owned(), inspector);

            evm.journaled_state.state = {
                let mut state = journal.state.clone();

                for (addr, acc_mut) in &mut state {
                    // mark all accounts cold, besides preloaded addresses
                    if !journal.warm_preloaded_addresses.contains(addr) {
                        acc_mut.mark_cold();
                    }

                    // mark all slots cold
                    for slot_mut in acc_mut.storage.values_mut() {
                        slot_mut.is_cold = true;
                        slot_mut.original_value = slot_mut.present_value;
                    }
                }

                state
            };

            // set depth to 1 to make sure traces are collected correctly
            evm.journaled_state.depth = 1;

            let res = evm.transact(env.tx.clone());

            // need to reset the env in case it was modified via cheatcodes during execution
            *env.cfg = evm.cfg.clone();
            *env.block = evm.block.clone();

            *env.tx = cached_env.tx;
            env.block.basefee = cached_env.evm_env.block_env.basefee;

            res
        });

        self.in_inner_context = false;
        self.inner_context_data = None;

        let mut gas = Gas::new(gas_limit);

        let Ok(res) = res else {
            // Should we match, encode and propagate error as a revert reason?
            let result =
                InterpreterResult { result: InstructionResult::Revert, output: Bytes::new(), gas };
            return (result, None);
        };

        for (addr, mut acc) in res.state {
            let Some(acc_mut) = ecx.journaled_state.state.get_mut(&addr) else {
                ecx.journaled_state.state.insert(addr, acc);
                continue;
            };

            // make sure accounts that were warmed earlier do not become cold
            if acc.status.contains(AccountStatus::Cold)
                && !acc_mut.status.contains(AccountStatus::Cold)
            {
                acc.status -= AccountStatus::Cold;
            }
            acc_mut.info = acc.info;
            acc_mut.status |= acc.status;

            for (key, val) in acc.storage {
                let Some(slot_mut) = acc_mut.storage.get_mut(&key) else {
                    acc_mut.storage.insert(key, val);
                    continue;
                };
                slot_mut.present_value = val.present_value;
                slot_mut.is_cold &= val.is_cold;
            }
        }

        let (result, address, output) = match res.result {
            ExecutionResult::Success { reason, gas_used, gas_refunded, logs: _, output } => {
                gas.set_refund(gas_refunded as i64);
                let _ = gas.record_cost(gas_used);
                let address = match output {
                    Output::Create(_, address) => address,
                    Output::Call(_) => None,
                };
                (reason.into(), address, output.into_data())
            }
            ExecutionResult::Halt { reason, gas_used } => {
                let _ = gas.record_cost(gas_used);
                (reason.into(), None, Bytes::new())
            }
            ExecutionResult::Revert { gas_used, output } => {
                let _ = gas.record_cost(gas_used);
                (InstructionResult::Revert, None, output)
            }
        };
        (InterpreterResult { result, output, gas }, address)
    }

    /// Moves out of references, constructs a new [`InspectorStackRefMut`] and runs the given
    /// closure with it.
    fn with_inspector<O>(&mut self, f: impl FnOnce(InspectorStackRefMut<'_>) -> O) -> O {
        let mut cheatcodes = self
            .cheatcodes
            .as_deref_mut()
            .map(|cheats| core::mem::replace(cheats, Cheatcodes::new(cheats.config.clone())));
        let mut inner = std::mem::take(self.inner);

        let out = f(InspectorStackRefMut { cheatcodes: cheatcodes.as_mut(), inner: &mut inner });

        if let Some(cheats) = self.cheatcodes.as_deref_mut() {
            *cheats = cheatcodes.unwrap();
        }

        *self.inner = inner;

        out
    }

    /// Invoked at the beginning of a new top-level (0 depth) frame.
    fn top_level_frame_start(&mut self, ecx: &mut EthEvmContext<&mut dyn DatabaseExt>) {
        if self.enable_isolation {
            // If we're in isolation mode, we need to keep track of the state at the beginning of
            // the frame to be able to roll back on revert
            self.top_frame_journal.clone_from(&ecx.journaled_state.state);
        }
    }

    /// Invoked at the end of root frame.
    fn top_level_frame_end(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        result: InstructionResult,
    ) {
        if !result.is_revert() {
            return;
        }
        // Encountered a revert, since cheatcodes may have altered the evm state in such a way
        // that violates some constraints, e.g. `deal`, we need to manually roll back on revert
        // before revm reverts the state itself
        if let Some(cheats) = self.cheatcodes.as_mut() {
            cheats.on_revert(ecx);
        }

        // If we're in isolation mode, we need to rollback to state before the root frame was
        // created We can't rely on revm's journal because it doesn't account for changes
        // made by isolated calls
        if self.enable_isolation {
            ecx.journaled_state.state = std::mem::take(&mut self.top_frame_journal);
        }
    }

    // We take extra care in optimizing `step` and `step_end`, as they're are likely the most
    // hot functions in all of Foundry.
    // We want to `#[inline(always)]` these functions so that `InspectorStack` does not
    // delegate to `InspectorStackRefMut` in this case.

    #[inline(always)]
    fn step_inlined(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        call_inspectors!(
            [
                // These are sorted in definition order.
                &mut self.edge_coverage,
                &mut self.fuzzer,
                &mut self.line_coverage,
                &mut self.printer,
                &mut self.revert_diag,
                &mut self.script_execution_inspector,
                &mut self.tracer,
                // Keep `cheatcodes` last to make use of the tail call.
                &mut self.cheatcodes,
            ],
            |inspector| (**inspector).step(interpreter, ecx),
        );
    }

    #[inline(always)]
    fn step_end_inlined(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        call_inspectors!(
            [
                // These are sorted in definition order.
                &mut self.chisel_state,
                &mut self.printer,
                &mut self.revert_diag,
                &mut self.tracer,
                // Keep `cheatcodes` last to make use of the tail call.
                &mut self.cheatcodes,
            ],
            |inspector| (**inspector).step_end(interpreter, ecx),
        );
    }
}

impl Inspector<EthEvmContext<&mut dyn DatabaseExt>> for InspectorStackRefMut<'_> {
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        call_inspectors!(
            [
                &mut self.line_coverage,
                &mut self.tracer,
                &mut self.cheatcodes,
                &mut self.script_execution_inspector,
                &mut self.printer
            ],
            |inspector| inspector.initialize_interp(interpreter, ecx),
        );
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        self.step_inlined(interpreter, ecx);
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        self.step_end_inlined(interpreter, ecx);
    }

    #[allow(clippy::redundant_clone)]
    fn log(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        log: Log,
    ) {
        call_inspectors!(
            [&mut self.tracer, &mut self.log_collector, &mut self.cheatcodes, &mut self.printer],
            |inspector| inspector.log(interpreter, ecx, log.clone()),
        );
    }

    fn call(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        call: &mut CallInputs,
    ) -> Option<CallOutcome> {
        if self.in_inner_context && ecx.journaled_state.depth == 1 {
            self.adjust_evm_data_for_inner_context(ecx);
            return None;
        }

        if ecx.journaled_state.depth == 0 {
            self.top_level_frame_start(ecx);
        }

        call_inspectors!(
            #[ret]
            [
                &mut self.fuzzer,
                &mut self.tracer,
                &mut self.log_collector,
                &mut self.printer,
                &mut self.revert_diag
            ],
            |inspector| {
                let mut out = None;
                if let Some(output) = inspector.call(ecx, call) {
                    out = Some(Some(output));
                }
                out
            },
        );

        if let Some(cheatcodes) = self.cheatcodes.as_deref_mut() {
            // Handle mocked functions, replace bytecode address with mock if matched.
            if let Some(mocks) = cheatcodes.mocked_functions.get(&call.target_address) {
                // Check if any mock function set for call data or if catch-all mock function set
                // for selector.
                if let Some(target) = mocks.get(&call.input.bytes(ecx)).or_else(|| {
                    call.input.bytes(ecx).get(..4).and_then(|selector| mocks.get(selector))
                }) {
                    call.bytecode_address = *target;
                }
            }

            if let Some(output) = cheatcodes.call_with_executor(ecx, call, self.inner) {
                return Some(output);
            }
        }

        if self.enable_isolation && !self.in_inner_context && ecx.journaled_state.depth == 1 {
            match call.scheme {
                // Isolate CALLs
                CallScheme::Call => {
                    let input = call.input.bytes(ecx);
                    let (result, _) = self.transact_inner(
                        ecx,
                        TxKind::Call(call.target_address),
                        call.caller,
                        input,
                        call.gas_limit,
                        call.value.get(),
                    );
                    return Some(CallOutcome {
                        result,
                        memory_offset: call.return_memory_offset.clone(),
                    });
                }
                // Mark accounts and storage cold before STATICCALLs
                CallScheme::StaticCall => {
                    let JournaledState { state, warm_preloaded_addresses, .. } =
                        &mut ecx.journaled_state.inner;
                    for (addr, acc_mut) in state {
                        // Do not mark accounts and storage cold accounts with arbitrary storage.
                        if let Some(cheatcodes) = &self.cheatcodes
                            && cheatcodes.has_arbitrary_storage(addr)
                        {
                            continue;
                        }

                        if !warm_preloaded_addresses.contains(addr) {
                            acc_mut.mark_cold();
                        }

                        for slot_mut in acc_mut.storage.values_mut() {
                            slot_mut.is_cold = true;
                        }
                    }
                }
                // Process other variants as usual
                CallScheme::CallCode | CallScheme::DelegateCall => {}
            }
        }

        None
    }

    fn call_end(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        // We are processing inner context outputs in the outer context, so need to avoid processing
        // twice.
        if self.in_inner_context && ecx.journaled_state.depth == 1 {
            return;
        }

        self.do_call_end(ecx, inputs, outcome);

        if ecx.journaled_state.depth == 0 {
            self.top_level_frame_end(ecx, outcome.result.result);
        }
    }

    fn create(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        create: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if self.in_inner_context && ecx.journaled_state.depth == 1 {
            self.adjust_evm_data_for_inner_context(ecx);
            return None;
        }

        if ecx.journaled_state.depth == 0 {
            self.top_level_frame_start(ecx);
        }

        call_inspectors!(
            #[ret]
            [&mut self.tracer, &mut self.line_coverage, &mut self.cheatcodes],
            |inspector| inspector.create(ecx, create).map(Some),
        );

        if !matches!(create.scheme, CreateScheme::Create2 { .. })
            && self.enable_isolation
            && !self.in_inner_context
            && ecx.journaled_state.depth == 1
        {
            let (result, address) = self.transact_inner(
                ecx,
                TxKind::Create,
                create.caller,
                create.init_code.clone(),
                create.gas_limit,
                create.value,
            );
            return Some(CreateOutcome { result, address });
        }

        None
    }

    fn create_end(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        call: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        // We are processing inner context outputs in the outer context, so need to avoid processing
        // twice.
        if self.in_inner_context && ecx.journaled_state.depth == 1 {
            return;
        }

        self.do_create_end(ecx, call, outcome);

        if ecx.journaled_state.depth == 0 {
            self.top_level_frame_end(ecx, outcome.result.result);
        }
    }
}

impl InspectorExt for InspectorStackRefMut<'_> {
    fn should_use_create2_factory(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &CreateInputs,
    ) -> bool {
        call_inspectors!(
            #[ret]
            [&mut self.cheatcodes],
            |inspector| { inspector.should_use_create2_factory(ecx, inputs).then_some(true) },
        );

        false
    }

    fn console_log(&mut self, msg: &str) {
        call_inspectors!([&mut self.log_collector], |inspector| InspectorExt::console_log(
            inspector, msg
        ));
    }

    fn is_odyssey(&self) -> bool {
        self.inner.odyssey
    }

    fn create2_deployer(&self) -> Address {
        self.inner.create2_deployer
    }
}

impl Inspector<EthEvmContext<&mut dyn DatabaseExt>> for InspectorStack {
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        self.as_mut().step_inlined(interpreter, ecx)
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        self.as_mut().step_end_inlined(interpreter, ecx)
    }

    fn call(
        &mut self,
        context: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        self.as_mut().call(context, inputs)
    }

    fn call_end(
        &mut self,
        context: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        self.as_mut().call_end(context, inputs, outcome)
    }

    fn create(
        &mut self,
        context: &mut EthEvmContext<&mut dyn DatabaseExt>,
        create: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        self.as_mut().create(context, create)
    }

    fn create_end(
        &mut self,
        context: &mut EthEvmContext<&mut dyn DatabaseExt>,
        call: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.as_mut().create_end(context, call, outcome)
    }

    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
    ) {
        self.as_mut().initialize_interp(interpreter, ecx)
    }

    fn log(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        log: Log,
    ) {
        self.as_mut().log(interpreter, ecx, log)
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        self.as_mut().selfdestruct(contract, target, value);
    }
}

impl InspectorExt for InspectorStack {
    fn should_use_create2_factory(
        &mut self,
        ecx: &mut EthEvmContext<&mut dyn DatabaseExt>,
        inputs: &CreateInputs,
    ) -> bool {
        self.as_mut().should_use_create2_factory(ecx, inputs)
    }

    fn is_odyssey(&self) -> bool {
        self.odyssey
    }

    fn create2_deployer(&self) -> Address {
        self.create2_deployer
    }
}

impl<'a> Deref for InspectorStackRefMut<'a> {
    type Target = &'a mut InspectorStackInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for InspectorStackRefMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Deref for InspectorStack {
    type Target = InspectorStackInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for InspectorStack {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
