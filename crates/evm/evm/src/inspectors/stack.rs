use super::{
    Cheatcodes, CheatsConfig, ChiselState, CoverageCollector, Fuzzer, LogCollector,
    TracingInspector,
};
use alloy_primitives::{Address, Bytes, Log, TxKind, U256};
use foundry_cheatcodes::CheatcodesExecutor;
use foundry_evm_core::{
    backend::{update_state, DatabaseExt},
    InspectorExt,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_traces::{CallTraceArena, TraceMode};
use revm::{
    inspectors::CustomPrintTracer,
    interpreter::{
        CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, EOFCreateInputs,
        EOFCreateKind, Gas, InstructionResult, Interpreter, InterpreterResult,
    },
    primitives::{
        BlockEnv, CreateScheme, Env, EnvWithHandlerCfg, ExecutionResult, Output, TransactTo,
    },
    EvmContext, Inspector,
};
use std::{
    collections::HashMap,
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
    pub gas_price: Option<U256>,
    /// The cheatcodes config.
    pub cheatcodes: Option<Arc<CheatsConfig>>,
    /// The fuzzer inspector and its state, if it exists.
    pub fuzzer: Option<Fuzzer>,
    /// Whether to enable tracing.
    pub trace_mode: TraceMode,
    /// Whether logs should be collected.
    pub logs: Option<bool>,
    /// Whether coverage info should be collected.
    pub coverage: Option<bool>,
    /// Whether to print all opcode traces into the console. Useful for debugging the EVM.
    pub print: Option<bool>,
    /// The chisel state inspector.
    pub chisel_state: Option<usize>,
    /// Whether to enable call isolation.
    /// In isolation mode all top-level calls are executed as a separate transaction in a separate
    /// EVM context, enabling more precise gas accounting and transaction state changes.
    pub enable_isolation: bool,
    /// Whether to enable Alphanet features.
    pub alphanet: bool,
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
    pub fn gas_price(mut self, gas_price: U256) -> Self {
        self.gas_price = Some(gas_price);
        self
    }

    /// Enable cheatcodes with the given config.
    #[inline]
    pub fn cheatcodes(mut self, config: Arc<CheatsConfig>) -> Self {
        self.cheatcodes = Some(config);
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

    /// Set whether to collect coverage information.
    #[inline]
    pub fn coverage(mut self, yes: bool) -> Self {
        self.coverage = Some(yes);
        self
    }

    /// Set whether to enable the trace printer.
    #[inline]
    pub fn print(mut self, yes: bool) -> Self {
        self.print = Some(yes);
        self
    }

    /// Set whether to enable the tracer.
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

    /// Set whether to enable Alphanet features.
    /// For description of call isolation, see [`InspectorStack::enable_isolation`].
    #[inline]
    pub fn alphanet(mut self, yes: bool) -> Self {
        self.alphanet = yes;
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
            coverage,
            print,
            chisel_state,
            enable_isolation,
            alphanet,
        } = self;
        let mut stack = InspectorStack::new();

        // inspectors
        if let Some(config) = cheatcodes {
            stack.set_cheatcodes(Cheatcodes::new(config));
        }
        if let Some(fuzzer) = fuzzer {
            stack.set_fuzzer(fuzzer);
        }
        if let Some(chisel_state) = chisel_state {
            stack.set_chisel(chisel_state);
        }
        stack.collect_coverage(coverage.unwrap_or(false));
        stack.collect_logs(logs.unwrap_or(true));
        stack.print(print.unwrap_or(false));
        stack.tracing(trace_mode);

        stack.enable_isolation(enable_isolation);
        stack.alphanet(alphanet);

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
    ([$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr $(,)?) => {
        $(
            if let Some($id) = $inspector {
                ({ #[inline(always)] #[cold] || $call })();
            }
        )+
    };
    (#[ret] [$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr $(,)?) => {
        $(
            if let Some($id) = $inspector {
                if let Some(result) = ({ #[inline(always)] #[cold] || $call })() {
                    return result;
                }
            }
        )+
    };
}

/// Same as [`call_inspectors!`], but with depth adjustment for isolated execution.
macro_rules! call_inspectors_adjust_depth {
    ([$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr, $self:ident, $data:ident $(,)?) => {
        $data.journaled_state.depth += $self.in_inner_context as usize;
        call_inspectors!([$($inspector),+], |$id| $call);
        $data.journaled_state.depth -= $self.in_inner_context as usize;
    };
    (#[ret] [$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr, $self:ident, $data:ident $(,)?) => {
        $data.journaled_state.depth += $self.in_inner_context as usize;
        $(
            if let Some($id) = $inspector {
                if let Some(result) = ({ #[inline(always)] #[cold] || $call })() {
                    $data.journaled_state.depth -= $self.in_inner_context as usize;
                    return result;
                }
            }
        )+
        $data.journaled_state.depth -= $self.in_inner_context as usize;
    };
}

/// The collected results of [`InspectorStack`].
pub struct InspectorData {
    pub logs: Vec<Log>,
    pub labels: HashMap<Address, String>,
    pub traces: Option<CallTraceArena>,
    pub coverage: Option<HitMaps>,
    pub cheatcodes: Option<Cheatcodes>,
    pub chisel_state: Option<(Vec<U256>, Vec<u8>, InstructionResult)>,
}

/// Contains data about the state of outer/main EVM which created and invoked the inner EVM context.
/// Used to adjust EVM state while in inner context.
///
/// We need this to avoid breaking changes due to EVM behavior differences in isolated vs
/// non-isolated mode. For descriptions and workarounds for those changes see: <https://github.com/foundry-rs/foundry/pull/7186#issuecomment-1959102195>
#[derive(Debug, Clone)]
pub struct InnerContextData {
    /// The sender of the inner EVM context.
    /// It is also an origin of the transaction that created the inner EVM context.
    sender: Address,
    /// Nonce of the sender before invocation of the inner EVM context.
    original_sender_nonce: u64,
    /// Origin of the transaction in the outer EVM context.
    original_origin: Address,
    /// Whether the inner context was created by a CREATE transaction.
    is_create: bool,
}

/// An inspector that calls multiple inspectors in sequence.
///
/// If a call to an inspector returns a value other than [InstructionResult::Continue] (or
/// equivalent) the remaining inspectors are not called.
///
/// Stack is divided into [Cheatcodes] and `InspectorStackInner`. This is done to allow assembling
/// `InspectorStackRefMut` inside [Cheatcodes] to allow usage of it as [revm::Inspector]. This gives
/// us ability to create and execute separate EVM frames from inside cheatcodes while still having
/// access to entire stack of inspectors and correctly handling traces, logs, debugging info
/// collection, etc.
#[derive(Clone, Debug, Default)]
pub struct InspectorStack {
    pub cheatcodes: Option<Cheatcodes>,
    pub inner: InspectorStackInner,
}

/// All used inpectors besides [Cheatcodes].
///
/// See [`InspectorStack`].
#[derive(Default, Clone, Debug)]
pub struct InspectorStackInner {
    pub chisel_state: Option<ChiselState>,
    pub coverage: Option<CoverageCollector>,
    pub fuzzer: Option<Fuzzer>,
    pub log_collector: Option<LogCollector>,
    pub printer: Option<CustomPrintTracer>,
    pub tracer: Option<TracingInspector>,
    pub enable_isolation: bool,
    pub alphanet: bool,

    /// Flag marking if we are in the inner EVM context.
    pub in_inner_context: bool,
    pub inner_context_data: Option<InnerContextData>,
}

/// Struct keeping mutable references to both parts of [InspectorStack] and implementing
/// [revm::Inspector]. This struct can be obtained via [InspectorStack::as_mut] or via
/// [CheatcodesExecutor::get_inspector] method implemented for [InspectorStackInner].
pub struct InspectorStackRefMut<'a> {
    pub cheatcodes: Option<&'a mut Cheatcodes>,
    pub inner: &'a mut InspectorStackInner,
}

impl CheatcodesExecutor for InspectorStackInner {
    fn get_inspector<'a, DB: DatabaseExt>(
        &'a mut self,
        cheats: &'a mut Cheatcodes,
    ) -> impl InspectorExt<DB> + 'a {
        InspectorStackRefMut { cheatcodes: Some(cheats), inner: self }
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
            push!(cheatcodes, chisel_state, coverage, fuzzer, log_collector, printer, tracer);
            if self.enable_isolation {
                enabled.push("isolation");
            }
            format!("[{}]", enabled.join(", "))
        });
    }

    /// Set variables from an environment for the relevant inspectors.
    #[inline]
    pub fn set_env(&mut self, env: &Env) {
        self.set_block(&env.block);
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
    pub fn set_gas_price(&mut self, gas_price: U256) {
        if let Some(cheatcodes) = &mut self.cheatcodes {
            cheatcodes.gas_price = Some(gas_price);
        }
    }

    /// Set the cheatcodes inspector.
    #[inline]
    pub fn set_cheatcodes(&mut self, cheatcodes: Cheatcodes) {
        self.cheatcodes = Some(cheatcodes);
    }

    /// Set the fuzzer inspector.
    #[inline]
    pub fn set_fuzzer(&mut self, fuzzer: Fuzzer) {
        self.fuzzer = Some(fuzzer);
    }

    /// Set the Chisel inspector.
    #[inline]
    pub fn set_chisel(&mut self, final_pc: usize) {
        self.chisel_state = Some(ChiselState::new(final_pc));
    }

    /// Set whether to enable the coverage collector.
    #[inline]
    pub fn collect_coverage(&mut self, yes: bool) {
        self.coverage = yes.then(Default::default);
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub fn enable_isolation(&mut self, yes: bool) {
        self.enable_isolation = yes;
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub fn alphanet(&mut self, yes: bool) {
        self.alphanet = yes;
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
    #[inline]
    pub fn tracing(&mut self, mode: TraceMode) {
        if let Some(config) = mode.into_config() {
            *self.tracer.get_or_insert_with(Default::default).config_mut() = config;
        } else {
            self.tracer = None;
        }
    }

    /// Collects all the data gathered during inspection into a single struct.
    #[inline]
    pub fn collect(self) -> InspectorData {
        let Self {
            cheatcodes,
            inner: InspectorStackInner { chisel_state, coverage, log_collector, tracer, .. },
        } = self;

        InspectorData {
            logs: log_collector.map(|logs| logs.logs).unwrap_or_default(),
            labels: cheatcodes
                .as_ref()
                .map(|cheatcodes| cheatcodes.labels.clone())
                .unwrap_or_default(),
            traces: tracer.map(|tracer| tracer.into_traces()),
            coverage: coverage.map(|coverage| coverage.maps),
            cheatcodes,
            chisel_state: chisel_state.and_then(|state| state.state),
        }
    }

    #[inline(always)]
    fn as_mut(&mut self) -> InspectorStackRefMut<'_> {
        InspectorStackRefMut { cheatcodes: self.cheatcodes.as_mut(), inner: &mut self.inner }
    }
}

impl<'a> InspectorStackRefMut<'a> {
    /// Adjusts the EVM data for the inner EVM context.
    /// Should be called on the top-level call of inner context (depth == 0 &&
    /// self.in_inner_context) Decreases sender nonce for CALLs to keep backwards compatibility
    /// Updates tx.origin to the value before entering inner context
    fn adjust_evm_data_for_inner_context<DB: DatabaseExt>(&mut self, ecx: &mut EvmContext<DB>) {
        let inner_context_data =
            self.inner_context_data.as_ref().expect("should be called in inner context");
        let sender_acc = ecx
            .journaled_state
            .state
            .get_mut(&inner_context_data.sender)
            .expect("failed to load sender");
        if !inner_context_data.is_create {
            sender_acc.info.nonce = inner_context_data.original_sender_nonce;
        }
        ecx.env.tx.caller = inner_context_data.original_origin;
    }

    fn do_call_end<DB: DatabaseExt>(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        let result = outcome.result.result;
        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.fuzzer, &mut self.tracer, &mut self.cheatcodes, &mut self.printer],
            |inspector| {
                let new_outcome = inspector.call_end(ecx, inputs, outcome.clone());

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                let different = new_outcome.result.result != result ||
                    (new_outcome.result.result == InstructionResult::Revert &&
                        new_outcome.output() != outcome.output());
                different.then_some(new_outcome)
            },
            self,
            ecx
        );

        outcome
    }

    fn transact_inner<DB: DatabaseExt>(
        &mut self,
        ecx: &mut EvmContext<DB>,
        transact_to: TransactTo,
        caller: Address,
        input: Bytes,
        gas_limit: u64,
        value: U256,
    ) -> (InterpreterResult, Option<Address>) {
        let ecx = &mut ecx.inner;

        ecx.db.commit(ecx.journaled_state.state.clone());

        let nonce = ecx
            .journaled_state
            .load_account(caller, &mut ecx.db)
            .expect("failed to load caller")
            .info
            .nonce;

        let cached_env = ecx.env.clone();

        ecx.env.block.basefee = U256::ZERO;
        ecx.env.tx.caller = caller;
        ecx.env.tx.transact_to = transact_to;
        ecx.env.tx.data = input;
        ecx.env.tx.value = value;
        ecx.env.tx.nonce = Some(nonce);
        // Add 21000 to the gas limit to account for the base cost of transaction.
        ecx.env.tx.gas_limit = gas_limit + 21000;
        // If we haven't disabled gas limit checks, ensure that transaction gas limit will not
        // exceed block gas limit.
        if !ecx.env.cfg.disable_block_gas_limit {
            ecx.env.tx.gas_limit =
                std::cmp::min(ecx.env.tx.gas_limit, ecx.env.block.gas_limit.to());
        }
        ecx.env.tx.gas_price = U256::ZERO;

        self.inner_context_data = Some(InnerContextData {
            sender: ecx.env.tx.caller,
            original_origin: cached_env.tx.caller,
            original_sender_nonce: nonce,
            is_create: matches!(transact_to, TxKind::Create),
        });
        self.in_inner_context = true;

        let env = EnvWithHandlerCfg::new_with_spec_id(ecx.env.clone(), ecx.spec_id());
        let res = {
            let mut evm = crate::utils::new_evm_with_inspector(
                &mut ecx.db as &mut dyn DatabaseExt,
                env,
                &mut *self,
            );
            let res = evm.transact();

            // need to reset the env in case it was modified via cheatcodes during execution
            ecx.env = evm.context.evm.inner.env;
            res
        };

        self.in_inner_context = false;
        self.inner_context_data = None;

        ecx.env.tx = cached_env.tx;
        ecx.env.block.basefee = cached_env.block.basefee;

        let mut gas = Gas::new(gas_limit);

        let Ok(mut res) = res else {
            // Should we match, encode and propagate error as a revert reason?
            let result =
                InterpreterResult { result: InstructionResult::Revert, output: Bytes::new(), gas };
            return (result, None);
        };

        // Commit changes after transaction
        ecx.db.commit(res.state.clone());

        // Update both states with new DB data after commit.
        if let Err(e) = update_state(&mut ecx.journaled_state.state, &mut ecx.db, None) {
            let res = InterpreterResult {
                result: InstructionResult::Revert,
                output: Bytes::from(e.to_string()),
                gas,
            };
            return (res, None);
        }
        if let Err(e) = update_state(&mut res.state, &mut ecx.db, None) {
            let res = InterpreterResult {
                result: InstructionResult::Revert,
                output: Bytes::from(e.to_string()),
                gas,
            };
            return (res, None);
        }

        // Merge transaction journal into the active journal.
        for (addr, acc) in res.state {
            if let Some(acc_mut) = ecx.journaled_state.state.get_mut(&addr) {
                acc_mut.status |= acc.status;
                for (key, val) in acc.storage {
                    acc_mut.storage.entry(key).or_insert(val);
                }
            } else {
                ecx.journaled_state.state.insert(addr, acc);
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
}

impl<'a, DB: DatabaseExt> Inspector<DB> for InspectorStackRefMut<'a> {
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors_adjust_depth!(
            [&mut self.coverage, &mut self.tracer, &mut self.cheatcodes, &mut self.printer],
            |inspector| inspector.initialize_interp(interpreter, ecx),
            self,
            ecx
        );
    }

    fn step(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors_adjust_depth!(
            [
                &mut self.fuzzer,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.cheatcodes,
                &mut self.printer,
            ],
            |inspector| inspector.step(interpreter, ecx),
            self,
            ecx
        );
    }

    fn step_end(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        call_inspectors_adjust_depth!(
            [&mut self.tracer, &mut self.chisel_state, &mut self.printer],
            |inspector| inspector.step_end(interpreter, ecx),
            self,
            ecx
        );
    }

    fn log(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>, log: &Log) {
        call_inspectors_adjust_depth!(
            [&mut self.tracer, &mut self.log_collector, &mut self.cheatcodes, &mut self.printer],
            |inspector| inspector.log(interpreter, ecx, log),
            self,
            ecx
        );
    }

    fn call(&mut self, ecx: &mut EvmContext<DB>, call: &mut CallInputs) -> Option<CallOutcome> {
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            self.adjust_evm_data_for_inner_context(ecx);
            return None;
        }

        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.fuzzer, &mut self.tracer, &mut self.log_collector, &mut self.printer],
            |inspector| {
                let mut out = None;
                if let Some(output) = inspector.call(ecx, call) {
                    if output.result.result != InstructionResult::Continue {
                        out = Some(Some(output));
                    }
                }
                out
            },
            self,
            ecx
        );

        ecx.journaled_state.depth += self.in_inner_context as usize;
        if let Some(cheatcodes) = self.cheatcodes.as_deref_mut() {
            if let Some(output) = cheatcodes.call_with_executor(ecx, call, self.inner) {
                if output.result.result != InstructionResult::Continue {
                    ecx.journaled_state.depth -= self.in_inner_context as usize;
                    return Some(output);
                }
            }
        }
        ecx.journaled_state.depth -= self.in_inner_context as usize;

        if self.enable_isolation &&
            call.scheme == CallScheme::Call &&
            !self.in_inner_context &&
            ecx.journaled_state.depth == 1
        {
            let (result, _) = self.transact_inner(
                ecx,
                TxKind::Call(call.target_address),
                call.caller,
                call.input.clone(),
                call.gas_limit,
                call.value.get(),
            );
            return Some(CallOutcome { result, memory_offset: call.return_memory_offset.clone() });
        }

        None
    }

    fn call_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        // Inner context calls with depth 0 are being dispatched as top-level calls with depth 1.
        // Avoid processing twice.
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            return outcome;
        }

        let outcome = self.do_call_end(ecx, inputs, outcome);
        if outcome.result.is_revert() {
            // Encountered a revert, since cheatcodes may have altered the evm state in such a way
            // that violates some constraints, e.g. `deal`, we need to manually roll back on revert
            // before revm reverts the state itself
            if let Some(cheats) = self.cheatcodes.as_mut() {
                cheats.on_revert(ecx);
            }
        }

        outcome
    }

    fn create(
        &mut self,
        ecx: &mut EvmContext<DB>,
        create: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            self.adjust_evm_data_for_inner_context(ecx);
            return None;
        }

        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.tracer, &mut self.coverage, &mut self.cheatcodes],
            |inspector| inspector.create(ecx, create).map(Some),
            self,
            ecx
        );

        if !matches!(create.scheme, CreateScheme::Create2 { .. }) &&
            self.enable_isolation &&
            !self.in_inner_context &&
            ecx.journaled_state.depth == 1
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
        ecx: &mut EvmContext<DB>,
        call: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        // Inner context calls with depth 0 are being dispatched as top-level calls with depth 1.
        // Avoid processing twice.
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            return outcome;
        }

        let result = outcome.result.result;

        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.tracer, &mut self.cheatcodes, &mut self.printer],
            |inspector| {
                let new_outcome = inspector.create_end(ecx, call, outcome.clone());

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                let different = new_outcome.result.result != result ||
                    (new_outcome.result.result == InstructionResult::Revert &&
                        new_outcome.output() != outcome.output());
                different.then_some(new_outcome)
            },
            self,
            ecx
        );

        outcome
    }

    fn eofcreate(
        &mut self,
        ecx: &mut EvmContext<DB>,
        create: &mut EOFCreateInputs,
    ) -> Option<CreateOutcome> {
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            self.adjust_evm_data_for_inner_context(ecx);
            return None;
        }

        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.tracer, &mut self.coverage, &mut self.cheatcodes],
            |inspector| inspector.eofcreate(ecx, create).map(Some),
            self,
            ecx
        );

        if matches!(create.kind, EOFCreateKind::Tx { .. }) &&
            self.enable_isolation &&
            !self.in_inner_context &&
            ecx.journaled_state.depth == 1
        {
            let init_code = match &mut create.kind {
                EOFCreateKind::Tx { initdata } => initdata.clone(),
                EOFCreateKind::Opcode { .. } => unreachable!(),
            };

            let (result, address) = self.transact_inner(
                ecx,
                TxKind::Create,
                create.caller,
                init_code,
                create.gas_limit,
                create.value,
            );
            return Some(CreateOutcome { result, address });
        }

        None
    }

    fn eofcreate_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        call: &EOFCreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        // Inner context calls with depth 0 are being dispatched as top-level calls with depth 1.
        // Avoid processing twice.
        if self.in_inner_context && ecx.journaled_state.depth == 0 {
            return outcome;
        }

        let result = outcome.result.result;

        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.tracer, &mut self.cheatcodes, &mut self.printer],
            |inspector| {
                let new_outcome = inspector.eofcreate_end(ecx, call, outcome.clone());

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                let different = new_outcome.result.result != result ||
                    (new_outcome.result.result == InstructionResult::Revert &&
                        new_outcome.output() != outcome.output());
                different.then_some(new_outcome)
            },
            self,
            ecx
        );

        outcome
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        call_inspectors!([&mut self.tracer, &mut self.printer], |inspector| {
            Inspector::<DB>::selfdestruct(inspector, contract, target, value)
        });
    }
}

impl<'a, DB: DatabaseExt> InspectorExt<DB> for InspectorStackRefMut<'a> {
    fn should_use_create2_factory(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> bool {
        call_inspectors_adjust_depth!(
            #[ret]
            [&mut self.cheatcodes],
            |inspector| { inspector.should_use_create2_factory(ecx, inputs).then_some(true) },
            self,
            ecx
        );

        false
    }

    fn console_log(&mut self, input: String) {
        call_inspectors!([&mut self.log_collector], |inspector| InspectorExt::<DB>::console_log(
            inspector, input
        ));
    }

    fn is_alphanet(&self) -> bool {
        self.inner.alphanet
    }
}

impl<DB: DatabaseExt> Inspector<DB> for InspectorStack {
    #[inline]
    fn step(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        self.as_mut().step(interpreter, ecx)
    }

    #[inline]
    fn step_end(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        self.as_mut().step_end(interpreter, ecx)
    }

    fn call(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        self.as_mut().call(context, inputs)
    }

    fn call_end(
        &mut self,
        context: &mut EvmContext<DB>,
        inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        self.as_mut().call_end(context, inputs, outcome)
    }

    fn create(
        &mut self,
        context: &mut EvmContext<DB>,
        create: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        self.as_mut().create(context, create)
    }

    fn create_end(
        &mut self,
        context: &mut EvmContext<DB>,
        call: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        self.as_mut().create_end(context, call, outcome)
    }

    fn eofcreate(
        &mut self,
        context: &mut EvmContext<DB>,
        create: &mut EOFCreateInputs,
    ) -> Option<CreateOutcome> {
        self.as_mut().eofcreate(context, create)
    }

    fn eofcreate_end(
        &mut self,
        context: &mut EvmContext<DB>,
        call: &EOFCreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        self.as_mut().eofcreate_end(context, call, outcome)
    }

    fn initialize_interp(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        self.as_mut().initialize_interp(interpreter, ecx)
    }

    fn log(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>, log: &Log) {
        self.as_mut().log(interpreter, ecx, log)
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        Inspector::<DB>::selfdestruct(&mut self.as_mut(), contract, target, value)
    }
}

impl<DB: DatabaseExt> InspectorExt<DB> for InspectorStack {
    fn should_use_create2_factory(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> bool {
        self.as_mut().should_use_create2_factory(ecx, inputs)
    }

    fn is_alphanet(&self) -> bool {
        self.alphanet
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
