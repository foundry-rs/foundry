//! Cheatcode EVM [Inspector].

use crate::{
    evm::{
        mapping::{self, MappingSlots},
        mock::{MockCallDataContext, MockCallReturnData},
        prank::Prank,
        DealRecord, RecordAccess,
    },
    script::{Broadcast, ScriptWallets},
    test::expect::{
        self, ExpectedCallData, ExpectedCallTracker, ExpectedCallType, ExpectedEmit,
        ExpectedRevert, ExpectedRevertKind,
    },
    CheatsConfig, CheatsCtxt, DynCheatcode, Error, Result, Vm,
    Vm::AccountAccess,
};
use alloy_primitives::{Address, Bytes, Log, TxKind, B256, U256};
use alloy_rpc_types::request::{TransactionInput, TransactionRequest};
use alloy_sol_types::{SolInterface, SolValue};
use foundry_common::{evm::Breakpoints, SELECTOR_LEN};
use foundry_config::Config;
use foundry_evm_core::{
    abi::Vm::stopExpectSafeMemoryCall,
    backend::{DatabaseExt, RevertDiagnostic},
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    InspectorExt,
};
use itertools::Itertools;
use revm::{
    interpreter::{
        opcode, CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, Gas,
        InstructionResult, Interpreter, InterpreterAction, InterpreterResult,
    },
    primitives::{BlockEnv, CreateScheme, TransactTo},
    EvmContext, InnerEvmContext, Inspector,
};
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::File,
    io::BufReader,
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

macro_rules! try_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return,
        }
    };
}

/// Contains additional, test specific resources that should be kept for the duration of the test
#[derive(Debug, Default)]
pub struct Context {
    /// Buffered readers for files opened for reading (path => BufReader mapping)
    pub opened_read_files: HashMap<PathBuf, BufReader<File>>,
}

/// Every time we clone `Context`, we want it to be empty
impl Clone for Context {
    fn clone(&self) -> Self {
        Default::default()
    }
}

impl Context {
    /// Clears the context.
    #[inline]
    pub fn clear(&mut self) {
        self.opened_read_files.clear();
    }
}

/// Helps collecting transactions from different forks.
#[derive(Clone, Debug, Default)]
pub struct BroadcastableTransaction {
    /// The optional RPC URL.
    pub rpc: Option<String>,
    /// The transaction to broadcast.
    pub transaction: TransactionRequest,
}

/// List of transactions that can be broadcasted.
pub type BroadcastableTransactions = VecDeque<BroadcastableTransaction>;

/// An EVM inspector that handles calls to various cheatcodes, each with their own behavior.
///
/// Cheatcodes can be called by contracts during execution to modify the VM environment, such as
/// mocking addresses, signatures and altering call reverts.
///
/// Executing cheatcodes can be very powerful. Most cheatcodes are limited to evm internals, but
/// there are also cheatcodes like `ffi` which can execute arbitrary commands or `writeFile` and
/// `readFile` which can manipulate files of the filesystem. Therefore, several restrictions are
/// implemented for these cheatcodes:
/// - `ffi`, and file cheatcodes are _always_ opt-in (via foundry config) and never enabled by
///   default: all respective cheatcode handlers implement the appropriate checks
/// - File cheatcodes require explicit permissions which paths are allowed for which operation, see
///   `Config.fs_permission`
/// - Only permitted accounts are allowed to execute cheatcodes in forking mode, this ensures no
///   contract deployed on the live network is able to execute cheatcodes by simply calling the
///   cheatcode address: by default, the caller, test contract and newly deployed contracts are
///   allowed to execute cheatcodes
#[derive(Clone, Debug)]
pub struct Cheatcodes {
    /// The block environment
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: Option<BlockEnv>,

    /// The gas price
    ///
    /// Used in the cheatcode handler to overwrite the gas price separately from the gas price
    /// in the execution environment.
    pub gas_price: Option<U256>,

    /// Address labels
    pub labels: HashMap<Address, String>,

    /// Prank information
    pub prank: Option<Prank>,

    /// Expected revert information
    pub expected_revert: Option<ExpectedRevert>,

    /// Additional diagnostic for reverts
    pub fork_revert_diagnostic: Option<RevertDiagnostic>,

    /// Recorded storage reads and writes
    pub accesses: Option<RecordAccess>,

    /// Recorded account accesses (calls, creates) organized by relative call depth, where the
    /// topmost vector corresponds to accesses at the depth at which account access recording
    /// began. Each vector in the matrix represents a list of accesses at a specific call
    /// depth. Once that call context has ended, the last vector is removed from the matrix and
    /// merged into the previous vector.
    pub recorded_account_diffs_stack: Option<Vec<Vec<AccountAccess>>>,

    /// Recorded logs
    pub recorded_logs: Option<Vec<crate::Vm::Log>>,

    /// Cache of the amount of gas used in previous call.
    /// This is used by the `lastCallGas` cheatcode.
    pub last_call_gas: Option<crate::Vm::Gas>,

    /// Mocked calls
    // **Note**: inner must a BTreeMap because of special `Ord` impl for `MockCallDataContext`
    pub mocked_calls: HashMap<Address, BTreeMap<MockCallDataContext, MockCallReturnData>>,

    /// Expected calls
    pub expected_calls: ExpectedCallTracker,
    /// Expected emits
    pub expected_emits: VecDeque<ExpectedEmit>,

    /// Map of context depths to memory offset ranges that may be written to within the call depth.
    pub allowed_mem_writes: FxHashMap<u64, Vec<Range<u64>>>,

    /// Current broadcasting information
    pub broadcast: Option<Broadcast>,

    /// Scripting based transactions
    pub broadcastable_transactions: BroadcastableTransactions,

    /// Additional, user configurable context this Inspector has access to when inspecting a call
    pub config: Arc<CheatsConfig>,

    /// Test-scoped context holding data that needs to be reset every test run
    pub context: Context,

    /// Whether to commit FS changes such as file creations, writes and deletes.
    /// Used to prevent duplicate changes file executing non-committing calls.
    pub fs_commit: bool,

    /// Serialized JSON values.
    // **Note**: both must a BTreeMap to ensure the order of the keys is deterministic.
    pub serialized_jsons: BTreeMap<String, BTreeMap<String, Value>>,

    /// All recorded ETH `deal`s.
    pub eth_deals: Vec<DealRecord>,

    /// Holds the stored gas info for when we pause gas metering. It is an `Option<Option<..>>`
    /// because the `call` callback in an `Inspector` doesn't get access to
    /// the `revm::Interpreter` which holds the `revm::Gas` struct that
    /// we need to copy. So we convert it to a `Some(None)` in `apply_cheatcode`, and once we have
    /// the interpreter, we copy the gas struct. Then each time there is an execution of an
    /// operation, we reset the gas.
    pub gas_metering: Option<Option<Gas>>,

    /// Holds stored gas info for when we pause gas metering, and we're entering/inside
    /// CREATE / CREATE2 frames. This is needed to make gas meter pausing work correctly when
    /// paused and creating new contracts.
    pub gas_metering_create: Option<Option<Gas>>,

    /// Mapping slots.
    pub mapping_slots: Option<HashMap<Address, MappingSlots>>,

    /// The current program counter.
    pub pc: usize,
    /// Breakpoints supplied by the `breakpoint` cheatcode.
    /// `char -> (address, pc)`
    pub breakpoints: Breakpoints,
}

// This is not derived because calling this in `fn new` with `..Default::default()` creates a second
// `CheatsConfig` which is unused, and inside it `ProjectPathsConfig` is relatively expensive to
// create.
impl Default for Cheatcodes {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl Cheatcodes {
    /// Creates a new `Cheatcodes` with the given settings.
    pub fn new(config: Arc<CheatsConfig>) -> Self {
        Self {
            fs_commit: true,
            labels: config.labels.clone(),
            config,
            block: Default::default(),
            gas_price: Default::default(),
            prank: Default::default(),
            expected_revert: Default::default(),
            fork_revert_diagnostic: Default::default(),
            accesses: Default::default(),
            recorded_account_diffs_stack: Default::default(),
            recorded_logs: Default::default(),
            last_call_gas: Default::default(),
            mocked_calls: Default::default(),
            expected_calls: Default::default(),
            expected_emits: Default::default(),
            allowed_mem_writes: Default::default(),
            broadcast: Default::default(),
            broadcastable_transactions: Default::default(),
            context: Default::default(),
            serialized_jsons: Default::default(),
            eth_deals: Default::default(),
            gas_metering: Default::default(),
            gas_metering_create: Default::default(),
            mapping_slots: Default::default(),
            pc: Default::default(),
            breakpoints: Default::default(),
        }
    }

    /// Returns the configured script wallets.
    pub fn script_wallets(&self) -> Option<&ScriptWallets> {
        self.config.script_wallets.as_ref()
    }

    fn apply_cheatcode<DB: DatabaseExt>(
        &mut self,
        ecx: &mut EvmContext<DB>,
        call: &CallInputs,
    ) -> Result {
        // decode the cheatcode call
        let decoded = Vm::VmCalls::abi_decode(&call.input, false).map_err(|e| {
            if let alloy_sol_types::Error::UnknownSelector { name: _, selector } = e {
                let msg = format!(
                    "unknown cheatcode with selector {selector}; \
                     you may have a mismatch between the `Vm` interface (likely in `forge-std`) \
                     and the `forge` version"
                );
                return alloy_sol_types::Error::Other(std::borrow::Cow::Owned(msg));
            }
            e
        })?;
        let caller = call.caller;

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        ecx.db.ensure_cheatcode_access_forking_mode(&caller)?;

        apply_dispatch(
            &decoded,
            &mut CheatsCtxt {
                state: self,
                ecx: &mut ecx.inner,
                precompiles: &mut ecx.precompiles,
                caller,
            },
        )
    }

    /// Determines the address of the contract and marks it as allowed
    /// Returns the address of the contract created
    ///
    /// There may be cheatcodes in the constructor of the new contract, in order to allow them
    /// automatically we need to determine the new address
    fn allow_cheatcodes_on_create<DB: DatabaseExt>(
        &self,
        ecx: &mut InnerEvmContext<DB>,
        inputs: &CreateInputs,
    ) -> Address {
        let old_nonce = ecx
            .journaled_state
            .state
            .get(&inputs.caller)
            .map(|acc| acc.info.nonce)
            .unwrap_or_default();
        let created_address = inputs.created_address(old_nonce);

        if ecx.journaled_state.depth > 1 && !ecx.db.has_cheatcode_access(&inputs.caller) {
            // we only grant cheat code access for new contracts if the caller also has
            // cheatcode access and the new contract is created in top most call
            return created_address;
        }

        ecx.db.allow_cheatcode_access(created_address);

        created_address
    }

    /// Called when there was a revert.
    ///
    /// Cleanup any previously applied cheatcodes that altered the state in such a way that revm's
    /// revert would run into issues.
    pub fn on_revert<DB: DatabaseExt>(&mut self, ecx: &mut EvmContext<DB>) {
        trace!(deals=?self.eth_deals.len(), "rolling back deals");

        // Delay revert clean up until expected revert is handled, if set.
        if self.expected_revert.is_some() {
            return;
        }

        // we only want to apply cleanup top level
        if ecx.journaled_state.depth() > 0 {
            return;
        }

        // Roll back all previously applied deals
        // This will prevent overflow issues in revm's [`JournaledState::journal_revert`] routine
        // which rolls back any transfers.
        while let Some(record) = self.eth_deals.pop() {
            if let Some(acc) = ecx.journaled_state.state.get_mut(&record.address) {
                acc.info.balance = record.old_balance;
            }
        }
    }
}

impl<DB: DatabaseExt> Inspector<DB> for Cheatcodes {
    #[inline]
    fn initialize_interp(&mut self, _: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            ecx.env.block = block;
        }
        if let Some(gas_price) = self.gas_price.take() {
            ecx.env.tx.gas_price = gas_price;
        }
    }

    fn step(&mut self, interpreter: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        let ecx = &mut ecx.inner;
        self.pc = interpreter.program_counter();

        // reset gas if gas metering is turned off
        match self.gas_metering {
            Some(None) => {
                // need to store gas metering
                self.gas_metering = Some(Some(interpreter.gas));
            }
            Some(Some(gas)) => {
                match interpreter.current_opcode() {
                    opcode::CREATE | opcode::CREATE2 => {
                        // set we're about to enter CREATE frame to meter its gas on first opcode
                        // inside it
                        self.gas_metering_create = Some(None)
                    }
                    opcode::STOP | opcode::RETURN | opcode::SELFDESTRUCT | opcode::REVERT => {
                        // If we are ending current execution frame, we want to just fully reset gas
                        // otherwise weird things with returning gas from a call happen
                        // ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/evm_impl.rs#L190
                        //
                        // It would be nice if we had access to the interpreter in `call_end`, as we
                        // could just do this there instead.
                        match self.gas_metering_create {
                            None | Some(None) => {
                                interpreter.gas = Gas::new(0);
                            }
                            Some(Some(gas)) => {
                                // If this was CREATE frame, set correct gas limit. This is needed
                                // because CREATE opcodes deduct additional gas for code storage,
                                // and deducted amount is compared to gas limit. If we set this to
                                // 0, the CREATE would fail with out of gas.
                                //
                                // If we however set gas limit to the limit of outer frame, it would
                                // cause a panic after erasing gas cost post-create. Reason for this
                                // is pre-create REVM records `gas_limit - (gas_limit / 64)` as gas
                                // used, and erases costs by `remaining` gas post-create.
                                // gas used ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/instructions/host.rs#L254-L258
                                // post-create erase ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/instructions/host.rs#L279
                                interpreter.gas = Gas::new(gas.limit());

                                // reset CREATE gas metering because we're about to exit its frame
                                self.gas_metering_create = None
                            }
                        }
                    }
                    _ => {
                        // if just starting with CREATE opcodes, record its inner frame gas
                        if let Some(None) = self.gas_metering_create {
                            self.gas_metering_create = Some(Some(interpreter.gas))
                        }

                        // dont monitor gas changes, keep it constant
                        interpreter.gas = gas;
                    }
                }
            }
            _ => {}
        }

        // Record writes and reads if `record` has been called
        if let Some(storage_accesses) = &mut self.accesses {
            match interpreter.current_opcode() {
                opcode::SLOAD => {
                    let key = try_or_continue!(interpreter.stack().peek(0));
                    storage_accesses
                        .reads
                        .entry(interpreter.contract().target_address)
                        .or_default()
                        .push(key);
                }
                opcode::SSTORE => {
                    let key = try_or_continue!(interpreter.stack().peek(0));

                    // An SSTORE does an SLOAD internally
                    storage_accesses
                        .reads
                        .entry(interpreter.contract().target_address)
                        .or_default()
                        .push(key);
                    storage_accesses
                        .writes
                        .entry(interpreter.contract().target_address)
                        .or_default()
                        .push(key);
                }
                _ => (),
            }
        }

        // Record account access via SELFDESTRUCT if `recordAccountAccesses` has been called
        if let Some(account_accesses) = &mut self.recorded_account_diffs_stack {
            if interpreter.current_opcode() == opcode::SELFDESTRUCT {
                let target = try_or_continue!(interpreter.stack().peek(0));
                // load balance of this account
                let value = ecx
                    .balance(interpreter.contract().target_address)
                    .map(|(b, _)| b)
                    .unwrap_or(U256::ZERO);
                let account = Address::from_word(B256::from(target));
                // get previous balance and initialized status of the target account
                // TODO: use load_account_exists
                let (initialized, old_balance) = if let Ok((account, _)) =
                    ecx.journaled_state.load_account(account, &mut ecx.db)
                {
                    (account.info.exists(), account.info.balance)
                } else {
                    (false, U256::ZERO)
                };
                // register access for the target account
                let access = crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo {
                        forkId: ecx.db.active_fork_id().unwrap_or_default(),
                        chainId: U256::from(ecx.env.cfg.chain_id),
                    },
                    accessor: interpreter.contract().target_address,
                    account,
                    kind: crate::Vm::AccountAccessKind::SelfDestruct,
                    initialized,
                    oldBalance: old_balance,
                    newBalance: old_balance + value,
                    value,
                    data: Bytes::new(),
                    reverted: false,
                    deployedCode: Bytes::new(),
                    storageAccesses: vec![],
                    depth: ecx.journaled_state.depth(),
                };
                // Ensure that we're not selfdestructing a context recording was initiated on
                if let Some(last) = account_accesses.last_mut() {
                    last.push(access);
                }
            }
        }

        // Record granular ordered storage accesses if `startStateDiffRecording` has been called
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            match interpreter.current_opcode() {
                opcode::SLOAD => {
                    let key = try_or_continue!(interpreter.stack().peek(0));
                    let address = interpreter.contract().target_address;

                    // Try to include present value for informational purposes, otherwise assume
                    // it's not set (zero value)
                    let mut present_value = U256::ZERO;
                    // Try to load the account and the slot's present value
                    if ecx.load_account(address).is_ok() {
                        if let Ok((previous, _)) = ecx.sload(address, key) {
                            present_value = previous;
                        }
                    }
                    let access = crate::Vm::StorageAccess {
                        account: interpreter.contract().target_address,
                        slot: key.into(),
                        isWrite: false,
                        previousValue: present_value.into(),
                        newValue: present_value.into(),
                        reverted: false,
                    };
                    append_storage_access(
                        recorded_account_diffs_stack,
                        access,
                        ecx.journaled_state.depth(),
                    );
                }
                opcode::SSTORE => {
                    let key = try_or_continue!(interpreter.stack().peek(0));
                    let value = try_or_continue!(interpreter.stack().peek(1));
                    let address = interpreter.contract().target_address;
                    // Try to load the account and the slot's previous value, otherwise, assume it's
                    // not set (zero value)
                    let mut previous_value = U256::ZERO;
                    if ecx.load_account(address).is_ok() {
                        if let Ok((previous, _)) = ecx.sload(address, key) {
                            previous_value = previous;
                        }
                    }

                    let access = crate::Vm::StorageAccess {
                        account: address,
                        slot: key.into(),
                        isWrite: true,
                        previousValue: previous_value.into(),
                        newValue: value.into(),
                        reverted: false,
                    };
                    append_storage_access(
                        recorded_account_diffs_stack,
                        access,
                        ecx.journaled_state.depth(),
                    );
                }
                // Record account accesses via the EXT family of opcodes
                opcode::EXTCODECOPY |
                opcode::EXTCODESIZE |
                opcode::EXTCODEHASH |
                opcode::BALANCE => {
                    let kind = match interpreter.current_opcode() {
                        opcode::EXTCODECOPY => crate::Vm::AccountAccessKind::Extcodecopy,
                        opcode::EXTCODESIZE => crate::Vm::AccountAccessKind::Extcodesize,
                        opcode::EXTCODEHASH => crate::Vm::AccountAccessKind::Extcodehash,
                        opcode::BALANCE => crate::Vm::AccountAccessKind::Balance,
                        _ => unreachable!(),
                    };
                    let address = Address::from_word(B256::from(try_or_continue!(interpreter
                        .stack()
                        .peek(0))));
                    let balance;
                    let initialized;
                    // TODO: use ecx.load_account
                    if let Ok((acc, _)) = ecx.journaled_state.load_account(address, &mut ecx.db) {
                        initialized = acc.info.exists();
                        balance = acc.info.balance;
                    } else {
                        initialized = false;
                        balance = U256::ZERO;
                    }
                    let account_access = crate::Vm::AccountAccess {
                        chainInfo: crate::Vm::ChainInfo {
                            forkId: ecx.db.active_fork_id().unwrap_or_default(),
                            chainId: U256::from(ecx.env.cfg.chain_id),
                        },
                        accessor: interpreter.contract().target_address,
                        account: address,
                        kind,
                        initialized,
                        oldBalance: balance,
                        newBalance: balance,
                        value: U256::ZERO,
                        data: Bytes::new(),
                        reverted: false,
                        deployedCode: Bytes::new(),
                        storageAccesses: vec![],
                        depth: ecx.journaled_state.depth(),
                    };
                    // Record the EXT* call as an account access at the current depth
                    // (future storage accesses will be recorded in a new "Resume" context)
                    if let Some(last) = recorded_account_diffs_stack.last_mut() {
                        last.push(account_access);
                    } else {
                        recorded_account_diffs_stack.push(vec![account_access]);
                    }
                }
                _ => (),
            }
        }

        // If the allowed memory writes cheatcode is active at this context depth, check to see
        // if the current opcode can either mutate directly or expand memory. If the opcode at
        // the current program counter is a match, check if the modified memory lies within the
        // allowed ranges. If not, revert and fail the test.
        if let Some(ranges) = self.allowed_mem_writes.get(&ecx.journaled_state.depth()) {
            // The `mem_opcode_match` macro is used to match the current opcode against a list of
            // opcodes that can mutate memory (either directly or expansion via reading). If the
            // opcode is a match, the memory offsets that are being written to are checked to be
            // within the allowed ranges. If not, the test is failed and the transaction is
            // reverted. For all opcodes that can mutate memory aside from MSTORE,
            // MSTORE8, and MLOAD, the size and destination offset are on the stack, and
            // the macro expands all of these cases. For MSTORE, MSTORE8, and MLOAD, the
            // size of the memory write is implicit, so these cases are hard-coded.
            macro_rules! mem_opcode_match {
                ($(($opcode:ident, $offset_depth:expr, $size_depth:expr, $writes:expr)),* $(,)?) => {
                    match interpreter.current_opcode() {
                        ////////////////////////////////////////////////////////////////
                        //    OPERATIONS THAT CAN EXPAND/MUTATE MEMORY BY WRITING     //
                        ////////////////////////////////////////////////////////////////

                        opcode::MSTORE => {
                            // The offset of the mstore operation is at the top of the stack.
                            let offset = try_or_continue!(interpreter.stack().peek(0)).saturating_to::<u64>();

                            // If none of the allowed ranges contain [offset, offset + 32), memory has been
                            // unexpectedly mutated.
                            if !ranges.iter().any(|range| {
                                range.contains(&offset) && range.contains(&(offset + 31))
                            }) {
                                // SPECIAL CASE: When the compiler attempts to store the selector for
                                // `stopExpectSafeMemory`, this is allowed. It will do so at the current free memory
                                // pointer, which could have been updated to the exclusive upper bound during
                                // execution.
                                let value = try_or_continue!(interpreter.stack().peek(1)).to_be_bytes::<32>();
                                let selector = stopExpectSafeMemoryCall {}.cheatcode().func.selector_bytes;
                                if value[0..SELECTOR_LEN] == selector {
                                    return
                                }

                                disallowed_mem_write(offset, 32, interpreter, ranges);
                                return
                            }
                        }
                        opcode::MSTORE8 => {
                            // The offset of the mstore8 operation is at the top of the stack.
                            let offset = try_or_continue!(interpreter.stack().peek(0)).saturating_to::<u64>();

                            // If none of the allowed ranges contain the offset, memory has been
                            // unexpectedly mutated.
                            if !ranges.iter().any(|range| range.contains(&offset)) {
                                disallowed_mem_write(offset, 1, interpreter, ranges);
                                return
                            }
                        }

                        ////////////////////////////////////////////////////////////////
                        //        OPERATIONS THAT CAN EXPAND MEMORY BY READING        //
                        ////////////////////////////////////////////////////////////////

                        opcode::MLOAD => {
                            // The offset of the mload operation is at the top of the stack
                            let offset = try_or_continue!(interpreter.stack().peek(0)).saturating_to::<u64>();

                            // If the offset being loaded is >= than the memory size, the
                            // memory is being expanded. If none of the allowed ranges contain
                            // [offset, offset + 32), memory has been unexpectedly mutated.
                            if offset >= interpreter.shared_memory.len() as u64 && !ranges.iter().any(|range| {
                                range.contains(&offset) && range.contains(&(offset + 31))
                            }) {
                                disallowed_mem_write(offset, 32, interpreter, ranges);
                                return
                            }
                        }

                        ////////////////////////////////////////////////////////////////
                        //          OPERATIONS WITH OFFSET AND SIZE ON STACK          //
                        ////////////////////////////////////////////////////////////////

                        opcode::CALL => {
                            // The destination offset of the operation is the fifth element on the stack.
                            let dest_offset = try_or_continue!(interpreter.stack().peek(5)).saturating_to::<u64>();

                            // The size of the data that will be copied is the sixth element on the stack.
                            let size = try_or_continue!(interpreter.stack().peek(6)).saturating_to::<u64>();

                            // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                            // memory outside of the expected ranges has been touched. If the opcode
                            // only reads from memory, this is okay as long as the memory is not expanded.
                            let fail_cond = !ranges.iter().any(|range| {
                                range.contains(&dest_offset) &&
                                    range.contains(&(dest_offset + size.saturating_sub(1)))
                            });

                            // If the failure condition is met, set the output buffer to a revert string
                            // that gives information about the allowed ranges and revert.
                            if fail_cond {
                                // SPECIAL CASE: When a call to `stopExpectSafeMemory` is performed, this is allowed.
                                // It allocated calldata at the current free memory pointer, and will attempt to read
                                // from this memory region to perform the call.
                                let to = Address::from_word(try_or_continue!(interpreter.stack().peek(1)).to_be_bytes::<32>().into());
                                if to == CHEATCODE_ADDRESS {
                                    let args_offset = try_or_continue!(interpreter.stack().peek(3)).saturating_to::<usize>();
                                    let args_size = try_or_continue!(interpreter.stack().peek(4)).saturating_to::<usize>();
                                    let selector = stopExpectSafeMemoryCall {}.cheatcode().func.selector_bytes;
                                    let memory_word = interpreter.shared_memory.slice(args_offset, args_size);
                                    if memory_word[0..SELECTOR_LEN] == selector {
                                        return
                                    }
                                }

                                disallowed_mem_write(dest_offset, size, interpreter, ranges);
                                return
                            }
                        }

                        $(opcode::$opcode => {
                            // The destination offset of the operation.
                            let dest_offset = try_or_continue!(interpreter.stack().peek($offset_depth)).saturating_to::<u64>();

                            // The size of the data that will be copied.
                            let size = try_or_continue!(interpreter.stack().peek($size_depth)).saturating_to::<u64>();

                            // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                            // memory outside of the expected ranges has been touched. If the opcode
                            // only reads from memory, this is okay as long as the memory is not expanded.
                            let fail_cond = !ranges.iter().any(|range| {
                                    range.contains(&dest_offset) &&
                                        range.contains(&(dest_offset + size.saturating_sub(1)))
                                }) && ($writes ||
                                    [dest_offset, (dest_offset + size).saturating_sub(1)].into_iter().any(|offset| {
                                        offset >= interpreter.shared_memory.len() as u64
                                    })
                                );

                            // If the failure condition is met, set the output buffer to a revert string
                            // that gives information about the allowed ranges and revert.
                            if fail_cond {
                                disallowed_mem_write(dest_offset, size, interpreter, ranges);
                                return
                            }
                        })*
                        _ => ()
                    }
                }
            }

            // Check if the current opcode can write to memory, and if so, check if the memory
            // being written to is registered as safe to modify.
            mem_opcode_match!(
                (CALLDATACOPY, 0, 2, true),
                (CODECOPY, 0, 2, true),
                (RETURNDATACOPY, 0, 2, true),
                (EXTCODECOPY, 1, 3, true),
                (CALLCODE, 5, 6, true),
                (STATICCALL, 4, 5, true),
                (DELEGATECALL, 4, 5, true),
                (KECCAK256, 0, 1, false),
                (LOG0, 0, 1, false),
                (LOG1, 0, 1, false),
                (LOG2, 0, 1, false),
                (LOG3, 0, 1, false),
                (LOG4, 0, 1, false),
                (CREATE, 1, 2, false),
                (CREATE2, 1, 2, false),
                (RETURN, 0, 1, false),
                (REVERT, 0, 1, false),
            )
        }

        // Record writes with sstore (and sha3) if `StartMappingRecording` has been called
        if let Some(mapping_slots) = &mut self.mapping_slots {
            mapping::step(mapping_slots, interpreter);
        }
    }

    fn log(&mut self, _context: &mut EvmContext<DB>, log: &Log) {
        if !self.expected_emits.is_empty() {
            expect::handle_expect_emit(self, log);
        }

        // Stores this log if `recordLogs` has been called
        if let Some(storage_recorded_logs) = &mut self.recorded_logs {
            storage_recorded_logs.push(Vm::Log {
                topics: log.data.topics().to_vec(),
                data: log.data.data.clone(),
                emitter: log.address,
            });
        }
    }

    fn call(&mut self, ecx: &mut EvmContext<DB>, call: &mut CallInputs) -> Option<CallOutcome> {
        let gas = Gas::new(call.gas_limit);

        // At the root call to test function or script `run()`/`setUp()` functions, we are
        // decreasing sender nonce to ensure that it matches on-chain nonce once we start
        // broadcasting.
        if ecx.journaled_state.depth == 0 {
            let sender = ecx.env.tx.caller;
            if sender != Config::DEFAULT_SENDER {
                let account = match super::evm::journaled_account(ecx, sender) {
                    Ok(account) => account,
                    Err(err) => {
                        return Some(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: err.abi_encode().into(),
                                gas,
                            },
                            memory_offset: call.return_memory_offset.clone(),
                        })
                    }
                };
                let prev = account.info.nonce;
                account.info.nonce = prev.saturating_sub(1);

                debug!(target: "cheatcodes", %sender, nonce=account.info.nonce, prev, "corrected nonce");
            }
        }

        if call.target_address == CHEATCODE_ADDRESS {
            return match self.apply_cheatcode(ecx, call) {
                Ok(retdata) => Some(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Return,
                        output: retdata.into(),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                }),
                Err(err) => Some(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: err.abi_encode().into(),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                }),
            };
        }

        let ecx = &mut ecx.inner;

        if call.target_address == HARDHAT_CONSOLE_ADDRESS {
            return None
        }

        // Handle expected calls

        // Grab the different calldatas expected.
        if let Some(expected_calls_for_target) = self.expected_calls.get_mut(&call.bytecode_address)
        {
            // Match every partial/full calldata
            for (calldata, (expected, actual_count)) in expected_calls_for_target {
                // Increment actual times seen if...
                // The calldata is at most, as big as this call's input, and
                if calldata.len() <= call.input.len() &&
                    // Both calldata match, taking the length of the assumed smaller one (which will have at least the selector), and
                    *calldata == call.input[..calldata.len()] &&
                    // The value matches, if provided
                    expected
                        .value
                        .map_or(true, |value| Some(value) == call.transfer_value()) &&
                    // The gas matches, if provided
                    expected.gas.map_or(true, |gas| gas == call.gas_limit) &&
                    // The minimum gas matches, if provided
                    expected.min_gas.map_or(true, |min_gas| min_gas <= call.gas_limit)
                {
                    *actual_count += 1;
                }
            }
        }

        // Handle mocked calls
        if let Some(mocks) = self.mocked_calls.get(&call.target_address) {
            let ctx =
                MockCallDataContext { calldata: call.input.clone(), value: call.transfer_value() };
            if let Some(return_data) = mocks.get(&ctx).or_else(|| {
                mocks
                    .iter()
                    .find(|(mock, _)| {
                        call.input.get(..mock.calldata.len()) == Some(&mock.calldata[..]) &&
                            mock.value.map_or(true, |value| Some(value) == call.transfer_value())
                    })
                    .map(|(_, v)| v)
            }) {
                return Some(CallOutcome {
                    result: InterpreterResult {
                        result: return_data.ret_type,
                        output: return_data.data.clone(),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                })
            }
        }

        // Apply our prank
        if let Some(prank) = &self.prank {
            if ecx.journaled_state.depth() >= prank.depth && call.caller == prank.prank_caller {
                let mut prank_applied = false;

                // At the target depth we set `msg.sender`
                if ecx.journaled_state.depth() == prank.depth {
                    call.caller = prank.new_caller;
                    prank_applied = true;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    ecx.env.tx.caller = new_origin;
                    prank_applied = true;
                }

                // If prank applied for first time, then update
                if prank_applied {
                    if let Some(applied_prank) = prank.first_time_applied() {
                        self.prank = Some(applied_prank);
                    }
                }
            }
        }

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast {
            // We only apply a broadcast *to a specific depth*.
            //
            // We do this because any subsequent contract calls *must* exist on chain and
            // we only want to grab *this* call, not internal ones
            if ecx.journaled_state.depth() == broadcast.depth &&
                call.caller == broadcast.original_caller
            {
                // At the target depth we set `msg.sender` & tx.origin.
                // We are simulating the caller as being an EOA, so *both* must be set to the
                // broadcast.origin.
                ecx.env.tx.caller = broadcast.new_origin;

                call.caller = broadcast.new_origin;
                // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                // because we only need the from, to, value, and data. We can later change this
                // into 1559, in the cli package, relatively easily once we
                // know the target chain supports EIP-1559.
                if !call.is_static {
                    if let Err(err) = ecx.load_account(broadcast.new_origin) {
                        return Some(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: Error::encode(err),
                                gas,
                            },
                            memory_offset: call.return_memory_offset.clone(),
                        })
                    }

                    let is_fixed_gas_limit = check_if_fixed_gas_limit(ecx, call.gas_limit);

                    let account =
                        ecx.journaled_state.state().get_mut(&broadcast.new_origin).unwrap();

                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc: ecx.db.active_fork_url(),
                        transaction: TransactionRequest {
                            from: Some(broadcast.new_origin),
                            to: Some(TxKind::from(Some(call.target_address))),
                            value: call.transfer_value(),
                            input: TransactionInput::new(call.input.clone()),
                            nonce: Some(account.info.nonce),
                            gas: if is_fixed_gas_limit {
                                Some(call.gas_limit as u128)
                            } else {
                                None
                            },
                            ..Default::default()
                        },
                    });
                    debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable call");

                    let prev = account.info.nonce;

                    // Touch account to ensure that incremented nonce is committed
                    account.mark_touch();
                    account.info.nonce += 1;
                    debug!(target: "cheatcodes", address=%broadcast.new_origin, nonce=prev+1, prev, "incremented nonce");
                } else if broadcast.single_call {
                    let msg = "`staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead";
                    return Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: Error::encode(msg),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                    })
                }
            }
        }

        // Record called accounts if `startStateDiffRecording` has been called
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // Determine if account is "initialized," ie, it has a non-zero balance, a non-zero
            // nonce, a non-zero KECCAK_EMPTY codehash, or non-empty code
            let initialized;
            let old_balance;
            // TODO: use ecx.load_account
            if let Ok((acc, _)) = ecx.journaled_state.load_account(call.target_address, &mut ecx.db)
            {
                initialized = acc.info.exists();
                old_balance = acc.info.balance;
            } else {
                initialized = false;
                old_balance = U256::ZERO;
            }
            let kind = match call.scheme {
                CallScheme::Call => crate::Vm::AccountAccessKind::Call,
                CallScheme::CallCode => crate::Vm::AccountAccessKind::CallCode,
                CallScheme::DelegateCall => crate::Vm::AccountAccessKind::DelegateCall,
                CallScheme::StaticCall => crate::Vm::AccountAccessKind::StaticCall,
            };
            // Record this call by pushing it to a new pending vector; all subsequent calls at
            // that depth will be pushed to the same vector. When the call ends, the
            // RecordedAccountAccess (and all subsequent RecordedAccountAccesses) will be
            // updated with the revert status of this call, since the EVM does not mark accounts
            // as "warm" if the call from which they were accessed is reverted
            recorded_account_diffs_stack.push(vec![AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: ecx.db.active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.env.cfg.chain_id),
                },
                accessor: call.caller,
                account: call.bytecode_address,
                kind,
                initialized,
                oldBalance: old_balance,
                newBalance: U256::ZERO, // updated on call_end
                value: call.call_value(),
                data: call.input.clone(),
                reverted: false,
                deployedCode: Bytes::new(),
                storageAccesses: vec![], // updated on step
                depth: ecx.journaled_state.depth(),
            }]);
        }

        None
    }

    fn call_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        call: &CallInputs,
        mut outcome: CallOutcome,
    ) -> CallOutcome {
        let ecx = &mut ecx.inner;
        let cheatcode_call = call.target_address == CHEATCODE_ADDRESS ||
            call.target_address == HARDHAT_CONSOLE_ADDRESS;

        // Clean up pranks/broadcasts if it's not a cheatcode call end. We shouldn't do
        // it for cheatcode calls because they are not appplied for cheatcodes in the `call` hook.
        // This should be placed before the revert handling, because we might exit early there
        if !cheatcode_call {
            // Clean up pranks
            if let Some(prank) = &self.prank {
                if ecx.journaled_state.depth() == prank.depth {
                    ecx.env.tx.caller = prank.prank_origin;

                    // Clean single-call prank once we have returned to the original depth
                    if prank.single_call {
                        let _ = self.prank.take();
                    }
                }
            }

            // Clean up broadcast
            if let Some(broadcast) = &self.broadcast {
                if ecx.journaled_state.depth() == broadcast.depth {
                    ecx.env.tx.caller = broadcast.original_origin;

                    // Clean single-call broadcast once we have returned to the original depth
                    if broadcast.single_call {
                        let _ = self.broadcast.take();
                    }
                }
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if ecx.journaled_state.depth() <= expected_revert.depth {
                let needs_processing: bool = match expected_revert.kind {
                    ExpectedRevertKind::Default => !cheatcode_call,
                    // `pending_processing` == true means that we're in the `call_end` hook for
                    // `vm.expectCheatcodeRevert` and shouldn't expect revert here
                    ExpectedRevertKind::Cheatcode { pending_processing } => {
                        cheatcode_call && !pending_processing
                    }
                };

                if needs_processing {
                    let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                    return match expect::handle_expect_revert(
                        false,
                        expected_revert.reason.as_deref(),
                        outcome.result.result,
                        outcome.result.output.clone(),
                    ) {
                        Err(error) => {
                            trace!(expected=?expected_revert, ?error, status=?outcome.result.result, "Expected revert mismatch");
                            outcome.result.result = InstructionResult::Revert;
                            outcome.result.output = error.abi_encode().into();
                            outcome
                        }
                        Ok((_, retdata)) => {
                            outcome.result.result = InstructionResult::Return;
                            outcome.result.output = retdata;
                            outcome
                        }
                    };
                }

                // Flip `pending_processing` flag for cheatcode revert expectations, marking that
                // we've exited the `expectCheatcodeRevert` call scope
                if let ExpectedRevertKind::Cheatcode { pending_processing } =
                    &mut self.expected_revert.as_mut().unwrap().kind
                {
                    if *pending_processing {
                        *pending_processing = false;
                    }
                }
            }
        }

        // Exit early for calls to cheatcodes as other logic is not relevant for cheatcode
        // invocations
        if cheatcode_call {
            return outcome
        }

        // Record the gas usage of the call, this allows the `lastCallGas` cheatcode to
        // retrieve the gas usage of the last call.
        let gas = outcome.result.gas;
        self.last_call_gas = Some(crate::Vm::Gas {
            gasLimit: gas.limit(),
            gasTotalUsed: gas.spent(),
            gasMemoryUsed: 0,
            gasRefunded: gas.refunded(),
            gasRemaining: gas.remaining(),
        });

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // The root call cannot be recorded.
            if ecx.journaled_state.depth() > 0 {
                let mut last_recorded_depth =
                    recorded_account_diffs_stack.pop().expect("missing CALL account accesses");
                // Update the reverted status of all deeper calls if this call reverted, in
                // accordance with EVM behavior
                if outcome.result.is_revert() {
                    last_recorded_depth.iter_mut().for_each(|element| {
                        element.reverted = true;
                        element
                            .storageAccesses
                            .iter_mut()
                            .for_each(|storage_access| storage_access.reverted = true);
                    })
                }
                let call_access = last_recorded_depth.first_mut().expect("empty AccountAccesses");
                // Assert that we're at the correct depth before recording post-call state changes.
                // Depending on the depth the cheat was called at, there may not be any pending
                // calls to update if execution has percolated up to a higher depth.
                if call_access.depth == ecx.journaled_state.depth() {
                    // TODO: use ecx.load_account
                    if let Ok((acc, _)) =
                        ecx.journaled_state.load_account(call.target_address, &mut ecx.db)
                    {
                        debug_assert!(access_is_call(call_access.kind));
                        call_access.newBalance = acc.info.balance;
                    }
                }
                // Merge the last depth's AccountAccesses into the AccountAccesses at the current
                // depth, or push them back onto the pending vector if higher depths were not
                // recorded. This preserves ordering of accesses.
                if let Some(last) = recorded_account_diffs_stack.last_mut() {
                    last.append(&mut last_recorded_depth);
                } else {
                    recorded_account_diffs_stack.push(last_recorded_depth);
                }
            }
        }

        // At the end of the call,
        // we need to check if we've found all the emits.
        // We know we've found all the expected emits in the right order
        // if the queue is fully matched.
        // If it's not fully matched, then either:
        // 1. Not enough events were emitted (we'll know this because the amount of times we
        // inspected events will be less than the size of the queue) 2. The wrong events
        // were emitted (The inspected events should match the size of the queue, but still some
        // events will not be matched)

        // First, check that we're at the call depth where the emits were declared from.
        let should_check_emits = self
            .expected_emits
            .iter()
            .any(|expected| expected.depth == ecx.journaled_state.depth()) &&
            // Ignore staticcalls
            !call.is_static;
        if should_check_emits {
            // Not all emits were matched.
            if self.expected_emits.iter().any(|expected| !expected.found) {
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = "log != expected log".abi_encode().into();
                return outcome
            } else {
                // All emits were found, we're good.
                // Clear the queue, as we expect the user to declare more events for the next call
                // if they wanna match further events.
                self.expected_emits.clear()
            }
        }

        // this will ensure we don't have false positives when trying to diagnose reverts in fork
        // mode
        let diag = self.fork_revert_diagnostic.take();

        // if there's a revert and a previous call was diagnosed as fork related revert then we can
        // return a better error here
        if outcome.result.is_revert() {
            if let Some(err) = diag {
                outcome.result.output = Error::encode(err.to_error_msg(&self.labels));
                return outcome
            }
        }

        // try to diagnose reverts in multi-fork mode where a call is made to an address that does
        // not exist
        if let TransactTo::Call(test_contract) = ecx.env.tx.transact_to {
            // if a call to a different contract than the original test contract returned with
            // `Stop` we check if the contract actually exists on the active fork
            if ecx.db.is_forked_mode() &&
                outcome.result.result == InstructionResult::Stop &&
                call.target_address != test_contract
            {
                self.fork_revert_diagnostic =
                    ecx.db.diagnose_revert(call.target_address, &ecx.journaled_state);
            }
        }

        // If the depth is 0, then this is the root call terminating
        if ecx.journaled_state.depth() == 0 {
            // If we already have a revert, we shouldn't run the below logic as it can obfuscate an
            // earlier error that happened first with unrelated information about
            // another error when using cheatcodes.
            if outcome.result.is_revert() {
                return outcome;
            }

            // If there's not a revert, we can continue on to run the last logic for expect*
            // cheatcodes. Match expected calls
            for (address, calldatas) in &self.expected_calls {
                // Loop over each address, and for each address, loop over each calldata it expects.
                for (calldata, (expected, actual_count)) in calldatas {
                    // Grab the values we expect to see
                    let ExpectedCallData { gas, min_gas, value, count, call_type } = expected;

                    let failed = match call_type {
                        // If the cheatcode was called with a `count` argument,
                        // we must check that the EVM performed a CALL with this calldata exactly
                        // `count` times.
                        ExpectedCallType::Count => *count != *actual_count,
                        // If the cheatcode was called without a `count` argument,
                        // we must check that the EVM performed a CALL with this calldata at least
                        // `count` times. The amount of times to check was
                        // the amount of time the cheatcode was called.
                        ExpectedCallType::NonCount => *count > *actual_count,
                    };
                    if failed {
                        let expected_values = [
                            Some(format!("data {}", hex::encode_prefixed(calldata))),
                            value.as_ref().map(|v| format!("value {v}")),
                            gas.map(|g| format!("gas {g}")),
                            min_gas.map(|g| format!("minimum gas {g}")),
                        ]
                        .into_iter()
                        .flatten()
                        .join(", ");
                        let but = if outcome.result.is_ok() {
                            let s = if *actual_count == 1 { "" } else { "s" };
                            format!("was called {actual_count} time{s}")
                        } else {
                            "the call reverted instead; \
                             ensure you're testing the happy path when using `expectCall`"
                                .to_string()
                        };
                        let s = if *count == 1 { "" } else { "s" };
                        let msg = format!(
                            "expected call to {address} with {expected_values} \
                             to be called {count} time{s}, but {but}"
                        );
                        outcome.result.result = InstructionResult::Revert;
                        outcome.result.output = Error::encode(msg);

                        return outcome;
                    }
                }
            }

            // Check if we have any leftover expected emits
            // First, if any emits were found at the root call, then we its ok and we remove them.
            self.expected_emits.retain(|expected| !expected.found);
            // If not empty, we got mismatched emits
            if !self.expected_emits.is_empty() {
                let msg = if outcome.result.is_ok() {
                    "expected an emit, but no logs were emitted afterwards. \
                     you might have mismatched events or not enough events were emitted"
                } else {
                    "expected an emit, but the call reverted instead. \
                     ensure you're testing the happy path when using `expectEmit`"
                };
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
                return outcome;
            }
        }

        outcome
    }

    fn create(
        &mut self,
        ecx: &mut EvmContext<DB>,
        call: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        let ecx = &mut ecx.inner;
        let gas = Gas::new(call.gas_limit);

        // Apply our prank
        if let Some(prank) = &self.prank {
            if ecx.journaled_state.depth() >= prank.depth && call.caller == prank.prank_caller {
                // At the target depth we set `msg.sender`
                if ecx.journaled_state.depth() == prank.depth {
                    call.caller = prank.new_caller;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    ecx.env.tx.caller = new_origin;
                }
            }
        }

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast {
            if ecx.journaled_state.depth() >= broadcast.depth &&
                call.caller == broadcast.original_caller
            {
                if let Err(err) =
                    ecx.journaled_state.load_account(broadcast.new_origin, &mut ecx.db)
                {
                    return Some(CreateOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: Error::encode(err),
                            gas,
                        },
                        address: None,
                    })
                }

                ecx.env.tx.caller = broadcast.new_origin;

                if ecx.journaled_state.depth() == broadcast.depth {
                    call.caller = broadcast.new_origin;
                    let is_fixed_gas_limit = check_if_fixed_gas_limit(ecx, call.gas_limit);

                    let account = &ecx.journaled_state.state()[&broadcast.new_origin];

                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc: ecx.db.active_fork_url(),
                        transaction: TransactionRequest {
                            from: Some(broadcast.new_origin),
                            to: None,
                            value: Some(call.value),
                            input: TransactionInput::new(call.init_code.clone()),
                            nonce: Some(account.info.nonce),
                            gas: if is_fixed_gas_limit {
                                Some(call.gas_limit as u128)
                            } else {
                                None
                            },
                            ..Default::default()
                        },
                    });

                    let kind = match call.scheme {
                        CreateScheme::Create => "create",
                        CreateScheme::Create2 { .. } => "create2",
                    };
                    debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable {kind}");
                }
            }
        }

        // allow cheatcodes from the address of the new contract
        // Compute the address *after* any possible broadcast updates, so it's based on the updated
        // call inputs
        let address = self.allow_cheatcodes_on_create(ecx, call);
        // If `recordAccountAccesses` has been called, record the create
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // Record the create context as an account access and create a new vector to record all
            // subsequent account accesses
            recorded_account_diffs_stack.push(vec![AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: ecx.db.active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.env.cfg.chain_id),
                },
                accessor: call.caller,
                account: address,
                kind: crate::Vm::AccountAccessKind::Create,
                initialized: true,
                oldBalance: U256::ZERO, // updated on create_end
                newBalance: U256::ZERO, // updated on create_end
                value: call.value,
                data: call.init_code.clone(),
                reverted: false,
                deployedCode: Bytes::new(), // updated on create_end
                storageAccesses: vec![],    // updated on create_end
                depth: ecx.journaled_state.depth(),
            }]);
        }

        None
    }

    fn create_end(
        &mut self,
        ecx: &mut EvmContext<DB>,
        _call: &CreateInputs,
        mut outcome: CreateOutcome,
    ) -> CreateOutcome {
        let ecx = &mut ecx.inner;

        // Clean up pranks
        if let Some(prank) = &self.prank {
            if ecx.journaled_state.depth() == prank.depth {
                ecx.env.tx.caller = prank.prank_origin;

                // Clean single-call prank once we have returned to the original depth
                if prank.single_call {
                    std::mem::take(&mut self.prank);
                }
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = &self.broadcast {
            if ecx.journaled_state.depth() == broadcast.depth {
                ecx.env.tx.caller = broadcast.original_origin;

                // Clean single-call broadcast once we have returned to the original depth
                if broadcast.single_call {
                    std::mem::take(&mut self.broadcast);
                }
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if ecx.journaled_state.depth() <= expected_revert.depth &&
                matches!(expected_revert.kind, ExpectedRevertKind::Default)
            {
                let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match expect::handle_expect_revert(
                    true,
                    expected_revert.reason.as_deref(),
                    outcome.result.result,
                    outcome.result.output.clone(),
                ) {
                    Ok((address, retdata)) => {
                        outcome.result.result = InstructionResult::Return;
                        outcome.result.output = retdata;
                        outcome.address = address;
                        outcome
                    }
                    Err(err) => {
                        outcome.result.result = InstructionResult::Revert;
                        outcome.result.output = err.abi_encode().into();
                        outcome
                    }
                };
            }
        }

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // The root call cannot be recorded.
            if ecx.journaled_state.depth() > 0 {
                let mut last_depth =
                    recorded_account_diffs_stack.pop().expect("missing CREATE account accesses");
                // Update the reverted status of all deeper calls if this call reverted, in
                // accordance with EVM behavior
                if outcome.result.is_revert() {
                    last_depth.iter_mut().for_each(|element| {
                        element.reverted = true;
                        element
                            .storageAccesses
                            .iter_mut()
                            .for_each(|storage_access| storage_access.reverted = true);
                    })
                }
                let create_access = last_depth.first_mut().expect("empty AccountAccesses");
                // Assert that we're at the correct depth before recording post-create state
                // changes. Depending on what depth the cheat was called at, there
                // may not be any pending calls to update if execution has
                // percolated up to a higher depth.
                if create_access.depth == ecx.journaled_state.depth() {
                    debug_assert_eq!(
                        create_access.kind as u8,
                        crate::Vm::AccountAccessKind::Create as u8
                    );
                    if let Some(address) = outcome.address {
                        if let Ok((created_acc, _)) =
                            ecx.journaled_state.load_account(address, &mut ecx.db)
                        {
                            create_access.newBalance = created_acc.info.balance;
                            create_access.deployedCode =
                                created_acc.info.code.clone().unwrap_or_default().original_bytes();
                        }
                    }
                }
                // Merge the last depth's AccountAccesses into the AccountAccesses at the current
                // depth, or push them back onto the pending vector if higher depths were not
                // recorded. This preserves ordering of accesses.
                if let Some(last) = recorded_account_diffs_stack.last_mut() {
                    last.append(&mut last_depth);
                } else {
                    recorded_account_diffs_stack.push(last_depth);
                }
            }
        }

        outcome
    }
}

impl<DB: DatabaseExt> InspectorExt<DB> for Cheatcodes {
    fn should_use_create2_factory(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> bool {
        if let CreateScheme::Create2 { .. } = inputs.scheme {
            let target_depth = if let Some(prank) = &self.prank {
                prank.depth
            } else if let Some(broadcast) = &self.broadcast {
                broadcast.depth
            } else {
                1
            };

            ecx.journaled_state.depth() == target_depth &&
                (self.broadcast.is_some() || self.config.always_use_create_2_factory)
        } else {
            false
        }
    }
}

/// Helper that expands memory, stores a revert string pertaining to a disallowed memory write,
/// and sets the return range to the revert string's location in memory.
///
/// This will set the interpreter's next action to a return with the revert string as the output.
/// And trigger a revert.
fn disallowed_mem_write(
    dest_offset: u64,
    size: u64,
    interpreter: &mut Interpreter,
    ranges: &[Range<u64>],
) {
    let revert_string = format!(
        "memory write at offset 0x{:02X} of size 0x{:02X} not allowed; safe range: {}",
        dest_offset,
        size,
        ranges.iter().map(|r| format!("(0x{:02X}, 0x{:02X}]", r.start, r.end)).join(" U ")
    );

    interpreter.instruction_result = InstructionResult::Revert;
    interpreter.next_action = InterpreterAction::Return {
        result: InterpreterResult {
            output: Error::encode(revert_string),
            gas: interpreter.gas,
            result: InstructionResult::Revert,
        },
    };
}

// Determines if the gas limit on a given call was manually set in the script and should therefore
// not be overwritten by later estimations
fn check_if_fixed_gas_limit<DB: DatabaseExt>(
    ecx: &InnerEvmContext<DB>,
    call_gas_limit: u64,
) -> bool {
    // If the gas limit was not set in the source code it is set to the estimated gas left at the
    // time of the call, which should be rather close to configured gas limit.
    // TODO: Find a way to reliably make this determination.
    // For example by generating it in the compilation or EVM simulation process
    U256::from(ecx.env.tx.gas_limit) > ecx.env.block.gas_limit &&
        U256::from(call_gas_limit) <= ecx.env.block.gas_limit
        // Transfers in forge scripts seem to be estimated at 2300 by revm leading to "Intrinsic
        // gas too low" failure when simulated on chain
        && call_gas_limit > 2300
}

/// Dispatches the cheatcode call to the appropriate function.
fn apply_dispatch<DB: DatabaseExt>(calls: &Vm::VmCalls, ccx: &mut CheatsCtxt<DB>) -> Result {
    macro_rules! match_ {
        ($($variant:ident),*) => {
            match calls {
                $(Vm::VmCalls::$variant(cheat) => crate::Cheatcode::apply_traced(cheat, ccx),)*
            }
        };
    }
    vm_calls!(match_)
}

/// Returns true if the kind of account access is a call.
fn access_is_call(kind: crate::Vm::AccountAccessKind) -> bool {
    matches!(
        kind,
        crate::Vm::AccountAccessKind::Call |
            crate::Vm::AccountAccessKind::StaticCall |
            crate::Vm::AccountAccessKind::CallCode |
            crate::Vm::AccountAccessKind::DelegateCall
    )
}

/// Appends an AccountAccess that resumes the recording of the current context.
fn append_storage_access(
    accesses: &mut [Vec<AccountAccess>],
    storage_access: crate::Vm::StorageAccess,
    storage_depth: u64,
) {
    if let Some(last) = accesses.last_mut() {
        // Assert that there's an existing record for the current context.
        if !last.is_empty() && last.first().unwrap().depth < storage_depth {
            // Three cases to consider:
            // 1. If there hasn't been a context switch since the start of this context, then add
            //    the storage access to the current context record.
            // 2. If there's an existing Resume record, then add the storage access to it.
            // 3. Otherwise, create a new Resume record based on the current context.
            if last.len() == 1 {
                last.first_mut().unwrap().storageAccesses.push(storage_access);
            } else {
                let last_record = last.last_mut().unwrap();
                if last_record.kind as u8 == crate::Vm::AccountAccessKind::Resume as u8 {
                    last_record.storageAccesses.push(storage_access);
                } else {
                    let entry = last.first().unwrap();
                    let resume_record = crate::Vm::AccountAccess {
                        chainInfo: crate::Vm::ChainInfo {
                            forkId: entry.chainInfo.forkId,
                            chainId: entry.chainInfo.chainId,
                        },
                        accessor: entry.accessor,
                        account: entry.account,
                        kind: crate::Vm::AccountAccessKind::Resume,
                        initialized: entry.initialized,
                        storageAccesses: vec![storage_access],
                        reverted: entry.reverted,
                        // The remaining fields are defaults
                        oldBalance: U256::ZERO,
                        newBalance: U256::ZERO,
                        value: U256::ZERO,
                        data: Bytes::new(),
                        deployedCode: Bytes::new(),
                        depth: entry.depth,
                    };
                    last.push(resume_record);
                }
            }
        }
    }
}
