//! Cheatcode EVM inspector.

use crate::{
    CheatsConfig, CheatsCtxt, DynCheatcode, Error, Result,
    Vm::{self, AccountAccess},
    evm::{
        DealRecord, GasRecord, RecordAccess,
        mapping::{self, MappingSlots},
        mock::{MockCallDataContext, MockCallReturnData},
        prank::Prank,
    },
    inspector::utils::CommonCreateInput,
    script::{Broadcast, Wallets},
    test::{
        assume::AssumeNoRevert,
        expect::{
            self, ExpectedCallData, ExpectedCallTracker, ExpectedCallType, ExpectedCreate,
            ExpectedEmitTracker, ExpectedRevert, ExpectedRevertKind,
        },
        revert_handlers,
    },
    utils::IgnoredTraces,
};
use alloy_consensus::BlobTransactionSidecar;
use alloy_evm::eth::EthEvmContext;
use alloy_network::TransactionBuilder4844;
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, hex,
    map::{AddressHashMap, HashMap, HashSet},
};
use alloy_rpc_types::{
    AccessList,
    request::{TransactionInput, TransactionRequest},
};
use alloy_sol_types::{SolCall, SolInterface, SolValue};
use foundry_common::{SELECTOR_LEN, TransactionMaybeSigned, evm::Breakpoints};
use foundry_evm_core::{
    InspectorExt,
    abi::Vm::stopExpectSafeMemoryCall,
    backend::{DatabaseError, DatabaseExt, RevertDiagnostic},
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME},
    evm::{FoundryEvm, new_evm_with_existing_context},
};
use foundry_evm_traces::{
    TracingInspector, TracingInspectorConfig, identifier::SignaturesIdentifier,
};
use foundry_wallets::multi_wallet::MultiWallet;
use itertools::Itertools;
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use rand::Rng;
use revm::{
    Inspector, Journal,
    bytecode::opcode as op,
    context::{BlockEnv, JournalTr, LocalContext, TransactionType, result::EVMError},
    context_interface::{CreateScheme, transaction::SignedAuthorization},
    handler::FrameResult,
    interpreter::{
        CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome, FrameInput, Gas, Host,
        InstructionResult, Interpreter, InterpreterAction, InterpreterResult,
        interpreter_types::{Jumps, LoopControl, MemoryTr},
    },
    state::EvmStorageSlot,
};
use serde_json::Value;
use std::{
    cmp::max,
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::BufReader,
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

mod utils;

pub type Ecx<'a, 'b, 'c> = &'a mut EthEvmContext<&'b mut (dyn DatabaseExt + 'c)>;

/// Helper trait for obtaining complete [revm::Inspector] instance from mutable reference to
/// [Cheatcodes].
///
/// This is needed for cases when inspector itself needs mutable access to [Cheatcodes] state and
/// allows us to correctly execute arbitrary EVM frames from inside cheatcode implementations.
pub trait CheatcodesExecutor {
    /// Core trait method accepting mutable reference to [Cheatcodes] and returning
    /// [revm::Inspector].
    fn get_inspector<'a>(&'a mut self, cheats: &'a mut Cheatcodes) -> Box<dyn InspectorExt + 'a>;

    /// Obtains [FoundryEvm] instance and executes the given CREATE frame.
    fn exec_create(
        &mut self,
        inputs: CreateInputs,
        ccx: &mut CheatsCtxt,
    ) -> Result<CreateOutcome, EVMError<DatabaseError>> {
        with_evm(self, ccx, |evm| {
            evm.inner.ctx.journaled_state.depth += 1;

            let frame = FrameInput::Create(Box::new(inputs));

            let outcome = match evm.run_execution(frame)? {
                FrameResult::Call(_) => unreachable!(),
                FrameResult::Create(create) => create,
            };

            evm.inner.ctx.journaled_state.depth -= 1;

            Ok(outcome)
        })
    }

    fn console_log(&mut self, ccx: &mut CheatsCtxt, msg: &str) {
        self.get_inspector(ccx.state).console_log(msg);
    }

    /// Returns a mutable reference to the tracing inspector if it is available.
    fn tracing_inspector(&mut self) -> Option<&mut TracingInspector> {
        None
    }
}

/// Constructs [FoundryEvm] and runs a given closure with it.
fn with_evm<E, F, O>(
    executor: &mut E,
    ccx: &mut CheatsCtxt,
    f: F,
) -> Result<O, EVMError<DatabaseError>>
where
    E: CheatcodesExecutor + ?Sized,
    F: for<'a, 'b> FnOnce(
        &mut FoundryEvm<'a, &'b mut dyn InspectorExt>,
    ) -> Result<O, EVMError<DatabaseError>>,
{
    let mut inspector = executor.get_inspector(ccx.state);
    let error = std::mem::replace(&mut ccx.ecx.error, Ok(()));

    let ctx = EthEvmContext {
        block: ccx.ecx.block.clone(),
        cfg: ccx.ecx.cfg.clone(),
        tx: ccx.ecx.tx.clone(),
        journaled_state: Journal {
            inner: ccx.ecx.journaled_state.inner.clone(),
            database: &mut *ccx.ecx.journaled_state.database as &mut dyn DatabaseExt,
        },
        local: LocalContext::default(),
        chain: (),
        error,
    };

    let mut evm = new_evm_with_existing_context(ctx, &mut *inspector);

    let res = f(&mut evm)?;

    ccx.ecx.journaled_state.inner = evm.inner.ctx.journaled_state.inner;
    ccx.ecx.block = evm.inner.ctx.block;
    ccx.ecx.tx = evm.inner.ctx.tx;
    ccx.ecx.cfg = evm.inner.ctx.cfg;
    ccx.ecx.error = evm.inner.ctx.error;

    Ok(res)
}

/// Basic implementation of [CheatcodesExecutor] that simply returns the [Cheatcodes] instance as an
/// inspector.
#[derive(Debug, Default, Clone, Copy)]
struct TransparentCheatcodesExecutor;

impl CheatcodesExecutor for TransparentCheatcodesExecutor {
    fn get_inspector<'a>(&'a mut self, cheats: &'a mut Cheatcodes) -> Box<dyn InspectorExt + 'a> {
        Box::new(cheats)
    }
}

macro_rules! try_or_return {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => return,
        }
    };
}

/// Contains additional, test specific resources that should be kept for the duration of the test
#[derive(Debug, Default)]
pub struct TestContext {
    /// Buffered readers for files opened for reading (path => BufReader mapping)
    pub opened_read_files: HashMap<PathBuf, BufReader<File>>,
}

/// Every time we clone `Context`, we want it to be empty
impl Clone for TestContext {
    fn clone(&self) -> Self {
        Default::default()
    }
}

impl TestContext {
    /// Clears the context.
    #[inline]
    pub fn clear(&mut self) {
        self.opened_read_files.clear();
    }
}

/// Helps collecting transactions from different forks.
#[derive(Clone, Debug)]
pub struct BroadcastableTransaction {
    /// The optional RPC URL.
    pub rpc: Option<String>,
    /// The transaction to broadcast.
    pub transaction: TransactionMaybeSigned,
}

#[derive(Clone, Debug, Copy)]
pub struct RecordDebugStepInfo {
    /// The debug trace node index when the recording starts.
    pub start_node_idx: usize,
    /// The original tracer config when the recording starts.
    pub original_tracer_config: TracingInspectorConfig,
}

/// Holds gas metering state.
#[derive(Clone, Debug, Default)]
pub struct GasMetering {
    /// True if gas metering is paused.
    pub paused: bool,
    /// True if gas metering was resumed or reset during the test.
    /// Used to reconcile gas when frame ends (if spent less than refunded).
    pub touched: bool,
    /// True if gas metering should be reset to frame limit.
    pub reset: bool,
    /// Stores paused gas frames.
    pub paused_frames: Vec<Gas>,

    /// The group and name of the active snapshot.
    pub active_gas_snapshot: Option<(String, String)>,

    /// Cache of the amount of gas used in previous call.
    /// This is used by the `lastCallGas` cheatcode.
    pub last_call_gas: Option<crate::Vm::Gas>,

    /// True if gas recording is enabled.
    pub recording: bool,
    /// The gas used in the last frame.
    pub last_gas_used: u64,
    /// Gas records for the active snapshots.
    pub gas_records: Vec<GasRecord>,
}

impl GasMetering {
    /// Start the gas recording.
    pub fn start(&mut self) {
        self.recording = true;
    }

    /// Stop the gas recording.
    pub fn stop(&mut self) {
        self.recording = false;
    }

    /// Resume paused gas metering.
    pub fn resume(&mut self) {
        if self.paused {
            self.paused = false;
            self.touched = true;
        }
        self.paused_frames.clear();
    }

    /// Reset gas to limit.
    pub fn reset(&mut self) {
        self.paused = false;
        self.touched = true;
        self.reset = true;
        self.paused_frames.clear();
    }
}

/// Holds data about arbitrary storage.
#[derive(Clone, Debug, Default)]
pub struct ArbitraryStorage {
    /// Mapping of arbitrary storage addresses to generated values (slot, arbitrary value).
    /// (SLOADs return random value if storage slot wasn't accessed).
    /// Changed values are recorded and used to copy storage to different addresses.
    pub values: HashMap<Address, HashMap<U256, U256>>,
    /// Mapping of address with storage copied to arbitrary storage address source.
    pub copies: HashMap<Address, Address>,
    /// Address with storage slots that should be overwritten even if previously set.
    pub overwrites: HashSet<Address>,
}

impl ArbitraryStorage {
    /// Marks an address with arbitrary storage.
    pub fn mark_arbitrary(&mut self, address: &Address, overwrite: bool) {
        self.values.insert(*address, HashMap::default());
        if overwrite {
            self.overwrites.insert(*address);
        } else {
            self.overwrites.remove(address);
        }
    }

    /// Maps an address that copies storage with the arbitrary storage address.
    pub fn mark_copy(&mut self, from: &Address, to: &Address) {
        if self.values.contains_key(from) {
            self.copies.insert(*to, *from);
        }
    }

    /// Saves arbitrary storage value for a given address:
    /// - store value in changed values cache.
    /// - update account's storage with given value.
    pub fn save(&mut self, ecx: Ecx, address: Address, slot: U256, data: U256) {
        self.values.get_mut(&address).expect("missing arbitrary address entry").insert(slot, data);
        if let Ok(mut account) = ecx.journaled_state.load_account(address) {
            account.storage.insert(slot, EvmStorageSlot::new(data, 0));
        }
    }

    /// Copies arbitrary storage value from source address to the given target address:
    /// - if a value is present in arbitrary values cache, then update target storage and return
    ///   existing value.
    /// - if no value was yet generated for given slot, then save new value in cache and update both
    ///   source and target storages.
    pub fn copy(&mut self, ecx: Ecx, target: Address, slot: U256, new_value: U256) -> U256 {
        let source = self.copies.get(&target).expect("missing arbitrary copy target entry");
        let storage_cache = self.values.get_mut(source).expect("missing arbitrary source storage");
        let value = match storage_cache.get(&slot) {
            Some(value) => *value,
            None => {
                storage_cache.insert(slot, new_value);
                // Update source storage with new value.
                if let Ok(mut source_account) = ecx.journaled_state.load_account(*source) {
                    source_account.storage.insert(slot, EvmStorageSlot::new(new_value, 0));
                }
                new_value
            }
        };
        // Update target storage with new value.
        if let Ok(mut target_account) = ecx.journaled_state.load_account(target) {
            target_account.storage.insert(slot, EvmStorageSlot::new(value, 0));
        }
        value
    }
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

    /// Currently active EIP-7702 delegations that will be consumed when building the next
    /// transaction. Set by `vm.attachDelegation()` and consumed via `.take()` during
    /// transaction construction.
    pub active_delegations: Vec<SignedAuthorization>,

    /// The active EIP-4844 blob that will be attached to the next call.
    pub active_blob_sidecar: Option<BlobTransactionSidecar>,

    /// The gas price.
    ///
    /// Used in the cheatcode handler to overwrite the gas price separately from the gas price
    /// in the execution environment.
    pub gas_price: Option<u128>,

    /// Address labels
    pub labels: AddressHashMap<String>,

    /// Prank information, mapped to the call depth where pranks were added.
    pub pranks: BTreeMap<usize, Prank>,

    /// Expected revert information
    pub expected_revert: Option<ExpectedRevert>,

    /// Assume next call can revert and discard fuzz run if it does.
    pub assume_no_revert: Option<AssumeNoRevert>,

    /// Additional diagnostic for reverts
    pub fork_revert_diagnostic: Option<RevertDiagnostic>,

    /// Recorded storage reads and writes
    pub accesses: RecordAccess,

    /// Whether storage access recording is currently active
    pub recording_accesses: bool,

    /// Recorded account accesses (calls, creates) organized by relative call depth, where the
    /// topmost vector corresponds to accesses at the depth at which account access recording
    /// began. Each vector in the matrix represents a list of accesses at a specific call
    /// depth. Once that call context has ended, the last vector is removed from the matrix and
    /// merged into the previous vector.
    pub recorded_account_diffs_stack: Option<Vec<Vec<AccountAccess>>>,

    /// The information of the debug step recording.
    pub record_debug_steps_info: Option<RecordDebugStepInfo>,

    /// Recorded logs
    pub recorded_logs: Option<Vec<crate::Vm::Log>>,

    /// Mocked calls
    // **Note**: inner must a BTreeMap because of special `Ord` impl for `MockCallDataContext`
    pub mocked_calls: HashMap<Address, BTreeMap<MockCallDataContext, VecDeque<MockCallReturnData>>>,

    /// Mocked functions. Maps target address to be mocked to pair of (calldata, mock address).
    pub mocked_functions: HashMap<Address, HashMap<Bytes, Address>>,

    /// Expected calls
    pub expected_calls: ExpectedCallTracker,
    /// Expected emits
    pub expected_emits: ExpectedEmitTracker,
    /// Expected creates
    pub expected_creates: Vec<ExpectedCreate>,

    /// Map of context depths to memory offset ranges that may be written to within the call depth.
    pub allowed_mem_writes: HashMap<u64, Vec<Range<u64>>>,

    /// Current broadcasting information
    pub broadcast: Option<Broadcast>,

    /// Scripting based transactions
    pub broadcastable_transactions: BroadcastableTransactions,

    /// Current EIP-2930 access lists.
    pub access_list: Option<AccessList>,

    /// Additional, user configurable context this Inspector has access to when inspecting a call.
    pub config: Arc<CheatsConfig>,

    /// Test-scoped context holding data that needs to be reset every test run
    pub test_context: TestContext,

    /// Whether to commit FS changes such as file creations, writes and deletes.
    /// Used to prevent duplicate changes file executing non-committing calls.
    pub fs_commit: bool,

    /// Serialized JSON values.
    // **Note**: both must a BTreeMap to ensure the order of the keys is deterministic.
    pub serialized_jsons: BTreeMap<String, BTreeMap<String, Value>>,

    /// All recorded ETH `deal`s.
    pub eth_deals: Vec<DealRecord>,

    /// Gas metering state.
    pub gas_metering: GasMetering,

    /// Contains gas snapshots made over the course of a test suite.
    // **Note**: both must a BTreeMap to ensure the order of the keys is deterministic.
    pub gas_snapshots: BTreeMap<String, BTreeMap<String, String>>,

    /// Mapping slots.
    pub mapping_slots: Option<AddressHashMap<MappingSlots>>,

    /// The current program counter.
    pub pc: usize,
    /// Breakpoints supplied by the `breakpoint` cheatcode.
    /// `char -> (address, pc)`
    pub breakpoints: Breakpoints,

    /// Whether the next contract creation should be intercepted to return its initcode.
    pub intercept_next_create_call: bool,

    /// Optional cheatcodes `TestRunner`. Used for generating random values from uint and int
    /// strategies.
    test_runner: Option<TestRunner>,

    /// Ignored traces.
    pub ignored_traces: IgnoredTraces,

    /// Addresses with arbitrary storage.
    pub arbitrary_storage: Option<ArbitraryStorage>,

    /// Deprecated cheatcodes mapped to the reason. Used to report warnings on test results.
    pub deprecated: HashMap<&'static str, Option<&'static str>>,
    /// Unlocked wallets used in scripts and testing of scripts.
    pub wallets: Option<Wallets>,
    /// Signatures identifier for decoding events and functions
    pub signatures_identifier: Option<SignaturesIdentifier>,
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
            active_delegations: Default::default(),
            active_blob_sidecar: Default::default(),
            gas_price: Default::default(),
            pranks: Default::default(),
            expected_revert: Default::default(),
            assume_no_revert: Default::default(),
            fork_revert_diagnostic: Default::default(),
            accesses: Default::default(),
            recording_accesses: Default::default(),
            recorded_account_diffs_stack: Default::default(),
            recorded_logs: Default::default(),
            record_debug_steps_info: Default::default(),
            mocked_calls: Default::default(),
            mocked_functions: Default::default(),
            expected_calls: Default::default(),
            expected_emits: Default::default(),
            expected_creates: Default::default(),
            allowed_mem_writes: Default::default(),
            broadcast: Default::default(),
            broadcastable_transactions: Default::default(),
            access_list: Default::default(),
            test_context: Default::default(),
            serialized_jsons: Default::default(),
            eth_deals: Default::default(),
            gas_metering: Default::default(),
            gas_snapshots: Default::default(),
            mapping_slots: Default::default(),
            pc: Default::default(),
            breakpoints: Default::default(),
            intercept_next_create_call: Default::default(),
            test_runner: Default::default(),
            ignored_traces: Default::default(),
            arbitrary_storage: Default::default(),
            deprecated: Default::default(),
            wallets: Default::default(),
            signatures_identifier: SignaturesIdentifier::new(true).ok(),
        }
    }

    /// Returns the configured prank at given depth or the first prank configured at a lower depth.
    /// For example, if pranks configured for depth 1, 3 and 5, the prank for depth 4 is the one
    /// configured at depth 3.
    pub fn get_prank(&self, depth: usize) -> Option<&Prank> {
        self.pranks.range(..=depth).last().map(|(_, prank)| prank)
    }

    /// Returns the configured wallets if available, else creates a new instance.
    pub fn wallets(&mut self) -> &Wallets {
        self.wallets.get_or_insert_with(|| Wallets::new(MultiWallet::default(), None))
    }

    /// Sets the unlocked wallets.
    pub fn set_wallets(&mut self, wallets: Wallets) {
        self.wallets = Some(wallets);
    }

    /// Adds a delegation to the active delegations list.
    pub fn add_delegation(&mut self, authorization: SignedAuthorization) {
        self.active_delegations.push(authorization);
    }

    /// Decodes the input data and applies the cheatcode.
    fn apply_cheatcode(
        &mut self,
        ecx: Ecx,
        call: &CallInputs,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Result {
        // decode the cheatcode call
        let decoded = Vm::VmCalls::abi_decode(&call.input.bytes(ecx)).map_err(|e| {
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
        ecx.journaled_state.database.ensure_cheatcode_access_forking_mode(&caller)?;

        apply_dispatch(
            &decoded,
            &mut CheatsCtxt { state: self, ecx, gas_limit: call.gas_limit, caller },
            executor,
        )
    }

    /// Grants cheat code access for new contracts if the caller also has
    /// cheatcode access or the new contract is created in top most call.
    ///
    /// There may be cheatcodes in the constructor of the new contract, in order to allow them
    /// automatically we need to determine the new address.
    fn allow_cheatcodes_on_create(&self, ecx: Ecx, caller: Address, created_address: Address) {
        if ecx.journaled_state.depth <= 1
            || ecx.journaled_state.database.has_cheatcode_access(&caller)
        {
            ecx.journaled_state.database.allow_cheatcode_access(created_address);
        }
    }

    /// Apply EIP-2930 access list.
    ///
    /// If the transaction type is [TransactionType::Legacy] we need to upgrade it to
    /// [TransactionType::Eip2930] in order to use access lists. Other transaction types support
    /// access lists themselves.
    fn apply_accesslist(&mut self, ecx: Ecx) {
        if let Some(access_list) = &self.access_list {
            ecx.tx.access_list = access_list.clone();

            if ecx.tx.tx_type == TransactionType::Legacy as u8 {
                ecx.tx.tx_type = TransactionType::Eip2930 as u8;
            }
        }
    }

    /// Called when there was a revert.
    ///
    /// Cleanup any previously applied cheatcodes that altered the state in such a way that revm's
    /// revert would run into issues.
    pub fn on_revert(&mut self, ecx: Ecx) {
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

    pub fn call_with_executor(
        &mut self,
        ecx: Ecx,
        call: &mut CallInputs,
        executor: &mut dyn CheatcodesExecutor,
    ) -> Option<CallOutcome> {
        let gas = Gas::new(call.gas_limit);
        let curr_depth = ecx.journaled_state.depth();

        // At the root call to test function or script `run()`/`setUp()` functions, we are
        // decreasing sender nonce to ensure that it matches on-chain nonce once we start
        // broadcasting.
        if curr_depth == 0 {
            let sender = ecx.tx.caller;
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
                    });
                }
            };
            let prev = account.info.nonce;
            account.info.nonce = prev.saturating_sub(1);

            trace!(target: "cheatcodes", %sender, nonce=account.info.nonce, prev, "corrected nonce");
        }

        if call.target_address == CHEATCODE_ADDRESS {
            return match self.apply_cheatcode(ecx, call, executor) {
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

        if call.target_address == HARDHAT_CONSOLE_ADDRESS {
            return None;
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
                    *calldata == call.input.bytes(ecx)[..calldata.len()] &&
                    // The value matches, if provided
                    expected
                        .value.is_none_or(|value| Some(value) == call.transfer_value()) &&
                    // The gas matches, if provided
                    expected.gas.is_none_or(|gas| gas == call.gas_limit) &&
                    // The minimum gas matches, if provided
                    expected.min_gas.is_none_or(|min_gas| min_gas <= call.gas_limit)
                {
                    *actual_count += 1;
                }
            }
        }

        // Handle mocked calls
        if let Some(mocks) = self.mocked_calls.get_mut(&call.bytecode_address) {
            let ctx = MockCallDataContext {
                calldata: call.input.bytes(ecx),
                value: call.transfer_value(),
            };

            if let Some(return_data_queue) = match mocks.get_mut(&ctx) {
                Some(queue) => Some(queue),
                None => mocks
                    .iter_mut()
                    .find(|(mock, _)| {
                        call.input.bytes(ecx).get(..mock.calldata.len()) == Some(&mock.calldata[..])
                            && mock.value.is_none_or(|value| Some(value) == call.transfer_value())
                    })
                    .map(|(_, v)| v),
            } && let Some(return_data) = if return_data_queue.len() == 1 {
                // If the mocked calls stack has a single element in it, don't empty it
                return_data_queue.front().map(|x| x.to_owned())
            } else {
                // Else, we pop the front element
                return_data_queue.pop_front()
            } {
                return Some(CallOutcome {
                    result: InterpreterResult {
                        result: return_data.ret_type,
                        output: return_data.data,
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                });
            }
        }

        // Apply our prank
        if let Some(prank) = &self.get_prank(curr_depth) {
            // Apply delegate call, `call.caller`` will not equal `prank.prank_caller`
            if prank.delegate_call
                && curr_depth == prank.depth
                && let CallScheme::DelegateCall = call.scheme
            {
                call.target_address = prank.new_caller;
                call.caller = prank.new_caller;
                if let Some(new_origin) = prank.new_origin {
                    ecx.tx.caller = new_origin;
                }
            }

            if curr_depth >= prank.depth && call.caller == prank.prank_caller {
                let mut prank_applied = false;

                // At the target depth we set `msg.sender`
                if curr_depth == prank.depth {
                    call.caller = prank.new_caller;
                    prank_applied = true;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    ecx.tx.caller = new_origin;
                    prank_applied = true;
                }

                // If prank applied for first time, then update
                if prank_applied && let Some(applied_prank) = prank.first_time_applied() {
                    self.pranks.insert(curr_depth, applied_prank);
                }
            }
        }

        // Apply EIP-2930 access list
        self.apply_accesslist(ecx);

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast {
            // We only apply a broadcast *to a specific depth*.
            //
            // We do this because any subsequent contract calls *must* exist on chain and
            // we only want to grab *this* call, not internal ones
            if curr_depth == broadcast.depth && call.caller == broadcast.original_caller {
                // At the target depth we set `msg.sender` & tx.origin.
                // We are simulating the caller as being an EOA, so *both* must be set to the
                // broadcast.origin.
                ecx.tx.caller = broadcast.new_origin;

                call.caller = broadcast.new_origin;
                // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                // because we only need the from, to, value, and data. We can later change this
                // into 1559, in the cli package, relatively easily once we
                // know the target chain supports EIP-1559.
                if !call.is_static {
                    if let Err(err) = ecx.journaled_state.load_account(broadcast.new_origin) {
                        return Some(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: Error::encode(err),
                                gas,
                            },
                            memory_offset: call.return_memory_offset.clone(),
                        });
                    }

                    let is_fixed_gas_limit = check_if_fixed_gas_limit(&ecx, call.gas_limit);

                    let input = TransactionInput::new(call.input.bytes(ecx));

                    let account =
                        ecx.journaled_state.inner.state().get_mut(&broadcast.new_origin).unwrap();

                    let mut tx_req = TransactionRequest {
                        from: Some(broadcast.new_origin),
                        to: Some(TxKind::from(Some(call.target_address))),
                        value: call.transfer_value(),
                        input,
                        nonce: Some(account.info.nonce),
                        chain_id: Some(ecx.cfg.chain_id),
                        gas: if is_fixed_gas_limit { Some(call.gas_limit) } else { None },
                        ..Default::default()
                    };

                    let active_delegations = std::mem::take(&mut self.active_delegations);
                    // Set active blob sidecar, if any.
                    if let Some(blob_sidecar) = self.active_blob_sidecar.take() {
                        // Ensure blob and delegation are not set for the same tx.
                        if !active_delegations.is_empty() {
                            let msg = "both delegation and blob are active; `attachBlob` and `attachDelegation` are not compatible";
                            return Some(CallOutcome {
                                result: InterpreterResult {
                                    result: InstructionResult::Revert,
                                    output: Error::encode(msg),
                                    gas,
                                },
                                memory_offset: call.return_memory_offset.clone(),
                            });
                        }
                        tx_req.set_blob_sidecar(blob_sidecar);
                    }

                    // Apply active EIP-7702 delegations, if any.
                    if !active_delegations.is_empty() {
                        for auth in &active_delegations {
                            let Ok(authority) = auth.recover_authority() else {
                                continue;
                            };
                            if authority == broadcast.new_origin {
                                // Increment nonce of broadcasting account to reflect signed
                                // authorization.
                                account.info.nonce += 1;
                            }
                        }
                        tx_req.authorization_list = Some(active_delegations);
                    }

                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc: ecx.journaled_state.database.active_fork_url(),
                        transaction: tx_req.into(),
                    });
                    debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable call");

                    // Explicitly increment nonce if calls are not isolated.
                    if !self.config.evm_opts.isolate {
                        let prev = account.info.nonce;
                        account.info.nonce += 1;
                        debug!(target: "cheatcodes", address=%broadcast.new_origin, nonce=prev+1, prev, "incremented nonce");
                    }
                } else if broadcast.single_call {
                    let msg = "`staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead";
                    return Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: Error::encode(msg),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                    });
                }
            }
        }

        // Record called accounts if `startStateDiffRecording` has been called
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // Determine if account is "initialized," ie, it has a non-zero balance, a non-zero
            // nonce, a non-zero KECCAK_EMPTY codehash, or non-empty code
            let initialized;
            let old_balance;
            if let Ok(acc) = ecx.journaled_state.load_account(call.target_address) {
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
                    forkId: ecx.journaled_state.db().active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.cfg.chain_id),
                },
                accessor: call.caller,
                account: call.bytecode_address,
                kind,
                initialized,
                oldBalance: old_balance,
                newBalance: U256::ZERO, // updated on call_end
                value: call.call_value(),
                data: call.input.bytes(ecx),
                reverted: false,
                deployedCode: Bytes::new(),
                storageAccesses: vec![], // updated on step
                depth: ecx
                    .journaled_state
                    .depth()
                    .try_into()
                    .expect("journaled state depth exceeds u64"),
            }]);
        }

        None
    }

    pub fn rng(&mut self) -> &mut impl Rng {
        self.test_runner().rng()
    }

    pub fn test_runner(&mut self) -> &mut TestRunner {
        self.test_runner.get_or_insert_with(|| match self.config.seed {
            Some(seed) => TestRunner::new_with_rng(
                proptest::test_runner::Config::default(),
                TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>()),
            ),
            None => TestRunner::new(proptest::test_runner::Config::default()),
        })
    }

    pub fn set_seed(&mut self, seed: U256) {
        self.test_runner = Some(TestRunner::new_with_rng(
            proptest::test_runner::Config::default(),
            TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>()),
        ));
    }

    /// Returns existing or set a default `ArbitraryStorage` option.
    /// Used by `setArbitraryStorage` cheatcode to track addresses with arbitrary storage.
    pub fn arbitrary_storage(&mut self) -> &mut ArbitraryStorage {
        self.arbitrary_storage.get_or_insert_with(ArbitraryStorage::default)
    }

    /// Whether the given address has arbitrary storage.
    pub fn has_arbitrary_storage(&self, address: &Address) -> bool {
        match &self.arbitrary_storage {
            Some(storage) => storage.values.contains_key(address),
            None => false,
        }
    }

    /// Whether the given slot of address with arbitrary storage should be overwritten.
    /// True if address is marked as and overwrite and if no value was previously generated for
    /// given slot.
    pub fn should_overwrite_arbitrary_storage(
        &self,
        address: &Address,
        storage_slot: U256,
    ) -> bool {
        match &self.arbitrary_storage {
            Some(storage) => {
                storage.overwrites.contains(address)
                    && storage
                        .values
                        .get(address)
                        .and_then(|arbitrary_values| arbitrary_values.get(&storage_slot))
                        .is_none()
            }
            None => false,
        }
    }

    /// Whether the given address is a copy of an address with arbitrary storage.
    pub fn is_arbitrary_storage_copy(&self, address: &Address) -> bool {
        match &self.arbitrary_storage {
            Some(storage) => storage.copies.contains_key(address),
            None => false,
        }
    }
}

impl Inspector<EthEvmContext<&mut dyn DatabaseExt>> for Cheatcodes {
    #[inline]
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            ecx.block = block;
        }
        if let Some(gas_price) = self.gas_price.take() {
            ecx.tx.gas_price = gas_price;
        }

        // Record gas for current frame.
        if self.gas_metering.paused {
            self.gas_metering.paused_frames.push(interpreter.gas);
        }

        // `expectRevert`: track the max call depth during `expectRevert`
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = max(ecx.journaled_state.depth(), expected.max_depth);
        }
    }

    fn step(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        self.pc = interpreter.bytecode.pc();

        // `pauseGasMetering`: pause / resume interpreter gas.
        if self.gas_metering.paused {
            self.meter_gas(interpreter);
        }

        // `resetGasMetering`: reset interpreter gas.
        if self.gas_metering.reset {
            self.meter_gas_reset(interpreter);
        }

        // `record`: record storage reads and writes.
        if self.recording_accesses {
            self.record_accesses(interpreter);
        }

        // `startStateDiffRecording`: record granular ordered storage accesses.
        if self.recorded_account_diffs_stack.is_some() {
            self.record_state_diffs(interpreter, ecx);
        }

        // `expectSafeMemory`: check if the current opcode is allowed to interact with memory.
        if !self.allowed_mem_writes.is_empty() {
            self.check_mem_opcodes(
                interpreter,
                ecx.journaled_state.depth().try_into().expect("journaled state depth exceeds u64"),
            );
        }

        // `startMappingRecording`: record SSTORE and KECCAK256.
        if let Some(mapping_slots) = &mut self.mapping_slots {
            mapping::step(mapping_slots, interpreter);
        }

        // `snapshotGas*`: take a snapshot of the current gas.
        if self.gas_metering.recording {
            self.meter_gas_record(interpreter, ecx);
        }
    }

    fn step_end(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        if self.gas_metering.paused {
            self.meter_gas_end(interpreter);
        }

        if self.gas_metering.touched {
            self.meter_gas_check(interpreter);
        }

        // `setArbitraryStorage` and `copyStorage`: add arbitrary values to storage.
        if self.arbitrary_storage.is_some() {
            self.arbitrary_storage_end(interpreter, ecx);
        }
    }

    fn log(&mut self, interpreter: &mut Interpreter, _ecx: Ecx, log: Log) {
        if !self.expected_emits.is_empty() {
            expect::handle_expect_emit(self, &log, interpreter);
        }

        // `recordLogs`
        if let Some(storage_recorded_logs) = &mut self.recorded_logs {
            storage_recorded_logs.push(Vm::Log {
                topics: log.data.topics().to_vec(),
                data: log.data.data.clone(),
                emitter: log.address,
            });
        }
    }

    fn call(&mut self, ecx: Ecx, inputs: &mut CallInputs) -> Option<CallOutcome> {
        Self::call_with_executor(self, ecx, inputs, &mut TransparentCheatcodesExecutor)
    }

    fn call_end(&mut self, ecx: Ecx, call: &CallInputs, outcome: &mut CallOutcome) {
        let cheatcode_call = call.target_address == CHEATCODE_ADDRESS
            || call.target_address == HARDHAT_CONSOLE_ADDRESS;

        // Clean up pranks/broadcasts if it's not a cheatcode call end. We shouldn't do
        // it for cheatcode calls because they are not applied for cheatcodes in the `call` hook.
        // This should be placed before the revert handling, because we might exit early there
        if !cheatcode_call {
            // Clean up pranks
            let curr_depth = ecx.journaled_state.depth();
            if let Some(prank) = &self.get_prank(curr_depth)
                && curr_depth == prank.depth
            {
                ecx.tx.caller = prank.prank_origin;

                // Clean single-call prank once we have returned to the original depth
                if prank.single_call {
                    self.pranks.remove(&curr_depth);
                }
            }

            // Clean up broadcast
            if let Some(broadcast) = &self.broadcast
                && curr_depth == broadcast.depth
            {
                ecx.tx.caller = broadcast.original_origin;

                // Clean single-call broadcast once we have returned to the original depth
                if broadcast.single_call {
                    let _ = self.broadcast.take();
                }
            }
        }

        // Handle assume no revert cheatcode.
        if let Some(assume_no_revert) = &mut self.assume_no_revert {
            // Record current reverter address before processing the expect revert if call reverted,
            // expect revert is set with expected reverter address and no actual reverter set yet.
            if outcome.result.is_revert() && assume_no_revert.reverted_by.is_none() {
                assume_no_revert.reverted_by = Some(call.target_address);
            }

            // allow multiple cheatcode calls at the same depth
            let curr_depth = ecx.journaled_state.depth();
            if curr_depth <= assume_no_revert.depth && !cheatcode_call {
                // Discard run if we're at the same depth as cheatcode, call reverted, and no
                // specific reason was supplied
                if outcome.result.is_revert() {
                    let assume_no_revert = std::mem::take(&mut self.assume_no_revert).unwrap();
                    return match revert_handlers::handle_assume_no_revert(
                        &assume_no_revert,
                        outcome.result.result,
                        &outcome.result.output,
                        &self.config.available_artifacts,
                    ) {
                        // if result is Ok, it was an anticipated revert; return an "assume" error
                        // to reject this run
                        Ok(_) => {
                            outcome.result.output = Error::from(MAGIC_ASSUME).abi_encode().into();
                        }
                        // if result is Error, it was an unanticipated revert; should revert
                        // normally
                        Err(error) => {
                            trace!(expected=?assume_no_revert, ?error, status=?outcome.result.result, "Expected revert mismatch");
                            outcome.result.result = InstructionResult::Revert;
                            outcome.result.output = error.abi_encode().into();
                        }
                    };
                } else {
                    // Call didn't revert, reset `assume_no_revert` state.
                    self.assume_no_revert = None;
                }
            }
        }

        // Handle expected reverts.
        if let Some(expected_revert) = &mut self.expected_revert {
            // Record current reverter address and call scheme before processing the expect revert
            // if call reverted.
            if outcome.result.is_revert() {
                // Record current reverter address if expect revert is set with expected reverter
                // address and no actual reverter was set yet or if we're expecting more than one
                // revert.
                if expected_revert.reverter.is_some()
                    && (expected_revert.reverted_by.is_none() || expected_revert.count > 1)
                {
                    expected_revert.reverted_by = Some(call.target_address);
                }
            }

            let curr_depth = ecx.journaled_state.depth();
            if curr_depth <= expected_revert.depth {
                let needs_processing = match expected_revert.kind {
                    ExpectedRevertKind::Default => !cheatcode_call,
                    // `pending_processing` == true means that we're in the `call_end` hook for
                    // `vm.expectCheatcodeRevert` and shouldn't expect revert here
                    ExpectedRevertKind::Cheatcode { pending_processing } => {
                        cheatcode_call && !pending_processing
                    }
                };

                if needs_processing {
                    let mut expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                    return match revert_handlers::handle_expect_revert(
                        cheatcode_call,
                        false,
                        self.config.internal_expect_revert,
                        &expected_revert,
                        outcome.result.result,
                        outcome.result.output.clone(),
                        &self.config.available_artifacts,
                    ) {
                        Err(error) => {
                            trace!(expected=?expected_revert, ?error, status=?outcome.result.result, "Expected revert mismatch");
                            outcome.result.result = InstructionResult::Revert;
                            outcome.result.output = error.abi_encode().into();
                        }
                        Ok((_, retdata)) => {
                            expected_revert.actual_count += 1;
                            if expected_revert.actual_count < expected_revert.count {
                                self.expected_revert = Some(expected_revert.clone());
                            }
                            outcome.result.result = InstructionResult::Return;
                            outcome.result.output = retdata;
                        }
                    };
                }

                // Flip `pending_processing` flag for cheatcode revert expectations, marking that
                // we've exited the `expectCheatcodeRevert` call scope
                if let ExpectedRevertKind::Cheatcode { pending_processing } =
                    &mut self.expected_revert.as_mut().unwrap().kind
                {
                    *pending_processing = false;
                }
            }
        }

        // Exit early for calls to cheatcodes as other logic is not relevant for cheatcode
        // invocations
        if cheatcode_call {
            return;
        }

        // Record the gas usage of the call, this allows the `lastCallGas` cheatcode to
        // retrieve the gas usage of the last call.
        let gas = outcome.result.gas;
        self.gas_metering.last_call_gas = Some(crate::Vm::Gas {
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
            if ecx.journaled_state.depth() > 0
                && let Some(last_recorded_depth) = &mut recorded_account_diffs_stack.pop()
            {
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

                if let Some(call_access) = last_recorded_depth.first_mut() {
                    // Assert that we're at the correct depth before recording post-call state
                    // changes. Depending on the depth the cheat was
                    // called at, there may not be any pending
                    // calls to update if execution has percolated up to a higher depth.
                    let curr_depth = ecx.journaled_state.depth();
                    if call_access.depth == curr_depth as u64
                        && let Ok(acc) = ecx.journaled_state.load_account(call.target_address)
                    {
                        debug_assert!(access_is_call(call_access.kind));
                        call_access.newBalance = acc.info.balance;
                    }
                    // Merge the last depth's AccountAccesses into the AccountAccesses at the
                    // current depth, or push them back onto the pending
                    // vector if higher depths were not recorded. This
                    // preserves ordering of accesses.
                    if let Some(last) = recorded_account_diffs_stack.last_mut() {
                        last.append(last_recorded_depth);
                    } else {
                        recorded_account_diffs_stack.push(last_recorded_depth.clone());
                    }
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
            .any(|(expected, _)| {
                let curr_depth = ecx.journaled_state.depth();
                expected.depth == curr_depth
            }) &&
            // Ignore staticcalls
            !call.is_static;
        if should_check_emits {
            let expected_counts = self
                .expected_emits
                .iter()
                .filter_map(|(expected, count_map)| {
                    let count = match expected.address {
                        Some(emitter) => match count_map.get(&emitter) {
                            Some(log_count) => expected
                                .log
                                .as_ref()
                                .map(|l| log_count.count(l))
                                .unwrap_or_else(|| log_count.count_unchecked()),
                            None => 0,
                        },
                        None => match &expected.log {
                            Some(log) => count_map.values().map(|logs| logs.count(log)).sum(),
                            None => count_map.values().map(|logs| logs.count_unchecked()).sum(),
                        },
                    };

                    if count != expected.count { Some((expected, count)) } else { None }
                })
                .collect::<Vec<_>>();

            // Revert if not all emits expected were matched.
            if let Some((expected, _)) = self
                .expected_emits
                .iter()
                .find(|(expected, _)| !expected.found && expected.count > 0)
            {
                outcome.result.result = InstructionResult::Revert;
                let error_msg = expected.mismatch_error.as_deref().unwrap_or("log != expected log");
                outcome.result.output = error_msg.abi_encode().into();
                return;
            }

            if !expected_counts.is_empty() {
                let msg = if outcome.result.is_ok() {
                    let (expected, count) = expected_counts.first().unwrap();
                    format!("log emitted {count} times, expected {}", expected.count)
                } else {
                    "expected an emit, but the call reverted instead. \
                     ensure you're testing the happy path when using `expectEmit`"
                        .to_string()
                };

                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
                return;
            }

            // All emits were found, we're good.
            // Clear the queue, as we expect the user to declare more events for the next call
            // if they wanna match further events.
            self.expected_emits.clear()
        }

        // this will ensure we don't have false positives when trying to diagnose reverts in fork
        // mode
        let diag = self.fork_revert_diagnostic.take();

        // if there's a revert and a previous call was diagnosed as fork related revert then we can
        // return a better error here
        if outcome.result.is_revert()
            && let Some(err) = diag
        {
            outcome.result.output = Error::encode(err.to_error_msg(&self.labels));
            return;
        }

        // try to diagnose reverts in multi-fork mode where a call is made to an address that does
        // not exist
        if let TxKind::Call(test_contract) = ecx.tx.kind {
            // if a call to a different contract than the original test contract returned with
            // `Stop` we check if the contract actually exists on the active fork
            if ecx.journaled_state.db().is_forked_mode()
                && outcome.result.result == InstructionResult::Stop
                && call.target_address != test_contract
            {
                let journaled_state = ecx.journaled_state.clone();
                self.fork_revert_diagnostic =
                    ecx.journaled_state.db().diagnose_revert(call.target_address, &journaled_state);
            }
        }

        // If the depth is 0, then this is the root call terminating
        if ecx.journaled_state.depth() == 0 {
            // If we already have a revert, we shouldn't run the below logic as it can obfuscate an
            // earlier error that happened first with unrelated information about
            // another error when using cheatcodes.
            if outcome.result.is_revert() {
                return;
            }

            // If there's not a revert, we can continue on to run the last logic for expect*
            // cheatcodes.

            // Match expected calls
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

                        return;
                    }
                }
            }

            // Check if we have any leftover expected emits
            // First, if any emits were found at the root call, then we its ok and we remove them.
            self.expected_emits.retain(|(expected, _)| expected.count > 0 && !expected.found);
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
                return;
            }

            // Check for leftover expected creates
            if let Some(expected_create) = self.expected_creates.first() {
                let msg = format!(
                    "expected {} call by address {} for bytecode {} but not found",
                    expected_create.create_scheme,
                    hex::encode_prefixed(expected_create.deployer),
                    hex::encode_prefixed(&expected_create.bytecode),
                );
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
            }
        }
    }

    fn create(&mut self, ecx: Ecx, mut input: &mut CreateInputs) -> Option<CreateOutcome> {
        let gas = Gas::new(input.gas_limit());
        // Check if we should intercept this create
        if self.intercept_next_create_call {
            // Reset the flag
            self.intercept_next_create_call = false;

            // Get initcode from the input
            let output = input.init_code();

            // Return a revert with the initcode as error data
            return Some(CreateOutcome {
                result: InterpreterResult { result: InstructionResult::Revert, output, gas },
                address: None,
            });
        }

        let curr_depth = ecx.journaled_state.depth();

        // Apply our prank
        if let Some(prank) = &self.get_prank(curr_depth)
            && curr_depth >= prank.depth
            && input.caller() == prank.prank_caller
        {
            let mut prank_applied = false;

            // At the target depth we set `msg.sender`
            if curr_depth == prank.depth {
                input.set_caller(prank.new_caller);
                prank_applied = true;
            }

            // At the target depth, or deeper, we set `tx.origin`
            if let Some(new_origin) = prank.new_origin {
                ecx.tx.caller = new_origin;
                prank_applied = true;
            }

            // If prank applied for first time, then update
            if prank_applied && let Some(applied_prank) = prank.first_time_applied() {
                self.pranks.insert(curr_depth, applied_prank);
            }
        }

        // Apply EIP-2930 access list
        self.apply_accesslist(ecx);

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast
            && curr_depth >= broadcast.depth
            && input.caller() == broadcast.original_caller
        {
            if let Err(err) = ecx.journaled_state.load_account(broadcast.new_origin) {
                return Some(CreateOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: Error::encode(err),
                        gas,
                    },
                    address: None,
                });
            }

            ecx.tx.caller = broadcast.new_origin;

            if curr_depth == broadcast.depth {
                input.set_caller(broadcast.new_origin);
                let is_fixed_gas_limit = check_if_fixed_gas_limit(&ecx, input.gas_limit());

                let account = &ecx.journaled_state.inner.state()[&broadcast.new_origin];
                self.broadcastable_transactions.push_back(BroadcastableTransaction {
                    rpc: ecx.journaled_state.database.active_fork_url(),
                    transaction: TransactionRequest {
                        from: Some(broadcast.new_origin),
                        to: None,
                        value: Some(input.value()),
                        input: TransactionInput::new(input.init_code()),
                        nonce: Some(account.info.nonce),
                        gas: if is_fixed_gas_limit { Some(input.gas_limit()) } else { None },
                        ..Default::default()
                    }
                    .into(),
                });

                input.log_debug(self, &input.scheme().unwrap_or(CreateScheme::Create));
            }
        }

        // Allow cheatcodes from the address of the new contract
        let address = input.allow_cheatcodes(self, ecx);

        // If `recordAccountAccesses` has been called, record the create
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            recorded_account_diffs_stack.push(vec![AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: ecx.journaled_state.db().active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.cfg.chain_id),
                },
                accessor: input.caller(),
                account: address,
                kind: crate::Vm::AccountAccessKind::Create,
                initialized: true,
                oldBalance: U256::ZERO, // updated on create_end
                newBalance: U256::ZERO, // updated on create_end
                value: input.value(),
                data: input.init_code(),
                reverted: false,
                deployedCode: Bytes::new(), // updated on create_end
                storageAccesses: vec![],    // updated on create_end
                depth: curr_depth as u64,
            }]);
        }

        None
    }

    fn create_end(&mut self, ecx: Ecx, call: &CreateInputs, outcome: &mut CreateOutcome) {
        let call = Some(call);
        let curr_depth = ecx.journaled_state.depth();

        // Clean up pranks
        if let Some(prank) = &self.get_prank(curr_depth)
            && curr_depth == prank.depth
        {
            ecx.tx.caller = prank.prank_origin;

            // Clean single-call prank once we have returned to the original depth
            if prank.single_call {
                std::mem::take(&mut self.pranks);
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = &self.broadcast
            && curr_depth == broadcast.depth
        {
            ecx.tx.caller = broadcast.original_origin;

            // Clean single-call broadcast once we have returned to the original depth
            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert
            && curr_depth <= expected_revert.depth
            && matches!(expected_revert.kind, ExpectedRevertKind::Default)
        {
            let mut expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
            return match revert_handlers::handle_expect_revert(
                false,
                true,
                self.config.internal_expect_revert,
                &expected_revert,
                outcome.result.result,
                outcome.result.output.clone(),
                &self.config.available_artifacts,
            ) {
                Ok((address, retdata)) => {
                    expected_revert.actual_count += 1;
                    if expected_revert.actual_count < expected_revert.count {
                        self.expected_revert = Some(expected_revert.clone());
                    }

                    outcome.result.result = InstructionResult::Return;
                    outcome.result.output = retdata;
                    outcome.address = address;
                }
                Err(err) => {
                    outcome.result.result = InstructionResult::Revert;
                    outcome.result.output = err.abi_encode().into();
                }
            };
        }

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // The root call cannot be recorded.
            if curr_depth > 0
                && let Some(last_depth) = &mut recorded_account_diffs_stack.pop()
            {
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

                if let Some(create_access) = last_depth.first_mut() {
                    // Assert that we're at the correct depth before recording post-create state
                    // changes. Depending on what depth the cheat was called at, there
                    // may not be any pending calls to update if execution has
                    // percolated up to a higher depth.
                    let depth = ecx.journaled_state.depth();
                    if create_access.depth == depth as u64 {
                        debug_assert_eq!(
                            create_access.kind as u8,
                            crate::Vm::AccountAccessKind::Create as u8
                        );
                        if let Some(address) = outcome.address
                            && let Ok(created_acc) = ecx.journaled_state.load_account(address)
                        {
                            create_access.newBalance = created_acc.info.balance;
                            create_access.deployedCode =
                                created_acc.info.code.clone().unwrap_or_default().original_bytes();
                        }
                    }
                    // Merge the last depth's AccountAccesses into the AccountAccesses at the
                    // current depth, or push them back onto the pending
                    // vector if higher depths were not recorded. This
                    // preserves ordering of accesses.
                    if let Some(last) = recorded_account_diffs_stack.last_mut() {
                        last.append(last_depth);
                    } else {
                        recorded_account_diffs_stack.push(last_depth.clone());
                    }
                }
            }
        }

        // Match the create against expected_creates
        if !self.expected_creates.is_empty()
            && let (Some(address), Some(call)) = (outcome.address, call)
            && let Ok(created_acc) = ecx.journaled_state.load_account(address)
        {
            let bytecode = created_acc.info.code.clone().unwrap_or_default().original_bytes();
            if let Some((index, _)) =
                self.expected_creates.iter().find_position(|expected_create| {
                    expected_create.deployer == call.caller
                        && expected_create.create_scheme.eq(call.scheme.into())
                        && expected_create.bytecode == bytecode
                })
            {
                self.expected_creates.swap_remove(index);
            }
        }
    }
}

impl InspectorExt for Cheatcodes {
    fn should_use_create2_factory(&mut self, ecx: Ecx, inputs: &CreateInputs) -> bool {
        if let CreateScheme::Create2 { .. } = inputs.scheme {
            let depth = ecx.journaled_state.depth();
            let target_depth = if let Some(prank) = &self.get_prank(depth) {
                prank.depth
            } else if let Some(broadcast) = &self.broadcast {
                broadcast.depth
            } else {
                1
            };

            depth == target_depth
                && (self.broadcast.is_some() || self.config.always_use_create_2_factory)
        } else {
            false
        }
    }

    fn create2_deployer(&self) -> Address {
        self.config.evm_opts.create2_deployer
    }
}

impl Cheatcodes {
    #[cold]
    fn meter_gas(&mut self, interpreter: &mut Interpreter) {
        if let Some(paused_gas) = self.gas_metering.paused_frames.last() {
            // Keep gas constant if paused.
            // Make sure we record the memory changes so that memory expansion is not paused.
            let memory = *interpreter.gas.memory();
            interpreter.gas = *paused_gas;
            interpreter.gas.memory_mut().words_num = memory.words_num;
            interpreter.gas.memory_mut().expansion_cost = memory.expansion_cost;
        } else {
            // Record frame paused gas.
            self.gas_metering.paused_frames.push(interpreter.gas);
        }
    }

    #[cold]
    fn meter_gas_record(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        if interpreter.bytecode.action.as_ref().and_then(|i| i.instruction_result()).is_none() {
            self.gas_metering.gas_records.iter_mut().for_each(|record| {
                let curr_depth = ecx.journaled_state.depth();
                if curr_depth == record.depth {
                    // Skip the first opcode of the first call frame as it includes the gas cost of
                    // creating the snapshot.
                    if self.gas_metering.last_gas_used != 0 {
                        let gas_diff =
                            interpreter.gas.spent().saturating_sub(self.gas_metering.last_gas_used);
                        record.gas_used = record.gas_used.saturating_add(gas_diff);
                    }

                    // Update `last_gas_used` to the current spent gas for the next iteration to
                    // compare against.
                    self.gas_metering.last_gas_used = interpreter.gas.spent();
                }
            });
        }
    }

    #[cold]
    fn meter_gas_end(&mut self, interpreter: &mut Interpreter) {
        // Remove recorded gas if we exit frame.
        if let Some(interpreter_action) = interpreter.bytecode.action.as_ref()
            && will_exit(interpreter_action)
        {
            self.gas_metering.paused_frames.pop();
        }
    }

    #[cold]
    fn meter_gas_reset(&mut self, interpreter: &mut Interpreter) {
        interpreter.gas = Gas::new(interpreter.gas.limit());
        self.gas_metering.reset = false;
    }

    #[cold]
    fn meter_gas_check(&mut self, interpreter: &mut Interpreter) {
        if let Some(interpreter_action) = interpreter.bytecode.action.as_ref()
            && will_exit(interpreter_action)
        {
            // Reset gas if spent is less than refunded.
            // This can happen if gas was paused / resumed or reset.
            // https://github.com/foundry-rs/foundry/issues/4370
            if interpreter.gas.spent()
                < u64::try_from(interpreter.gas.refunded()).unwrap_or_default()
            {
                interpreter.gas = Gas::new(interpreter.gas.limit());
            }
        }
    }

    /// Generates or copies arbitrary values for storage slots.
    /// Invoked in inspector `step_end` (when the current opcode is not executed), if current opcode
    /// to execute is `SLOAD` and storage slot is cold.
    /// Ensures that in next step (when `SLOAD` opcode is executed) an arbitrary value is returned:
    /// - copies the existing arbitrary storage value (or the new generated one if no value in
    ///   cache) from mapped source address to the target address.
    /// - generates arbitrary value and saves it in target address storage.
    #[cold]
    fn arbitrary_storage_end(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        let (key, target_address) = if interpreter.bytecode.opcode() == op::SLOAD {
            (try_or_return!(interpreter.stack.peek(0)), interpreter.input.target_address)
        } else {
            return;
        };

        let Some(value) = ecx.sload(target_address, key) else {
            return;
        };

        if (value.is_cold && value.data.is_zero())
            || self.should_overwrite_arbitrary_storage(&target_address, key)
        {
            if self.has_arbitrary_storage(&target_address) {
                let arbitrary_value = self.rng().random();
                self.arbitrary_storage.as_mut().unwrap().save(
                    ecx,
                    target_address,
                    key,
                    arbitrary_value,
                );
            } else if self.is_arbitrary_storage_copy(&target_address) {
                let arbitrary_value = self.rng().random();
                self.arbitrary_storage.as_mut().unwrap().copy(
                    ecx,
                    target_address,
                    key,
                    arbitrary_value,
                );
            }
        }
    }

    /// Records storage slots reads and writes.
    #[cold]
    fn record_accesses(&mut self, interpreter: &mut Interpreter) {
        let access = &mut self.accesses;
        match interpreter.bytecode.opcode() {
            op::SLOAD => {
                let key = try_or_return!(interpreter.stack.peek(0));
                access.record_read(interpreter.input.target_address, key);
            }
            op::SSTORE => {
                let key = try_or_return!(interpreter.stack.peek(0));
                access.record_write(interpreter.input.target_address, key);
            }
            _ => {}
        }
    }

    #[cold]
    fn record_state_diffs(&mut self, interpreter: &mut Interpreter, ecx: Ecx) {
        let Some(account_accesses) = &mut self.recorded_account_diffs_stack else { return };
        match interpreter.bytecode.opcode() {
            op::SELFDESTRUCT => {
                // Ensure that we're not selfdestructing a context recording was initiated on
                let Some(last) = account_accesses.last_mut() else { return };

                // get previous balance and initialized status of the target account
                let target = try_or_return!(interpreter.stack.peek(0));
                let target = Address::from_word(B256::from(target));
                let (initialized, old_balance) = ecx
                    .journaled_state
                    .load_account(target)
                    .map(|account| (account.info.exists(), account.info.balance))
                    .unwrap_or_default();

                // load balance of this account
                let value = ecx
                    .balance(interpreter.input.target_address)
                    .map(|b| b.data)
                    .unwrap_or(U256::ZERO);

                // register access for the target account
                last.push(crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo {
                        forkId: ecx.journaled_state.database.active_fork_id().unwrap_or_default(),
                        chainId: U256::from(ecx.cfg.chain_id),
                    },
                    accessor: interpreter.input.target_address,
                    account: target,
                    kind: crate::Vm::AccountAccessKind::SelfDestruct,
                    initialized,
                    oldBalance: old_balance,
                    newBalance: old_balance + value,
                    value,
                    data: Bytes::new(),
                    reverted: false,
                    deployedCode: Bytes::new(),
                    storageAccesses: vec![],
                    depth: ecx
                        .journaled_state
                        .depth()
                        .try_into()
                        .expect("journaled state depth exceeds u64"),
                });
            }

            op::SLOAD => {
                let Some(last) = account_accesses.last_mut() else { return };

                let key = try_or_return!(interpreter.stack.peek(0));
                let address = interpreter.input.target_address;

                // Try to include present value for informational purposes, otherwise assume
                // it's not set (zero value)
                let mut present_value = U256::ZERO;
                // Try to load the account and the slot's present value
                if ecx.journaled_state.load_account(address).is_ok()
                    && let Some(previous) = ecx.sload(address, key)
                {
                    present_value = previous.data;
                }
                let access = crate::Vm::StorageAccess {
                    account: interpreter.input.target_address,
                    slot: key.into(),
                    isWrite: false,
                    previousValue: present_value.into(),
                    newValue: present_value.into(),
                    reverted: false,
                };
                let curr_depth = ecx
                    .journaled_state
                    .depth()
                    .try_into()
                    .expect("journaled state depth exceeds u64");
                append_storage_access(last, access, curr_depth);
            }
            op::SSTORE => {
                let Some(last) = account_accesses.last_mut() else { return };

                let key = try_or_return!(interpreter.stack.peek(0));
                let value = try_or_return!(interpreter.stack.peek(1));
                let address = interpreter.input.target_address;
                // Try to load the account and the slot's previous value, otherwise, assume it's
                // not set (zero value)
                let mut previous_value = U256::ZERO;
                if ecx.journaled_state.load_account(address).is_ok()
                    && let Some(previous) = ecx.sload(address, key)
                {
                    previous_value = previous.data;
                }

                let access = crate::Vm::StorageAccess {
                    account: address,
                    slot: key.into(),
                    isWrite: true,
                    previousValue: previous_value.into(),
                    newValue: value.into(),
                    reverted: false,
                };
                let curr_depth = ecx
                    .journaled_state
                    .depth()
                    .try_into()
                    .expect("journaled state depth exceeds u64");
                append_storage_access(last, access, curr_depth);
            }

            // Record account accesses via the EXT family of opcodes
            op::EXTCODECOPY | op::EXTCODESIZE | op::EXTCODEHASH | op::BALANCE => {
                let kind = match interpreter.bytecode.opcode() {
                    op::EXTCODECOPY => crate::Vm::AccountAccessKind::Extcodecopy,
                    op::EXTCODESIZE => crate::Vm::AccountAccessKind::Extcodesize,
                    op::EXTCODEHASH => crate::Vm::AccountAccessKind::Extcodehash,
                    op::BALANCE => crate::Vm::AccountAccessKind::Balance,
                    _ => unreachable!(),
                };
                let address =
                    Address::from_word(B256::from(try_or_return!(interpreter.stack.peek(0))));
                let initialized;
                let balance;
                if let Ok(acc) = ecx.journaled_state.load_account(address) {
                    initialized = acc.info.exists();
                    balance = acc.info.balance;
                } else {
                    initialized = false;
                    balance = U256::ZERO;
                }
                let curr_depth = ecx
                    .journaled_state
                    .depth()
                    .try_into()
                    .expect("journaled state depth exceeds u64");
                let account_access = crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo {
                        forkId: ecx.journaled_state.database.active_fork_id().unwrap_or_default(),
                        chainId: U256::from(ecx.cfg.chain_id),
                    },
                    accessor: interpreter.input.target_address,
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
                    depth: curr_depth,
                };
                // Record the EXT* call as an account access at the current depth
                // (future storage accesses will be recorded in a new "Resume" context)
                if let Some(last) = account_accesses.last_mut() {
                    last.push(account_access);
                } else {
                    account_accesses.push(vec![account_access]);
                }
            }
            _ => {}
        }
    }

    /// Checks to see if the current opcode can either mutate directly or expand memory.
    ///
    /// If the opcode at the current program counter is a match, check if the modified memory lies
    /// within the allowed ranges. If not, revert and fail the test.
    #[cold]
    fn check_mem_opcodes(&self, interpreter: &mut Interpreter, depth: u64) {
        let Some(ranges) = self.allowed_mem_writes.get(&depth) else {
            return;
        };

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
                match interpreter.bytecode.opcode() {
                    ////////////////////////////////////////////////////////////////
                    //    OPERATIONS THAT CAN EXPAND/MUTATE MEMORY BY WRITING     //
                    ////////////////////////////////////////////////////////////////

                    op::MSTORE => {
                        // The offset of the mstore operation is at the top of the stack.
                        let offset = try_or_return!(interpreter.stack.peek(0)).saturating_to::<u64>();

                        // If none of the allowed ranges contain [offset, offset + 32), memory has been
                        // unexpectedly mutated.
                        if !ranges.iter().any(|range| {
                            range.contains(&offset) && range.contains(&(offset + 31))
                        }) {
                            // SPECIAL CASE: When the compiler attempts to store the selector for
                            // `stopExpectSafeMemory`, this is allowed. It will do so at the current free memory
                            // pointer, which could have been updated to the exclusive upper bound during
                            // execution.
                            let value = try_or_return!(interpreter.stack.peek(1)).to_be_bytes::<32>();
                            if value[..SELECTOR_LEN] == stopExpectSafeMemoryCall::SELECTOR {
                                return
                            }

                            disallowed_mem_write(offset, 32, interpreter, ranges);
                            return
                        }
                    }
                    op::MSTORE8 => {
                        // The offset of the mstore8 operation is at the top of the stack.
                        let offset = try_or_return!(interpreter.stack.peek(0)).saturating_to::<u64>();

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

                    op::MLOAD => {
                        // The offset of the mload operation is at the top of the stack
                        let offset = try_or_return!(interpreter.stack.peek(0)).saturating_to::<u64>();

                        // If the offset being loaded is >= than the memory size, the
                        // memory is being expanded. If none of the allowed ranges contain
                        // [offset, offset + 32), memory has been unexpectedly mutated.
                        if offset >= interpreter.memory.size() as u64 && !ranges.iter().any(|range| {
                            range.contains(&offset) && range.contains(&(offset + 31))
                        }) {
                            disallowed_mem_write(offset, 32, interpreter, ranges);
                            return
                        }
                    }

                    ////////////////////////////////////////////////////////////////
                    //          OPERATIONS WITH OFFSET AND SIZE ON STACK          //
                    ////////////////////////////////////////////////////////////////

                    op::CALL => {
                        // The destination offset of the operation is the fifth element on the stack.
                        let dest_offset = try_or_return!(interpreter.stack.peek(5)).saturating_to::<u64>();

                        // The size of the data that will be copied is the sixth element on the stack.
                        let size = try_or_return!(interpreter.stack.peek(6)).saturating_to::<u64>();

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
                            let to = Address::from_word(try_or_return!(interpreter.stack.peek(1)).to_be_bytes::<32>().into());
                            if to == CHEATCODE_ADDRESS {
                                let args_offset = try_or_return!(interpreter.stack.peek(3)).saturating_to::<usize>();
                                let args_size = try_or_return!(interpreter.stack.peek(4)).saturating_to::<usize>();
                                let memory_word = interpreter.memory.slice_len(args_offset, args_size);
                                if memory_word[..SELECTOR_LEN] == stopExpectSafeMemoryCall::SELECTOR {
                                    return
                                }
                            }

                            disallowed_mem_write(dest_offset, size, interpreter, ranges);
                            return
                        }
                    }

                    $(op::$opcode => {
                        // The destination offset of the operation.
                        let dest_offset = try_or_return!(interpreter.stack.peek($offset_depth)).saturating_to::<u64>();

                        // The size of the data that will be copied.
                        let size = try_or_return!(interpreter.stack.peek($size_depth)).saturating_to::<u64>();

                        // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                        // memory outside of the expected ranges has been touched. If the opcode
                        // only reads from memory, this is okay as long as the memory is not expanded.
                        let fail_cond = !ranges.iter().any(|range| {
                                range.contains(&dest_offset) &&
                                    range.contains(&(dest_offset + size.saturating_sub(1)))
                            }) && ($writes ||
                                [dest_offset, (dest_offset + size).saturating_sub(1)].into_iter().any(|offset| {
                                    offset >= interpreter.memory.size() as u64
                                })
                            );

                        // If the failure condition is met, set the output buffer to a revert string
                        // that gives information about the allowed ranges and revert.
                        if fail_cond {
                            disallowed_mem_write(dest_offset, size, interpreter, ranges);
                            return
                        }
                    })*

                    _ => {}
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
        );
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

    interpreter.bytecode.set_action(InterpreterAction::new_return(
        InstructionResult::Revert,
        Bytes::from(revert_string.into_bytes()),
        interpreter.gas,
    ));
}

// Determines if the gas limit on a given call was manually set in the script and should therefore
// not be overwritten by later estimations
fn check_if_fixed_gas_limit(ecx: &Ecx, call_gas_limit: u64) -> bool {
    // If the gas limit was not set in the source code it is set to the estimated gas left at the
    // time of the call, which should be rather close to configured gas limit.
    // TODO: Find a way to reliably make this determination.
    // For example by generating it in the compilation or EVM simulation process
    ecx.tx.gas_limit > ecx.block.gas_limit &&
        call_gas_limit <= ecx.block.gas_limit
        // Transfers in forge scripts seem to be estimated at 2300 by revm leading to "Intrinsic
        // gas too low" failure when simulated on chain
        && call_gas_limit > 2300
}

/// Returns true if the kind of account access is a call.
fn access_is_call(kind: crate::Vm::AccountAccessKind) -> bool {
    matches!(
        kind,
        crate::Vm::AccountAccessKind::Call
            | crate::Vm::AccountAccessKind::StaticCall
            | crate::Vm::AccountAccessKind::CallCode
            | crate::Vm::AccountAccessKind::DelegateCall
    )
}

/// Appends an AccountAccess that resumes the recording of the current context.
fn append_storage_access(
    last: &mut Vec<AccountAccess>,
    storage_access: crate::Vm::StorageAccess,
    storage_depth: u64,
) {
    // Assert that there's an existing record for the current context.
    if !last.is_empty() && last.first().unwrap().depth < storage_depth {
        // Three cases to consider:
        // 1. If there hasn't been a context switch since the start of this context, then add the
        //    storage access to the current context record.
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

/// Dispatches the cheatcode call to the appropriate function.
fn apply_dispatch(
    calls: &Vm::VmCalls,
    ccx: &mut CheatsCtxt,
    executor: &mut dyn CheatcodesExecutor,
) -> Result {
    let cheat = calls_as_dyn_cheatcode(calls);

    let _guard = debug_span!(target: "cheatcodes", "apply", id = %cheat.id()).entered();
    trace!(target: "cheatcodes", cheat = ?cheat.as_debug(), "applying");

    if let spec::Status::Deprecated(replacement) = *cheat.status() {
        ccx.state.deprecated.insert(cheat.signature(), replacement);
    }

    // Apply the cheatcode.
    let mut result = cheat.dyn_apply(ccx, executor);

    // Format the error message to include the cheatcode name.
    if let Err(e) = &mut result
        && e.is_str()
    {
        let name = cheat.name();
        // Skip showing the cheatcode name for:
        // - assertions: too verbose, and can already be inferred from the error message
        // - `rpcUrl`: forge-std relies on it in `getChainWithUpdatedRpcUrl`
        if !name.contains("assert") && name != "rpcUrl" {
            *e = fmt_err!("vm.{name}: {e}");
        }
    }

    trace!(
        target: "cheatcodes",
        return = %match &result {
            Ok(b) => hex::encode(b),
            Err(e) => e.to_string(),
        }
    );

    result
}

fn calls_as_dyn_cheatcode(calls: &Vm::VmCalls) -> &dyn DynCheatcode {
    macro_rules! as_dyn {
        ($($variant:ident),*) => {
            match calls {
                $(Vm::VmCalls::$variant(cheat) => cheat,)*
            }
        };
    }
    vm_calls!(as_dyn)
}

/// Helper function to check if frame execution will exit.
fn will_exit(action: &InterpreterAction) -> bool {
    match action {
        InterpreterAction::Return(result) => {
            result.result.is_ok_or_revert() || result.result.is_error()
        }
        _ => false,
    }
}
