use super::{
    Cheatcodes, CheatsConfig, ChiselState, CoverageCollector, Debugger, Fuzzer, LogCollector,
    StackSnapshotType, TracePrinter, TracingInspector, TracingInspectorConfig,
};
use alloy_primitives::{Address, Bytes, Log, B256, U256};
use foundry_evm_core::{
    backend::DatabaseExt,
    debug::DebugArena,
    utils::{eval_to_instruction_result, halt_to_instruction_result},
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_traces::CallTraceArena;
use revm::{
    evm_inner,
    interpreter::{
        return_revert, CallInputs, CallScheme, CreateInputs, Gas, InstructionResult, Interpreter,
        Stack,
    },
    primitives::{BlockEnv, Env, ExecutionResult, Output, State, TransactTo},
    DatabaseCommit, EVMData, Inspector,
};
use std::{collections::HashMap, sync::Arc};

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
    pub trace: Option<bool>,
    /// Whether to enable the debugger.
    pub debug: Option<bool>,
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

    /// Set whether to enable the debugger.
    #[inline]
    pub fn debug(mut self, yes: bool) -> Self {
        self.debug = Some(yes);
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
    pub fn trace(mut self, yes: bool) -> Self {
        self.trace = Some(yes);
        self
    }

    /// Set whether to enable the call isolation.
    /// For description of call isolation, see [`InspectorStack::enable_isolation`].
    #[inline]
    pub fn enable_isolation(mut self, yes: bool) -> Self {
        self.enable_isolation = yes;
        self
    }

    /// Builds the stack of inspectors to use when transacting/committing on the EVM.
    ///
    /// See also [`revm::Evm::inspect_ref`] and [`revm::Evm::commit_ref`].
    pub fn build(self) -> InspectorStack {
        let Self {
            block,
            gas_price,
            cheatcodes,
            fuzzer,
            trace,
            debug,
            logs,
            coverage,
            print,
            chisel_state,
            enable_isolation,
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
        stack.enable_debugger(debug.unwrap_or(false));
        stack.print(print.unwrap_or(false));
        stack.tracing(trace.unwrap_or(false));

        stack.enable_isolation(enable_isolation);

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
    ([$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr $(,)?) => {{$(
        if let Some($id) = $inspector {
            $call
        }
    )+}}
}

/// Same as [call_inspectors] macro, but with depth adjustment for isolated execution.
macro_rules! call_inspectors_adjust_depth {
    ([$($inspector:expr),+ $(,)?], |$id:ident $(,)?| $call:expr, $self:ident, $data:ident $(,)?) => {
        if $self.in_inner_context {
            $data.journaled_state.depth += 1;
        }
        {$(
            if let Some($id) = $inspector {
                if let Some(result) = $call {
                    if $self.in_inner_context {
                        $data.journaled_state.depth -= 1;
                    }
                    return result;
                }
            }
        )+}
        if $self.in_inner_context {
            $data.journaled_state.depth -= 1;
        }
    }
}

/// Helper method which updates data in the state with the data from the database.
fn update_state<DB: DatabaseExt>(state: &mut State, db: &mut DB) {
    for (addr, acc) in state.iter_mut() {
        acc.info = db.basic(*addr).unwrap().unwrap_or_default();
        for (key, val) in acc.storage.iter_mut() {
            val.present_value = db.storage(*addr, *key).unwrap();
        }
    }
}

/// The collected results of [`InspectorStack`].
pub struct InspectorData {
    pub logs: Vec<Log>,
    pub labels: HashMap<Address, String>,
    pub traces: Option<CallTraceArena>,
    pub debug: Option<DebugArena>,
    pub coverage: Option<HitMaps>,
    pub cheatcodes: Option<Cheatcodes>,
    pub chisel_state: Option<(Stack, Vec<u8>, InstructionResult)>,
}

/// Contains data about the state of outer/main EVM which created and invoked the inner EVM context.
/// Used to adjust EVM state while in inner context.
///
/// We need this to avoid breaking changes due to EVM behavior differences in isolated vs
/// non-isolated mode. For descriptions and workarounds for those changes see: https://github.com/foundry-rs/foundry/pull/7186#issuecomment-1959102195
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
#[derive(Clone, Debug, Default)]
pub struct InspectorStack {
    pub cheatcodes: Option<Cheatcodes>,
    pub chisel_state: Option<ChiselState>,
    pub coverage: Option<CoverageCollector>,
    pub debugger: Option<Debugger>,
    pub fuzzer: Option<Fuzzer>,
    pub log_collector: Option<LogCollector>,
    pub printer: Option<TracePrinter>,
    pub tracer: Option<TracingInspector>,
    pub enable_isolation: bool,

    /// Flag marking if we are in the inner EVM context.
    pub in_inner_context: bool,
    pub inner_context_data: Option<InnerContextData>,
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

    /// Set whether to enable the debugger.
    #[inline]
    pub fn enable_debugger(&mut self, yes: bool) {
        self.debugger = yes.then(Default::default);
    }

    /// Set whether to enable call isolation.
    #[inline]
    pub fn enable_isolation(&mut self, yes: bool) {
        self.enable_isolation = yes;
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
    pub fn tracing(&mut self, yes: bool) {
        self.tracer = yes.then(|| {
            TracingInspector::new(TracingInspectorConfig {
                record_steps: false,
                record_memory_snapshots: false,
                record_stack_snapshots: StackSnapshotType::None,
                record_state_diff: false,
                exclude_precompile_calls: false,
                record_call_return_data: true,
                record_logs: true,
            })
        });
    }

    /// Collects all the data gathered during inspection into a single struct.
    #[inline]
    pub fn collect(self) -> InspectorData {
        InspectorData {
            logs: self.log_collector.map(|logs| logs.logs).unwrap_or_default(),
            labels: self
                .cheatcodes
                .as_ref()
                .map(|cheatcodes| {
                    cheatcodes.labels.clone().into_iter().map(|l| (l.0, l.1)).collect()
                })
                .unwrap_or_default(),
            traces: self.tracer.map(|tracer| tracer.get_traces().clone()),
            debug: self.debugger.map(|debugger| debugger.arena),
            coverage: self.coverage.map(|coverage| coverage.maps),
            cheatcodes: self.cheatcodes,
            chisel_state: self.chisel_state.and_then(|state| state.state),
        }
    }

    fn do_call_end<DB: DatabaseExt>(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors_adjust_depth!(
            [
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                let (new_status, new_gas, new_retdata) =
                    inspector.call_end(data, call, remaining_gas, status, retdata.clone());

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                if new_status != status ||
                    (new_status == InstructionResult::Revert && new_retdata != retdata)
                {
                    Some((new_status, new_gas, new_retdata))
                } else {
                    None
                }
            },
            self,
            data
        );
        (status, remaining_gas, retdata)
    }

    fn transact_inner<DB: DatabaseExt + DatabaseCommit>(
        &mut self,
        data: &mut EVMData<'_, DB>,
        transact_to: TransactTo,
        caller: Address,
        input: Bytes,
        gas_limit: u64,
        value: U256,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        data.db.commit(data.journaled_state.state.clone());

        let nonce = data
            .journaled_state
            .load_account(caller, data.db)
            .expect("failed to load caller")
            .0
            .info
            .nonce;

        let cached_env = data.env.clone();

        data.env.block.basefee = U256::ZERO;
        data.env.tx.caller = caller;
        data.env.tx.transact_to = transact_to.clone();
        data.env.tx.data = input;
        data.env.tx.value = value;
        data.env.tx.nonce = Some(nonce);
        // Add 21000 to the gas limit to account for the base cost of transaction.
        // We might have modified block gas limit earlier and revm will reject tx with gas limit >
        // block gas limit, so we adjust.
        data.env.tx.gas_limit = std::cmp::min(gas_limit + 21000, data.env.block.gas_limit.to());
        data.env.tx.gas_price = U256::ZERO;

        self.inner_context_data = Some(InnerContextData {
            sender: data.env.tx.caller,
            original_origin: cached_env.tx.caller,
            original_sender_nonce: nonce,
            is_create: matches!(transact_to, TransactTo::Create(_)),
        });
        self.in_inner_context = true;
        let res = evm_inner(data.env, data.db, Some(self)).transact();
        self.in_inner_context = false;
        self.inner_context_data = None;

        data.env.tx = cached_env.tx;
        data.env.block.basefee = cached_env.block.basefee;

        let mut gas = Gas::new(gas_limit);

        let Ok(mut res) = res else {
            // Should we match, encode and propagate error as a revert reason?
            return (InstructionResult::Revert, None, gas, Bytes::new());
        };

        // Commit changes after transaction
        data.db.commit(res.state.clone());

        // Update both states with new DB data after commit.
        update_state(&mut data.journaled_state.state, data.db);
        update_state(&mut res.state, data.db);

        // Merge transaction journal into the active journal.
        for (addr, acc) in res.state {
            if let Some(acc_mut) = data.journaled_state.state.get_mut(&addr) {
                acc_mut.status |= acc.status;
                for (key, val) in acc.storage {
                    if !acc_mut.storage.contains_key(&key) {
                        acc_mut.storage.insert(key, val);
                    }
                }
            } else {
                data.journaled_state.state.insert(addr, acc);
            }
        }

        match res.result {
            ExecutionResult::Success { reason, gas_used, gas_refunded, logs: _, output } => {
                gas.set_refund(gas_refunded as i64);
                gas.record_cost(gas_used);
                let address = match output {
                    Output::Create(_, address) => address,
                    Output::Call(_) => None,
                };
                (eval_to_instruction_result(reason), address, gas, output.into_data())
            }
            ExecutionResult::Halt { reason, gas_used } => {
                gas.record_cost(gas_used);
                (halt_to_instruction_result(reason), None, gas, Bytes::new())
            }
            ExecutionResult::Revert { gas_used, output } => {
                gas.record_cost(gas_used);
                (InstructionResult::Revert, None, gas, output)
            }
        }
    }

    /// Adjusts the EVM data for the inner EVM context.
    /// Should be called on the top-level call of inner context (depth == 0 &&
    /// self.in_inner_context) Decreases sender nonce for CALLs to keep backwards compatibility
    /// Updates tx.origin to the value before entering inner context
    fn adjust_evm_data_for_inner_context<DB: DatabaseExt>(&mut self, data: &mut EVMData<'_, DB>) {
        let inner_context_data =
            self.inner_context_data.as_ref().expect("should be called in inner context");
        let sender_acc = data
            .journaled_state
            .state
            .get_mut(&inner_context_data.sender)
            .expect("failed to load sender");
        if !inner_context_data.is_create {
            sender_acc.info.nonce = inner_context_data.original_sender_nonce;
        }
        data.env.tx.caller = inner_context_data.original_origin;
    }
}

impl<DB: DatabaseExt + DatabaseCommit> Inspector<DB> for InspectorStack {
    fn initialize_interp(&mut self, interpreter: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        let res = interpreter.instruction_result;
        call_inspectors_adjust_depth!(
            [
                &mut self.debugger,
                &mut self.coverage,
                &mut self.tracer,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                inspector.initialize_interp(interpreter, data);

                // Allow inspectors to exit early
                if interpreter.instruction_result != res {
                    Some(())
                } else {
                    None
                }
            },
            self,
            data
        );
    }

    fn step(&mut self, interpreter: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        let res = interpreter.instruction_result;
        call_inspectors_adjust_depth!(
            [
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                inspector.step(interpreter, data);

                // Allow inspectors to exit early
                if interpreter.instruction_result != res {
                    Some(())
                } else {
                    None
                }
            },
            self,
            data
        );
    }

    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &Address,
        topics: &[B256],
        data: &Bytes,
    ) {
        call_inspectors_adjust_depth!(
            [&mut self.tracer, &mut self.log_collector, &mut self.cheatcodes, &mut self.printer],
            |inspector| {
                inspector.log(evm_data, address, topics, data);
                None
            },
            self,
            evm_data
        );
    }

    fn step_end(&mut self, interpreter: &mut Interpreter<'_>, data: &mut EVMData<'_, DB>) {
        let res = interpreter.instruction_result;
        call_inspectors_adjust_depth!(
            [
                &mut self.debugger,
                &mut self.tracer,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer,
                &mut self.chisel_state
            ],
            |inspector| {
                inspector.step_end(interpreter, data);

                // Allow inspectors to exit early
                if interpreter.instruction_result != res {
                    Some(())
                } else {
                    None
                }
            },
            self,
            data
        );
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        if self.in_inner_context && data.journaled_state.depth == 0 {
            self.adjust_evm_data_for_inner_context(data);
            return (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new());
        }

        call_inspectors_adjust_depth!(
            [
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                let (status, gas, retdata) = inspector.call(data, call);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    Some((status, gas, retdata))
                } else {
                    None
                }
            },
            self,
            data
        );

        if self.enable_isolation &&
            call.context.scheme == CallScheme::Call &&
            !self.in_inner_context &&
            data.journaled_state.depth == 1
        {
            let (res, _, gas, output) = self.transact_inner(
                data,
                TransactTo::Call(call.contract),
                call.context.caller,
                call.input.clone(),
                call.gas_limit,
                call.transfer.value,
            );
            return (res, gas, output);
        }

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
    ) -> (InstructionResult, Gas, Bytes) {
        // Inner context calls with depth 0 are being dispatched as top-level calls with depth 1.
        // Avoid processing twice.
        if self.in_inner_context && data.journaled_state.depth == 0 {
            return (status, remaining_gas, retdata);
        }

        let res = self.do_call_end(data, call, remaining_gas, status, retdata);
        if matches!(res.0, return_revert!()) {
            // Encountered a revert, since cheatcodes may have altered the evm state in such a way
            // that violates some constraints, e.g. `deal`, we need to manually roll back on revert
            // before revm reverts the state itself
            if let Some(cheats) = self.cheatcodes.as_mut() {
                cheats.on_revert(data);
            }
        }

        res
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        if self.in_inner_context && data.journaled_state.depth == 0 {
            self.adjust_evm_data_for_inner_context(data);
            return (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new());
        }

        call_inspectors_adjust_depth!(
            [
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                let (status, addr, gas, retdata) = inspector.create(data, call);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    Some((status, addr, gas, retdata))
                } else {
                    None
                }
            },
            self,
            data
        );

        if self.enable_isolation && !self.in_inner_context && data.journaled_state.depth == 1 {
            return self.transact_inner(
                data,
                TransactTo::Create(call.scheme),
                call.caller,
                call.init_code.clone(),
                call.gas_limit,
                call.value,
            );
        }

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
        status: InstructionResult,
        address: Option<Address>,
        remaining_gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        // Inner context calls with depth 0 are being dispatched as top-level calls with depth 1.
        // Avoid processing twice.
        if self.in_inner_context && data.journaled_state.depth == 0 {
            return (status, address, remaining_gas, retdata);
        }
        call_inspectors_adjust_depth!(
            [
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            |inspector| {
                let (new_status, new_address, new_gas, new_retdata) = inspector.create_end(
                    data,
                    call,
                    status,
                    address,
                    remaining_gas,
                    retdata.clone(),
                );

                if new_status != status {
                    Some((new_status, new_address, new_gas, new_retdata))
                } else {
                    None
                }
            },
            self,
            data
        );

        (status, address, remaining_gas, retdata)
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        call_inspectors!(
            [
                &mut self.debugger,
                &mut self.tracer,
                &mut self.log_collector,
                &mut self.cheatcodes,
                &mut self.printer,
                &mut self.chisel_state
            ],
            |inspector| {
                Inspector::<DB>::selfdestruct(inspector, contract, target, value);
            }
        );
    }
}
