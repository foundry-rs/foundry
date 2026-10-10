//! Cheatcode EVM inspector.

use crate::{
    Cheatcode, CheatsConfig, CheatsCtxt, Error, Result,
    Vm::{self, AccountAccess},
    evm::{
        DealRecord, GasRecord, RecordAccess, append_storage_access, journaled_account,
        mark_account_accesses_reverted, merge_recorded_frame,
        mock::{self, MockCallDataContext, MockCallReturnData},
        prank::Prank,
    },
    expected_emit::{self, ExpectedEmitTracker},
    inspector::utils::CommonCreateInput,
    script::{Broadcast, Wallets},
    test::{
        assume::AssumeNoRevert,
        expect::{self, ExpectedCallTracker, ExpectedCreate, ExpectedRevert, ExpectedRevertKind},
        revert_handlers,
    },
    utils::IgnoredTraces,
};
use alloy_consensus::BlobTransactionSidecarVariant;
use alloy_network::{Ethereum, Network, TransactionBuilder};
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, hex,
    map::{AddressHashMap, HashMap, HashSet},
};
use alloy_rpc_types::AccessList;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, SolInterface};
use foundry_common::{
    FoundryTransactionBuilder, SELECTOR_LEN, TransactionMaybeSigned,
    mapping_slots::{
        MappingSlots, PendingMappingHash, capture_hash as capture_mapping_hash,
        record_hash as record_mapping_hash, step as mapping_step,
    },
};
use foundry_evm_core::{
    Breakpoints, EvmEnv, FoundryTransaction, InspectorExt,
    abi::Vm::stopExpectSafeMemoryCall,
    backend::{
        ContextUpdateFor, DatabaseError, DatabaseExt, JournaledState, LocalForkId, RevertDiagnostic,
    },
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME},
    env::FoundryContextExt,
    evm::{
        BlockEnvFor, ChainFor, EthEvmNetwork, EvmFactoryFor, FoundryContextFor, FoundryEvmFactory,
        FoundryEvmNetwork, NestedEvmClosureFor, SpecFor, TransactionRequestFor, TxEnvFor,
        with_inherited_evm,
    },
};
use foundry_evm_traces::{
    TracingInspector, TracingInspectorConfig, identifier::SignaturesIdentifier,
};
use foundry_wallets::wallet_multi::MultiWallet;
use itertools::Itertools;
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use rand::Rng;
use revm::{
    Inspector, JournalEntry,
    bytecode::opcode as op,
    context::{Cfg, ContextTr, Host, JournalTr, Transaction, TransactionType, result::EVMError},
    context_interface::{CreateScheme, transaction::SignedAuthorization},
    handler::FrameResult,
    interpreter::{
        CallInput, CallInputs, CallOutcome, CallScheme, CallValue, CreateInputs, CreateOutcome,
        FrameInput, Gas, InstructionResult, Interpreter, InterpreterAction, InterpreterResult,
        interpreter_types::{Jumps, LoopControl, MemoryTr, ReturnData},
    },
};
use serde_json::Value;
use std::{
    cmp::max,
    collections::{BTreeMap, VecDeque},
    fmt::Debug,
    fs::File,
    io::BufReader,
    ops::Range,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

mod env_overrides;
pub use env_overrides::EnvOverrideState;

mod utils;

pub mod analysis;
pub use analysis::CheatcodeAnalysis;

/// Helper trait for running nested EVM operations from inside cheatcode implementations.
pub trait CheatcodesExecutor<FEN: FoundryEvmNetwork> {
    /// Runs a closure with a nested EVM built from the current context.
    /// The inspector is assembled internally — never exposed to the caller.
    fn with_nested_evm(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        f: NestedEvmClosureFor<'_, FEN>,
    ) -> Result<(), EVMError<DatabaseError>>;

    /// Replays a historical transaction on the database. Inspector is assembled internally.
    fn transact_on_db(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<ContextUpdateFor<EvmFactoryFor<FEN>>>;

    /// Executes a `TransactionRequest` on the database. Inspector is assembled internally.
    fn transact_from_tx_on_db(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        tx: TxEnvFor<FEN>,
    ) -> eyre::Result<()>;

    /// Runs a closure with a fresh nested EVM using the current environment and database.
    /// Unlike `with_nested_evm`, this starts an independent journal and does not write back.
    /// The caller is responsible for state merging. Used by `executeTransactionCall`.
    /// Returns the final EVM environment after the closure runs (consumed without cloning).
    #[allow(clippy::type_complexity)]
    fn with_fresh_nested_evm(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        chain_context: ChainFor<FEN>,
        f: NestedEvmClosureFor<'_, FEN>,
    ) -> Result<EvmEnv<SpecFor<FEN>, BlockEnvFor<FEN>>, EVMError<DatabaseError>>;

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, msg: &str);

    /// Returns a mutable reference to the tracing inspector if it is available.
    fn tracing_inspector(&mut self) -> Option<&mut TracingInspector> {
        None
    }
}

/// Builds a sub-EVM from the current context and executes the given CREATE frame.
pub(crate) fn exec_create<FEN: FoundryEvmNetwork>(
    executor: &mut dyn CheatcodesExecutor<FEN>,
    inputs: CreateInputs,
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
) -> std::result::Result<CreateOutcome, EVMError<DatabaseError>> {
    let fee_token = ccx.tx_fee_token();
    let tx_origin = ccx.tx_caller();
    let mut inputs = Some(inputs);
    let mut outcome = None;
    executor.with_nested_evm(ccx.state, ccx.ecx, &mut |evm| {
        evm.tx_mut().set_fee_token(fee_token);
        evm.tx_mut().set_caller(tx_origin);
        let inputs = inputs.take().unwrap();
        evm.journal_inner_mut().depth += 1;

        let frame = FrameInput::Create(Box::new(inputs));

        let result = match evm.run_execution(frame)? {
            FrameResult::Call(_) => unreachable!(),
            FrameResult::Create(create) => create,
        };

        evm.journal_inner_mut().depth -= 1;

        outcome = Some(result);
        Ok(())
    })?;
    Ok(outcome.unwrap())
}

/// Basic implementation of [CheatcodesExecutor] that simply returns the [Cheatcodes] instance as an
/// inspector.
#[derive(Debug, Default, Clone, Copy)]
struct TransparentCheatcodesExecutor;

impl<FEN: FoundryEvmNetwork> CheatcodesExecutor<FEN> for TransparentCheatcodesExecutor {
    fn with_nested_evm(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        f: NestedEvmClosureFor<'_, FEN>,
    ) -> Result<(), EVMError<DatabaseError>> {
        with_inherited_evm::<FEN::EvmFactory, _>(ecx, cheats, f)
    }

    fn with_fresh_nested_evm(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        chain_context: ChainFor<FEN>,
        f: NestedEvmClosureFor<'_, FEN>,
    ) -> Result<EvmEnv<SpecFor<FEN>, BlockEnvFor<FEN>>, EVMError<DatabaseError>> {
        let depth = ecx.journal().depth();
        let evm_env = ecx.evm_clone();
        let (db, _) = ecx.db_journal_inner_mut();
        let mut evm =
            FEN::EvmFactory::default().create_nested_evm_with_inspector(db, evm_env, cheats);
        evm.journal_inner_mut().depth = depth;
        *evm.chain_mut() = chain_context;
        f(&mut *evm)?;
        Ok(evm.to_evm_env())
    }

    fn transact_on_db(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<ContextUpdateFor<EvmFactoryFor<FEN>>> {
        let evm_env = ecx.evm_clone();
        let outer_tx_env = ecx.tx_clone();
        let (db, inner) = ecx.db_journal_inner_mut();
        db.transact(fork_id, transaction, evm_env, &outer_tx_env, inner, cheats)
    }

    fn transact_from_tx_on_db(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        ecx: &mut FoundryContextFor<'_, FEN>,
        tx: TxEnvFor<FEN>,
    ) -> eyre::Result<()> {
        let evm_env = ecx.evm_clone();
        let (db, inner) = ecx.db_journal_inner_mut();
        db.transact_from_tx(tx, evm_env, inner, cheats)
    }

    fn console_log(&mut self, _msg: &str) {}
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
    pub fn clear(&mut self) {
        self.opened_read_files.clear();
    }
}

/// Helps collecting transactions from different forks.
#[derive(Clone, Debug)]
pub struct BroadcastableTransaction<N: Network = Ethereum> {
    /// The optional RPC URL.
    pub rpc: Option<String>,
    /// The transaction to broadcast.
    pub transaction: TransactionMaybeSigned<N>,
}

#[derive(Clone, Debug, Copy)]
pub struct RecordDebugStepInfo {
    /// The debug trace node index when the recording starts.
    pub start_node_idx: usize,
    /// The original tracer config when the recording starts.
    pub original_tracer_config: TracingInspectorConfig,
}

/// A callback registered for a storage access hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageHook {
    /// Contract that receives the callback.
    pub callback_target: Address,
    /// Callback function selector.
    pub callback_selector: [u8; 4],
}

#[derive(Clone, Debug)]
enum PendingStorageHook {
    Load {
        account: Address,
        slot: U256,
        hook: StorageHook,
    },
    Store {
        account: Address,
        slot: U256,
        old_value: U256,
        mapping: Option<(B256, Vec<B256>)>,
        hook: StorageHook,
    },
}

#[derive(Clone, Debug)]
struct ActiveStorageHook {
    parent_depth: usize,
    callback_target: Address,
    callback_input: Bytes,
    saved_gas: Gas,
    saved_return_data: Bytes,
    saved_stack_item: Option<U256>,
    journal_start: usize,
    inspector_state: StorageHookInspectorState,
    outcome: Option<(InstructionResult, Bytes)>,
}

#[derive(Clone, Debug)]
struct StorageHookInspectorState {
    accesses: RecordAccess,
    recording_accesses: bool,
    mapping_slots: Option<AddressHashMap<MappingSlots>>,
    recorded_logs: Option<Vec<Vm::Log>>,
    mocked_calls: HashMap<Address, BTreeMap<MockCallDataContext, VecDeque<MockCallReturnData>>>,
    mocked_functions: HashMap<Address, HashMap<Bytes, Address>>,
    expected_revert: Option<ExpectedRevert>,
    assume_no_revert: Option<AssumeNoRevert>,
    expected_calls: ExpectedCallTracker,
    expected_emits: ExpectedEmitTracker,
    expected_creates: Vec<ExpectedCreate>,
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
    /// Gas used by `snapshotGasLastCall`.
    pub(crate) last_call_snapshot_gas_used: u64,

    /// Cache of the amount of gas used in previous call or create frame.
    /// This is used by the `lastFrameGas` cheatcode.
    pub last_frame_gas: Option<crate::Vm::Gas>,
    /// Gas used by `snapshotGasLastFrame`.
    pub(crate) last_frame_snapshot_gas_used: u64,

    /// Post-refund gas used by the isolated transaction wrapping the current frame, and the
    /// account-creation state gas in it that the outer opcode charged before entering the frame.
    isolated_snapshot_gas_used: Option<(u64, u64)>,

    /// Caller depth, gas charged to the last isolated frame, and the transaction gas that replaces
    /// it in the next region sample at that depth.
    pending_isolated_region_gas: Option<(usize, u64, u64)>,

    /// True if gas recording is enabled.
    pub recording: bool,
    /// The gas used in the last frame.
    pub last_gas_used: u64,
    /// Gas records for the active snapshots.
    pub gas_records: Vec<GasRecord>,
}

impl GasMetering {
    /// Start the gas recording.
    pub const fn start(&mut self) {
        self.recording = true;
        self.last_gas_used = 0;
        self.pending_isolated_region_gas = None;
    }

    /// Stop the gas recording.
    pub const fn stop(&mut self) {
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

    /// Preserves the receipt gas of an isolated transaction for gas snapshots.
    ///
    /// `precharged_state` is the account-creation state gas in `gas_used` that the outer opcode
    /// already charged before entering the isolated frame.
    pub const fn set_isolated_snapshot_gas_used(&mut self, gas_used: u64, precharged_state: u64) {
        self.isolated_snapshot_gas_used = Some((gas_used, precharged_state));
    }

    /// Takes the receipt gas of the isolated transaction that wrapped the ending frame.
    ///
    /// Region snapshots replace the gas charged to the frame with the transaction gas, without
    /// changing the interpreter's gas. The transaction gas can be lower because of refunds, or
    /// higher because of intrinsic gas that did not fit in the frame's budget.
    const fn take_isolated_snapshot_gas_used(&mut self, depth: usize, gas: &Gas) -> Option<u64> {
        let Some((gas_used, precharged_state)) = self.isolated_snapshot_gas_used.take() else {
            return None;
        };
        if self.recording {
            // The caller's sample already includes the precharged state gas.
            self.pending_isolated_region_gas =
                Some((depth, gas.total_gas_spent(), gas_used.saturating_sub(precharged_state)));
        }
        Some(gas_used)
    }
}

/// Holds data about arbitrary storage.
#[derive(Clone, Debug, Default)]
pub struct ArbitraryStorage {
    /// Mapping of arbitrary storage addresses to generated values (slot, arbitrary value).
    /// (SLOADs return random value if storage slot wasn't accessed).
    /// Changed values are recorded and used to copy storage to different addresses.
    values: HashMap<Address, HashMap<U256, U256>>,
    /// Mapping of address with storage copied to arbitrary storage address source.
    copies: HashMap<Address, Address>,
    /// Address with storage slots that should be overwritten even if previously set.
    overwrites: HashSet<Address>,
    /// Storage slots explicitly written with `vm.store`, grouped by address.
    explicit_slots: HashMap<Address, HashSet<U256>>,
}

impl ArbitraryStorage {
    /// Marks an address with arbitrary storage.
    pub fn mark_arbitrary(&mut self, address: &Address, overwrite: bool) {
        self.values.insert(*address, HashMap::default());
        self.explicit_slots.remove(address);
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
            if let Some(slots) = self.explicit_slots.get(from).cloned() {
                self.explicit_slots.insert(*to, slots);
            } else {
                self.explicit_slots.remove(to);
            }
        }
    }

    /// Marks a slot as explicitly written if the address has arbitrary or copied storage.
    fn mark_explicit(&mut self, address: Address, slot: U256) {
        if self.values.contains_key(&address) || self.copies.contains_key(&address) {
            self.explicit_slots.entry(address).or_default().insert(slot);
        }
    }

    /// Returns whether a slot was explicitly written for the given address.
    fn is_explicit(&self, address: Address, slot: U256) -> bool {
        self.explicit_slots.get(&address).is_some_and(|slots| slots.contains(&slot))
    }

    /// Returns addresses explicitly marked with arbitrary storage.
    fn targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.values.keys().copied()
    }

    /// Returns addresses explicitly marked with arbitrary storage and whether nonzero slots are
    /// overwritten.
    fn target_overwrite_modes(&self) -> impl Iterator<Item = (Address, bool)> + '_ {
        self.values.keys().map(|address| (*address, self.overwrites.contains(address)))
    }

    /// Returns addresses that copy storage from arbitrary-storage targets.
    fn copied_targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.copies.keys().copied()
    }

    /// Returns copied arbitrary-storage targets and their source address.
    fn copied_target_sources(&self) -> impl Iterator<Item = (Address, Address)> + '_ {
        self.copies.iter().map(|(target, source)| (*target, *source))
    }

    /// Caches a concrete value for a slot on an arbitrary-storage address or copied target.
    fn cache_value(&mut self, address: Address, slot: U256, data: U256) {
        if let Some(values) = self.values.get_mut(&address) {
            values.insert(slot, data);
            return;
        }

        let Some(source) = self.copies.get(&address).copied() else {
            return;
        };
        if let Some(values) = self.values.get_mut(&source) {
            values.insert(slot, data);
        }
    }

    /// Returns a cached arbitrary value for a slot.
    fn cached_value(&self, address: Address, slot: U256) -> Option<U256> {
        self.values.get(&address).and_then(|values| values.get(&slot)).copied()
    }

    /// Saves arbitrary storage value for a given address:
    /// - store value in changed values cache.
    /// - update account's storage with given value.
    pub fn save<CTX: ContextTr>(
        &mut self,
        ecx: &mut CTX,
        address: Address,
        slot: U256,
        data: U256,
    ) {
        self.values.get_mut(&address).expect("missing arbitrary address entry").insert(slot, data);
        if ecx.journal_mut().load_account(address).is_ok() {
            ecx.journal_mut()
                .sstore(address, slot, data)
                .expect("could not set arbitrary storage value");
        }
    }

    /// Copies arbitrary storage value from source address to the given target address:
    /// - if a value is present in arbitrary values cache, then update target storage and return
    ///   existing value.
    /// - if no value was yet generated for given slot, then save new value in cache and update both
    ///   source and target storages.
    pub fn copy<CTX: ContextTr>(
        &mut self,
        ecx: &mut CTX,
        target: Address,
        slot: U256,
        new_value: U256,
    ) -> U256 {
        let source = self.copies.get(&target).expect("missing arbitrary copy target entry");
        let storage_cache = self.values.get_mut(source).expect("missing arbitrary source storage");
        let value = match storage_cache.get(&slot) {
            Some(value) => *value,
            None => {
                storage_cache.insert(slot, new_value);
                // Update source storage with new value.
                if ecx.journal_mut().load_account(*source).is_ok() {
                    ecx.journal_mut()
                        .sstore(*source, slot, new_value)
                        .expect("could not copy arbitrary storage value");
                }
                new_value
            }
        };
        // Update target storage with new value.
        if ecx.journal_mut().load_account(target).is_ok() {
            ecx.journal_mut().sstore(target, slot, value).expect("could not set storage");
        }
        value
    }
}

/// List of transactions that can be broadcasted.
pub type BroadcastableTransactions<N> = VecDeque<BroadcastableTransaction<N>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreatedAccountsFrameKind {
    Call,
    Create,
}

#[derive(Clone, Copy, Debug)]
struct CreatedAccountsFrame {
    kind: CreatedAccountsFrameKind,
    depth: usize,
    checkpoint: usize,
}

#[derive(Clone, Copy, Debug)]
struct CreatedAccountChange {
    fork_id: Option<LocalForkId>,
    address: Address,
    creation: usize,
    previous: Option<usize>,
    committed: bool,
}

#[derive(Clone, Debug)]
struct CreatedAccountsSnapshot {
    fork_id: Option<LocalForkId>,
    bindings: AddressHashMap<usize>,
}

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
pub struct Cheatcodes<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// Solar compiler instance, to grant syntactic and semantic analysis capabilities
    pub analysis: Option<CheatcodeAnalysis>,

    /// The block environment
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: Option<BlockEnvFor<FEN>>,

    /// The active fork block override updated by a fork-switching cheatcode.
    ///
    /// This persists fork changes made through a copy-on-write backend between invariant calls.
    pub fork_block_number_override: Option<u64>,

    /// Currently active EIP-7702 delegations that will be consumed when building the next
    /// transaction. Set by `vm.attachDelegation()` and consumed via `.take()` during
    /// transaction construction.
    pub active_delegations: Vec<SignedAuthorization>,

    /// The active EIP-4844 blob that will be attached to the next call.
    pub active_blob_sidecar: Option<BlobTransactionSidecarVariant>,

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

    /// Account accesses performed by the test runner before user code can start recording.
    pending_account_diffs: Option<Arc<[AccountAccess]>>,

    /// Completed account accesses prepended to the active user recording session.
    recorded_account_diffs_prefix: Option<Arc<[AccountAccess]>>,

    /// Successfully created accounts in execution order.
    created_accounts: Vec<Address>,

    /// The creation currently represented by each address on each fork.
    created_account_bindings: HashMap<(Option<LocalForkId>, Address), usize>,

    /// Revertible changes to creation bindings made by EVM create frames.
    created_account_changes: Vec<CreatedAccountChange>,

    /// Creation-list checkpoints for frames observed by this inspector.
    created_accounts_frames: Vec<CreatedAccountsFrame>,

    /// Creation lists captured by state snapshots.
    created_accounts_snapshots: HashMap<U256, CreatedAccountsSnapshot>,

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
    pub broadcastable_transactions: BroadcastableTransactions<FEN::Network>,

    /// Current EIP-2930 access lists.
    pub access_list: Option<AccessList>,

    /// Additional, user configurable context this Inspector has access to when inspecting a call.
    pub config: Arc<CheatsConfig>,

    /// Additional addresses recognized as cheatcode contracts by this executor.
    pub extra_cheatcode_addresses: &'static [Address],

    /// Test-scoped context holding data that needs to be reset every test run
    pub test_context: TestContext,

    /// Revert payloads minted by the `skip` cheatcode during the current test call.
    ///
    /// A top-level revert is only classified as a skip when its data byte-equals one of these
    /// payloads, so user-crafted `FOUNDRY::SKIP` revert data never skips a test on its own.
    pub skip_payloads: Vec<Bytes>,

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

    /// SLOAD callbacks keyed by effective storage address.
    storage_load_hooks: AddressHashMap<StorageHook>,
    /// SSTORE callbacks keyed by effective storage address.
    storage_store_hooks: AddressHashMap<StorageHook>,
    /// Mapping SSTORE callbacks keyed by effective storage address and root slot.
    mapping_storage_store_hooks: AddressHashMap<HashMap<B256, StorageHook>>,
    /// Execution-local provenance used only by mapping storage hooks.
    storage_hook_mapping_slots: AddressHashMap<MappingSlots>,
    /// A 64-byte Keccak operation awaiting successful completion.
    pending_mapping_hash: Option<PendingMappingHash>,
    /// Whether any storage hook map contains a callback.
    storage_hooks_registered: bool,
    /// Matching storage access captured before the opcode executes.
    pending_storage_hook: Option<PendingStorageHook>,
    /// Synthetic callback frame currently executing or awaiting parent cleanup.
    active_storage_hook: Option<ActiveStorageHook>,

    /// Deprecated cheatcodes mapped to the reason. Used to report warnings on test results.
    pub deprecated: HashMap<&'static str, Option<&'static str>>,
    /// Main script contract, when script execution protection is enabled.
    pub script_address: Option<Address>,
    /// Unlocked wallets used in scripts and testing of scripts.
    pub wallets: Option<Wallets>,
    /// Parsed secp256k1 private-key signers for repeated `vm.addr` / `vm.sign` calls.
    pub private_key_signers: HashMap<U256, PrivateKeySigner>,
    /// Signatures identifier for decoding events and functions
    signatures_identifier: OnceLock<Option<SignaturesIdentifier>>,
    /// Used to determine whether the broadcasted call has dynamic gas limit.
    pub dynamic_gas_limit: bool,
    // Custom execution evm version.
    pub execution_evm_version: Option<SpecFor<FEN>>,

    /// Per-fork opcode environment overrides and their state-snapshot copies.
    pub env_overrides: EnvOverrideState,

    /// Per-state-snapshot copies of [`Self::fork_block_number_override`].
    pub fork_block_number_override_snapshots: HashMap<U256, Option<u64>>,

    /// Transaction-position context and Monad's reserve-balance-tracker state captured atomically
    /// alongside state snapshots.
    #[cfg(feature = "monad")]
    pub context_snapshots:
        HashMap<U256, (ChainFor<FEN>, monad_revm::reserve_balance::tracker::ReserveBalanceTracker)>,

    /// Whether we are currently executing inside an isolation context, i.e.
    /// the synthetic inner transaction wrapped by
    /// `InspectorStackRefMut::transact_inner` (used by `--gas-report` and
    /// `--isolate`).
    ///
    /// Toggled by the inspector stack around the inner `transact_raw`
    /// call. Cheatcodes that mutate the tx/block env consult this flag and
    /// route the change through `EnvOverrides` instead of the actual env
    /// when `true`, so they don't fight with the fee-accounting zeroing.
    pub in_isolation_context: bool,

    /// Journal restored by a state snapshot inside an isolated transaction, to be applied to its
    /// suspended parent alongside the returned state.
    pub pending_isolated_snapshot_journal: Option<Vec<JournalEntry>>,

    /// Whether snapshot restorations belong to the tracked transaction (an isolated call, or the
    /// top-level transaction when isolation is disabled) and must be unwound by failing frames.
    pub track_isolated_snapshots: bool,

    /// Journals replaced by snapshot restorations that may need to be reinstated when an
    /// enclosing frame of the tracked transaction fails.
    pub isolated_snapshot_restores: Vec<JournaledState>,

    /// Whether the next snapshot restoration is the first one since its calling frame started,
    /// and so must be recorded in `isolated_snapshot_restores`.
    pub capture_isolated_snapshot_restore: bool,

    /// Depth of the in-flight `deployCode` call, whose create frame runs one level deeper.
    pub deploy_code_depth: Option<usize>,
}

// This is not derived because calling this in `fn new` with `..Default::default()` creates a second
// `CheatsConfig` which is unused, and inside it `ProjectPathsConfig` is relatively expensive to
// create.
impl Default for Cheatcodes {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl<FEN: FoundryEvmNetwork> Cheatcodes<FEN> {
    /// Creates a new `Cheatcodes` with the given settings.
    pub fn new(config: Arc<CheatsConfig>) -> Self {
        Self {
            analysis: None,
            fs_commit: true,
            labels: config.labels.clone(),
            config,
            extra_cheatcode_addresses: &[],
            block: Default::default(),
            fork_block_number_override: Default::default(),
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
            pending_account_diffs: Default::default(),
            recorded_account_diffs_prefix: Default::default(),
            created_accounts: Default::default(),
            created_account_bindings: Default::default(),
            created_account_changes: Default::default(),
            created_accounts_frames: Default::default(),
            created_accounts_snapshots: Default::default(),
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
            skip_payloads: Default::default(),
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
            storage_load_hooks: Default::default(),
            storage_store_hooks: Default::default(),
            mapping_storage_store_hooks: Default::default(),
            storage_hook_mapping_slots: Default::default(),
            pending_mapping_hash: Default::default(),
            storage_hooks_registered: Default::default(),
            pending_storage_hook: Default::default(),
            active_storage_hook: Default::default(),
            deprecated: Default::default(),
            script_address: Default::default(),
            wallets: Default::default(),
            private_key_signers: Default::default(),
            signatures_identifier: Default::default(),
            dynamic_gas_limit: Default::default(),
            execution_evm_version: None,
            env_overrides: Default::default(),
            fork_block_number_override_snapshots: Default::default(),
            #[cfg(feature = "monad")]
            context_snapshots: Default::default(),
            in_isolation_context: false,
            pending_isolated_snapshot_journal: None,
            track_isolated_snapshots: false,
            isolated_snapshot_restores: Vec::new(),
            capture_isolated_snapshot_restore: false,
            deploy_code_depth: None,
        }
    }

    /// Sets additional addresses recognized as cheatcode contracts.
    #[inline]
    pub const fn set_extra_cheatcode_addresses(&mut self, addresses: &'static [Address]) {
        self.extra_cheatcode_addresses = addresses;
    }

    /// Enables cheatcode analysis capabilities by providing a solar compiler instance.
    pub fn set_analysis(&mut self, analysis: CheatcodeAnalysis) {
        self.analysis = Some(analysis);
    }

    /// Starts an internal account diff recording session for test runner setup.
    pub fn start_internal_state_diff_recording(&mut self) -> bool {
        if self.recorded_account_diffs_stack.is_some()
            || self.recorded_account_diffs_prefix.is_some()
        {
            return false;
        }
        self.recorded_account_diffs_stack = Some(Default::default());
        true
    }

    /// Stops an internal account diff recording session without leaving recording enabled.
    pub fn stop_internal_state_diff_recording(&mut self) -> Vec<AccountAccess> {
        self.recorded_account_diffs_stack.take().unwrap_or_default().into_iter().flatten().collect()
    }

    /// Makes account accesses captured by the test runner available to the next recording session.
    pub fn set_pending_account_diffs(&mut self, accesses: Vec<AccountAccess>) {
        self.pending_account_diffs = (!accesses.is_empty()).then(|| Arc::from(accesses));
    }

    /// Starts a user account diff recording session, including pending test runner accesses.
    pub fn start_state_diff_recording(&mut self) {
        self.recorded_account_diffs_prefix = self.pending_account_diffs.take();
        self.recorded_account_diffs_stack = Some(Default::default());
    }

    /// Returns completed and active account accesses in execution order.
    pub fn recorded_account_diffs(&self) -> impl Iterator<Item = &AccountAccess> {
        self.recorded_account_diffs_prefix
            .iter()
            .flat_map(|prefix| prefix.iter())
            .chain(self.recorded_account_diffs_stack.iter().flatten().flatten())
    }

    /// Takes completed account accesses from the active user recording session.
    pub fn take_recorded_account_diffs_prefix(&mut self) -> Vec<AccountAccess> {
        self.recorded_account_diffs_prefix
            .take()
            .map(|prefix| prefix.as_ref().to_vec())
            .unwrap_or_default()
    }

    /// Returns the current creation bound to each address on the given fork.
    pub(crate) fn created_account_bindings(
        &self,
        fork_id: Option<LocalForkId>,
    ) -> AddressHashMap<usize> {
        self.created_account_bindings
            .iter()
            .filter_map(|(&(event_fork_id, address), &creation)| {
                (event_fork_id == fork_id).then_some((address, creation))
            })
            .collect()
    }

    /// Returns successfully created accounts bound to the given fork in creation order.
    pub(crate) fn created_accounts(&self, fork_id: Option<LocalForkId>) -> Vec<Address> {
        let bindings = self.created_account_bindings(fork_id);
        self.created_accounts
            .iter()
            .enumerate()
            .filter_map(|(index, &address)| {
                (bindings.get(&address) == Some(&index)).then_some(address)
            })
            .collect()
    }

    /// Records a successfully created account.
    pub(crate) fn record_created_account(
        &mut self,
        fork_id: Option<LocalForkId>,
        address: Address,
    ) {
        let creation = self.created_accounts.len();
        self.created_accounts.push(address);
        let previous = self.created_account_bindings.insert((fork_id, address), creation);
        self.created_account_changes.push(CreatedAccountChange {
            fork_id,
            address,
            creation,
            previous,
            committed: false,
        });
    }

    /// Keeps creation bindings saved with an outgoing fork across later frame reverts.
    pub(crate) fn commit_created_account_changes(&mut self, fork_id: Option<LocalForkId>) {
        for change in &mut self.created_account_changes {
            if change.fork_id == fork_id {
                change.committed = true;
            }
        }
    }

    /// Records shared pre-fork creations on a newly selected fork without replacing local ones.
    pub(crate) fn record_initial_created_accounts(
        &mut self,
        fork_id: Option<LocalForkId>,
        accounts: impl IntoIterator<Item = (Address, usize)>,
    ) {
        for (address, creation) in accounts {
            self.created_account_bindings.entry((fork_id, address)).or_insert(creation);
        }
    }

    /// Records creations propagated to a fork with persistent account state.
    pub(crate) fn record_propagated_accounts(
        &mut self,
        fork_id: Option<LocalForkId>,
        accounts: impl IntoIterator<Item = (Address, usize)>,
    ) {
        self.created_account_bindings
            .extend(accounts.into_iter().map(|(address, creation)| ((fork_id, address), creation)));
    }

    /// Captures creation ordering alongside a state snapshot.
    pub(crate) fn snapshot_created_accounts(
        &mut self,
        snapshot_id: U256,
        fork_id: Option<LocalForkId>,
    ) {
        let bindings = self.created_account_bindings(fork_id);
        self.created_accounts_snapshots
            .insert(snapshot_id, CreatedAccountsSnapshot { fork_id, bindings });
    }

    /// Restores creation ordering from a state snapshot.
    pub(crate) fn revert_created_accounts(&mut self, snapshot_id: U256, remove: bool) {
        let snapshot = if remove {
            self.created_accounts_snapshots.remove(&snapshot_id)
        } else {
            self.created_accounts_snapshots.get(&snapshot_id).cloned()
        };
        if let Some(snapshot) = snapshot {
            self.created_account_bindings.retain(|(fork_id, _), _| *fork_id != snapshot.fork_id);
            self.created_account_bindings.extend(
                snapshot
                    .bindings
                    .into_iter()
                    .map(|(address, creation)| ((snapshot.fork_id, address), creation)),
            );
        }
    }

    /// Deletes one captured creation-order snapshot.
    pub(crate) fn delete_created_accounts_snapshot(&mut self, snapshot_id: U256) {
        self.created_accounts_snapshots.remove(&snapshot_id);
    }

    /// Deletes all captured creation-order snapshots.
    pub(crate) fn clear_created_accounts_snapshots(&mut self) {
        self.created_accounts_snapshots.clear();
    }

    fn start_created_accounts_frame(
        &mut self,
        reset: bool,
        kind: CreatedAccountsFrameKind,
        depth: usize,
    ) {
        if reset {
            self.created_accounts.clear();
            self.created_account_bindings.clear();
            self.created_account_changes.clear();
            self.created_accounts_frames.clear();
            // Earlier snapshots contain no creations from the new root transaction.
            for snapshot in self.created_accounts_snapshots.values_mut() {
                snapshot.bindings.clear();
            }
        }
        self.created_accounts_frames.push(CreatedAccountsFrame {
            kind,
            depth,
            checkpoint: self.created_account_changes.len(),
        });
    }

    fn finish_created_accounts_frame(
        &mut self,
        success: bool,
        kind: CreatedAccountsFrameKind,
        depth: usize,
    ) {
        let Some(frame) = self
            .created_accounts_frames
            .last()
            .copied()
            .filter(|frame| frame.kind == kind && frame.depth == depth)
        else {
            return;
        };
        let checkpoint = frame.checkpoint;
        self.created_accounts_frames.pop();
        if !success {
            while self.created_account_changes.len() > checkpoint {
                let change = self.created_account_changes.pop().expect("length checked");
                if change.committed {
                    continue;
                }
                let key = (change.fork_id, change.address);
                if self.created_account_bindings.get(&key) != Some(&change.creation) {
                    continue;
                }
                if let Some(previous) = change.previous {
                    self.created_account_bindings.insert(key, previous);
                } else {
                    self.created_account_bindings.remove(&key);
                }
            }
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

    /// Returns the signatures identifier.
    pub fn signatures_identifier(&self) -> Option<&SignaturesIdentifier> {
        self.signatures_identifier
            .get_or_init(|| {
                if let Some(artifacts) = &self.config.available_artifacts {
                    return SignaturesIdentifier::new_offline_with_abis(
                        artifacts.values().map(|contract| &contract.abi),
                    )
                    .ok();
                }
                SignaturesIdentifier::new(true).ok()
            })
            .as_ref()
    }

    /// Decodes the input data and applies the cheatcode.
    fn apply_cheatcode(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        call: &CallInputs,
        executor: &mut dyn CheatcodesExecutor<FEN>,
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

        let caller = call.transfer_from();

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        ecx.db_mut().ensure_cheatcode_access_forking_mode(&caller)?;

        apply_dispatch(
            &decoded,
            &mut CheatsCtxt {
                state: self,
                ecx,
                gas_limit: call.gas_limit,
                caller,
                is_static: call.is_static,
            },
            executor,
        )
    }

    /// Decodes the input data and applies Monad-specific cheatcodes.
    #[cfg(feature = "monad")]
    fn apply_monad_cheatcode(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        call: &CallInputs,
    ) -> Result {
        let input = call.input.bytes(ecx);
        let caller = call.transfer_from();

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        ecx.db_mut().ensure_cheatcode_access_forking_mode(&caller)?;

        crate::monad::apply_monad_cheatcode(
            &mut CheatsCtxt {
                state: self,
                ecx,
                gas_limit: call.gas_limit,
                caller,
                is_static: call.is_static,
            },
            &input,
        )
    }

    /// Grants cheat code access for new contracts if the caller also has
    /// cheatcode access or the new contract is created in top most call.
    ///
    /// There may be cheatcodes in the constructor of the new contract, in order to allow them
    /// automatically we need to determine the new address.
    fn allow_cheatcodes_on_create(
        &self,
        ecx: &mut FoundryContextFor<FEN>,
        caller: Address,
        created_address: Address,
    ) {
        if ecx.journal().depth() <= 1 || ecx.db().has_cheatcode_access(&caller) {
            ecx.db_mut().allow_cheatcode_access(created_address);
        }
    }

    /// Apply EIP-2930 access list.
    ///
    /// If the transaction type is [TransactionType::Legacy] we need to upgrade it to
    /// [TransactionType::Eip2930] in order to use access lists. Other transaction types support
    /// access lists themselves.
    fn apply_accesslist(&mut self, ecx: &mut FoundryContextFor<FEN>) {
        if let Some(access_list) = &self.access_list {
            ecx.tx_mut().set_access_list(access_list.clone());

            if ecx.tx().tx_type() == TransactionType::Legacy as u8 {
                ecx.tx_mut().set_tx_type(TransactionType::Eip2930 as u8);
            }
        }
    }

    /// Called when there was a revert.
    ///
    /// Cleanup any previously applied cheatcodes that altered the state in such a way that revm's
    /// revert would run into issues.
    pub fn on_revert(&mut self, ecx: &mut FoundryContextFor<FEN>) {
        trace!(deals=?self.eth_deals.len(), "rolling back deals");

        // Delay revert clean up until expected revert is handled, if set.
        if self.expected_revert.is_some() {
            return;
        }

        // we only want to apply cleanup top level
        if ecx.journal().depth() > 0 {
            return;
        }

        // Roll back all previously applied deals
        // This will prevent overflow issues in revm's [`JournaledState::journal_revert`] routine
        // which rolls back any transfers.
        while let Some(record) = self.eth_deals.pop() {
            if let Some(acc) = ecx.journal_mut().evm_state_mut().get_mut(&record.address) {
                acc.info.balance = record.old_balance;
            }
        }
    }

    /// Handles a call, accounting for whether the executor will isolate it as a transaction.
    ///
    /// If `isolate_call` is true, the executor owns the transaction nonce increment when the call
    /// proceeds to execution.
    pub fn call_with_executor(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        call: &mut CallInputs,
        executor: &mut dyn CheatcodesExecutor<FEN>,
        isolate_call: bool,
    ) -> Option<CallOutcome> {
        // Apply custom execution evm version.
        if let Some(spec_id) = self.execution_evm_version {
            EvmFactoryFor::<FEN>::set_execution_spec(ecx, spec_id);
        }

        let gas = Gas::new(call.gas_limit);
        let curr_depth = ecx.journal().depth();
        self.start_created_accounts_frame(
            curr_depth == 0,
            CreatedAccountsFrameKind::Call,
            curr_depth,
        );

        // At the root call to test function or script `run()`/`setUp()` functions, we are
        // decreasing sender nonce to ensure that it matches on-chain nonce once we start
        // broadcasting.
        if curr_depth == 0 {
            let sender = ecx.tx().caller();
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
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                        charged_new_account_state_gas: call.charged_new_account_state_gas,
                    });
                }
            };
            let prev = account.info.nonce;
            account.info.nonce = prev.saturating_sub(1);

            trace!(target: "cheatcodes", %sender, nonce=account.info.nonce, prev, "corrected nonce");
        }

        if call.transfer_to() == CHEATCODE_ADDRESS {
            return match self.apply_cheatcode(ecx, call, executor) {
                Ok(retdata) => Some(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Return,
                        output: retdata.into(),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                    was_precompile_called: true,
                    precompile_call_logs: vec![],
                    charged_new_account_state_gas: call.charged_new_account_state_gas,
                }),
                Err(err) => Some(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: err.abi_encode().into(),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                    was_precompile_called: false,
                    precompile_call_logs: vec![],
                    charged_new_account_state_gas: call.charged_new_account_state_gas,
                }),
            };
        }

        #[cfg(feature = "monad")]
        if crate::monad::is_monad_cheatcode_call(self.extra_cheatcode_addresses, call.transfer_to())
        {
            let checkpoint = ecx.journal_mut().checkpoint();
            return match self.apply_monad_cheatcode(ecx, call) {
                Ok(retdata) => {
                    ecx.journal_mut().checkpoint_commit();
                    Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Return,
                            output: retdata.into(),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                        was_precompile_called: true,
                        precompile_call_logs: vec![],
                        charged_new_account_state_gas: call.charged_new_account_state_gas,
                    })
                }
                Err(err) => {
                    ecx.journal_mut().checkpoint_revert(checkpoint);
                    Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: err.abi_encode().into(),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                        charged_new_account_state_gas: call.charged_new_account_state_gas,
                    })
                }
            };
        }

        if call.transfer_to() == HARDHAT_CONSOLE_ADDRESS {
            return None;
        }

        // `expectRevert`: track max call depth. This is also done in `initialize_interp`, but
        // precompile calls don't create an interpreter frame so we must also track it here.
        // The callee executes at `curr_depth + 1`.
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = max(curr_depth + 1, expected.max_depth);
        }

        // Handle expected calls
        if let Some(expected) = self.expected_calls.get_mut(&call.bytecode_address) {
            let input = call.input.as_bytes(ecx);
            expect::observe_call(
                expected,
                &input,
                call.transfer_value(),
                call.gas_limit,
                call.scheme,
            );
        }

        // Apply our prank
        if let Some(prank) = self.get_prank(curr_depth).copied() {
            // Apply delegate call, `call.caller`` will not equal `prank.prank_caller`
            if prank.delegate_call && curr_depth == prank.depth && call.scheme.is_delegate_call() {
                call.target_address = prank.new_caller;
                call.caller = prank.new_caller;
                if let Some(new_origin) = prank.new_origin {
                    ecx.tx_mut().set_caller(new_origin);
                }
                if let Some(used) = prank.first_time_applied() {
                    self.pranks.insert(curr_depth, used);
                }
            }

            if let Some(changes) = prank.changes_for(curr_depth, call.transfer_from()) {
                if let Some(new_caller) = changes.caller {
                    // Ensure new caller is loaded and touched
                    let _ = journaled_account(ecx, new_caller);
                    call.caller = new_caller;
                }
                if let Some(new_origin) = changes.origin {
                    ecx.tx_mut().set_caller(new_origin);
                }
                if let Some(used) = changes.used {
                    self.pranks.insert(curr_depth, used);
                }
            }
        }

        // Handle mocked calls
        if let Some(mocks) = self.mocked_calls.get_mut(&call.bytecode_address) {
            let input = call.input.bytes(ecx);
            if let Some(return_data_queue) =
                mock::find_mock_returns(mocks, &input, call.transfer_value())
                && let Some(return_data) = return_data_queue.front().map(|x| x.to_owned())
            {
                if let Some(value) = call.transfer_value() {
                    let checkpoint = ecx.journal_mut().checkpoint();
                    match ecx.journal_mut().transfer_loaded(
                        call.transfer_from(),
                        call.transfer_to(),
                        value,
                    ) {
                        None => {
                            if return_data.ret_type.is_ok() {
                                ecx.journal_mut().checkpoint_commit();
                            } else {
                                ecx.journal_mut().checkpoint_revert(checkpoint);
                            }
                        }
                        Some(err) => {
                            ecx.journal_mut().checkpoint_revert(checkpoint);
                            return Some(CallOutcome {
                                result: InterpreterResult {
                                    result: err.into(),
                                    output: Bytes::new(),
                                    gas,
                                },
                                memory_offset: call.return_memory_offset.clone(),
                                was_precompile_called: false,
                                precompile_call_logs: vec![],
                                charged_new_account_state_gas: call.charged_new_account_state_gas,
                            });
                        }
                    }
                }

                mock::advance_mock_returns(return_data_queue);

                return Some(CallOutcome {
                    result: InterpreterResult {
                        result: return_data.ret_type,
                        output: return_data.data,
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                    was_precompile_called: true,
                    precompile_call_logs: vec![],
                    charged_new_account_state_gas: call.charged_new_account_state_gas,
                });
            }
        }

        // Apply EIP-2930 access list
        self.apply_accesslist(ecx);

        // Apply our broadcast
        if let Some(broadcast) = &mut self.broadcast {
            // Additional check as transfers in forge scripts seem to be estimated at 2300
            // by revm leading to "Intrinsic gas too low" failure when simulated on chain.
            let is_fixed_gas_limit = call.gas_limit >= 21_000 && !self.dynamic_gas_limit;
            self.dynamic_gas_limit = false;

            // We only apply a broadcast *to a specific depth*.
            //
            // We do this because any subsequent contract calls *must* exist on chain and
            // we only want to grab *this* call, not internal ones. `deployCode` routed through
            // the CREATE2 factory runs one level deeper in a nested EVM.
            if (curr_depth == broadcast.depth || broadcast.deploy_from_code)
                && call.transfer_from() == broadcast.original_caller
            {
                // Reset deploy from code flag for upcoming calls.
                broadcast.deploy_from_code = false;

                // At the target depth we set `msg.sender` & tx.origin.
                // We are simulating the caller as being an EOA, so *both* must be set to the
                // broadcast.origin.
                ecx.tx_mut().set_caller(broadcast.new_origin);

                call.caller = broadcast.new_origin;
                // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                // because we only need the from, to, value, and data. We can later change this
                // into 1559, in the cli package, relatively easily once we
                // know the target chain supports EIP-1559.
                if !call.is_static {
                    if let Err(err) = ecx.journal_mut().load_account(broadcast.new_origin) {
                        return Some(CallOutcome {
                            result: InterpreterResult {
                                result: InstructionResult::Revert,
                                output: Error::encode(err),
                                gas,
                            },
                            memory_offset: call.return_memory_offset.clone(),
                            was_precompile_called: false,
                            precompile_call_logs: vec![],
                            charged_new_account_state_gas: call.charged_new_account_state_gas,
                        });
                    }

                    let input = call.input.bytes(ecx);
                    let chain_id = ecx.cfg().chain_id();
                    let rpc = ecx.db().active_fork_url();
                    let fee_token = ecx.tx().fee_token();
                    let nonce =
                        ecx.journal().evm_state().get(&broadcast.new_origin).unwrap().info.nonce;

                    let mut tx_req = TransactionRequestFor::<FEN>::default()
                        .with_from(broadcast.new_origin)
                        .with_to(call.transfer_to())
                        .with_value(call.transfer_value().unwrap_or_default())
                        .with_input(input)
                        .with_nonce(nonce)
                        .with_chain_id(chain_id);
                    if is_fixed_gas_limit {
                        tx_req.set_gas_limit(call.gas_limit)
                    }

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
                                was_precompile_called: false,
                                precompile_call_logs: vec![],
                                charged_new_account_state_gas: call.charged_new_account_state_gas,
                            });
                        }
                        tx_req.set_blob_sidecar(blob_sidecar);
                    }

                    // Apply active EIP-7702 delegations, if any.
                    if !active_delegations.is_empty() {
                        if let Err(err) = apply_authorization_nonces::<FEN>(
                            ecx,
                            &active_delegations,
                            broadcast.new_origin,
                            chain_id,
                        ) {
                            return Some(CallOutcome {
                                result: InterpreterResult {
                                    result: InstructionResult::Revert,
                                    output: err.abi_encode().into(),
                                    gas,
                                },
                                memory_offset: call.return_memory_offset.clone(),
                                was_precompile_called: false,
                                precompile_call_logs: vec![],
                                charged_new_account_state_gas: call.charged_new_account_state_gas,
                            });
                        }
                        tx_req.set_authorization_list(active_delegations);
                    }
                    if let Some(fee_token) = fee_token {
                        tx_req.set_fee_token(fee_token);
                    }
                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc,
                        transaction: TransactionMaybeSigned::new(tx_req),
                    });
                    debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable call");

                    // Isolated transactions increment the nonce during execution. Nested
                    // broadcasts do not start a separate transaction and need this increment.
                    if !isolate_call {
                        let account = ecx
                            .journal_mut()
                            .evm_state_mut()
                            .get_mut(&broadcast.new_origin)
                            .unwrap();
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
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                        charged_new_account_state_gas: call.charged_new_account_state_gas,
                    });
                }
            }
        }

        // Record called accounts if `startStateDiffRecording` has been called
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // Determine if account is "initialized," ie, it has a non-zero balance, a non-zero
            // nonce, a non-zero KECCAK_EMPTY codehash, or non-empty code
            let (initialized, old_balance, old_nonce) =
                if let Ok(acc) = ecx.journal_mut().load_account(call.transfer_to()) {
                    (acc.data.info.exists(), acc.data.info.balance, acc.data.info.nonce)
                } else {
                    (false, U256::ZERO, 0)
                };

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
                    forkId: ecx.db().active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.cfg().chain_id()),
                },
                accessor: call.transfer_from(),
                account: call.bytecode_address,
                kind,
                initialized,
                oldBalance: old_balance,
                newBalance: U256::ZERO, // updated on call_end
                oldNonce: old_nonce,
                newNonce: 0, // updated on call_end
                value: call.call_value(),
                data: call.input.bytes(ecx),
                reverted: false,
                deployedCode: Bytes::new(),
                storageAccesses: vec![], // updated on step
                depth: ecx.journal().depth().try_into().expect("journaled state depth exceeds u64"),
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

    /// Returns addresses explicitly marked with arbitrary storage.
    pub fn arbitrary_storage_targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.arbitrary_storage.as_ref().into_iter().flat_map(ArbitraryStorage::targets)
    }

    /// Returns addresses explicitly marked with arbitrary storage and whether nonzero slots are
    /// overwritten.
    pub fn arbitrary_storage_target_overwrite_modes(
        &self,
    ) -> impl Iterator<Item = (Address, bool)> + '_ {
        self.arbitrary_storage
            .as_ref()
            .into_iter()
            .flat_map(ArbitraryStorage::target_overwrite_modes)
    }

    /// Returns addresses that copy storage from arbitrary-storage targets.
    pub fn arbitrary_storage_copied_targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.arbitrary_storage.as_ref().into_iter().flat_map(ArbitraryStorage::copied_targets)
    }

    /// Returns copied arbitrary-storage targets and their source address.
    pub fn arbitrary_storage_copied_target_sources(
        &self,
    ) -> impl Iterator<Item = (Address, Address)> + '_ {
        self.arbitrary_storage
            .as_ref()
            .into_iter()
            .flat_map(ArbitraryStorage::copied_target_sources)
    }

    /// Caches a concrete replay value for a slot on an arbitrary-storage address or copied target.
    pub fn cache_arbitrary_storage_value(&mut self, address: Address, slot: U256, value: U256) {
        if let Some(storage) = &mut self.arbitrary_storage {
            storage.cache_value(address, slot, value);
        }
    }

    /// Marks a slot as explicitly written with `vm.store`.
    pub fn mark_arbitrary_storage_slot_explicit(&mut self, address: Address, slot: U256) {
        if let Some(storage) = &mut self.arbitrary_storage {
            storage.mark_explicit(address, slot);
        }
    }

    /// Returns whether a slot was explicitly written with `vm.store`.
    pub fn is_arbitrary_storage_slot_explicit(&self, address: Address, slot: U256) -> bool {
        self.arbitrary_storage.as_ref().is_some_and(|storage| storage.is_explicit(address, slot))
    }

    /// Returns a cached arbitrary-storage replay value for a slot.
    pub fn cached_arbitrary_storage_value(&self, address: Address, slot: U256) -> Option<U256> {
        self.arbitrary_storage.as_ref().and_then(|storage| storage.cached_value(address, slot))
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

    /// Registers an SLOAD callback, replacing the existing callback for `target`.
    pub fn register_storage_load_hook(
        &mut self,
        target: Address,
        callback_target: Address,
        callback_selector: [u8; 4],
    ) {
        self.storage_load_hooks.insert(target, StorageHook { callback_target, callback_selector });
        self.storage_hooks_registered = true;
    }

    /// Registers an SSTORE callback, replacing the existing callback for `target`.
    pub fn register_storage_store_hook(
        &mut self,
        target: Address,
        callback_target: Address,
        callback_selector: [u8; 4],
    ) {
        self.storage_store_hooks.insert(target, StorageHook { callback_target, callback_selector });
        self.storage_hooks_registered = true;
    }

    /// Registers a mapping SSTORE callback. Returns false when a raw hook conflicts.
    pub fn register_mapping_storage_store_hook(
        &mut self,
        target: Address,
        root_slot: B256,
        callback_target: Address,
        callback_selector: [u8; 4],
    ) -> bool {
        if self.storage_store_hooks.contains_key(&target) {
            return false;
        }
        self.storage_hook_mapping_slots.remove(&target);
        self.mapping_storage_store_hooks
            .entry(target)
            .or_default()
            .insert(root_slot, StorageHook { callback_target, callback_selector });
        self.storage_hooks_registered = true;
        true
    }

    /// Returns registered mapping SSTORE callbacks.
    pub fn mapping_storage_store_hooks(
        &self,
    ) -> impl Iterator<Item = (Address, B256, StorageHook)> + '_ {
        self.mapping_storage_store_hooks
            .iter()
            .flat_map(|(target, hooks)| hooks.iter().map(|(root, hook)| (*target, *root, *hook)))
    }

    /// Returns whether mapping hooks conflict with a raw store hook.
    pub fn has_mapping_storage_store_hooks(&self, target: Address) -> bool {
        self.mapping_storage_store_hooks.get(&target).is_some_and(|hooks| !hooks.is_empty())
    }

    /// Returns registered SLOAD callbacks.
    pub fn storage_load_hooks(&self) -> impl Iterator<Item = (Address, StorageHook)> + '_ {
        self.storage_load_hooks.iter().map(|(target, hook)| (*target, *hook))
    }

    /// Returns registered SSTORE callbacks.
    pub fn storage_store_hooks(&self) -> impl Iterator<Item = (Address, StorageHook)> + '_ {
        self.storage_store_hooks.iter().map(|(target, hook)| (*target, *hook))
    }

    /// Returns whether any storage callback is registered.
    #[inline]
    pub const fn has_storage_hooks(&self) -> bool {
        self.storage_hooks_registered
    }

    /// Clears execution-local mapping provenance while preserving hook registrations.
    pub fn clear_storage_hook_mapping_slots(&mut self) {
        self.storage_hook_mapping_slots.clear();
    }

    /// Returns whether a synthetic storage-hook callback or one of its child calls is executing.
    #[inline]
    pub const fn is_storage_hook_active(&self) -> bool {
        self.active_storage_hook.is_some()
    }

    /// Returns whether `call` is the synthetic callback for the active storage hook.
    pub fn is_storage_hook_callback(
        &self,
        ecx: &FoundryContextFor<'_, FEN>,
        call: &CallInputs,
    ) -> bool {
        self.active_storage_hook.as_ref().is_some_and(|active| {
            active.outcome.is_none()
                && ecx.journal().depth() == active.parent_depth
                && call.transfer_from() == CHEATCODE_ADDRESS
                && call.transfer_to() == active.callback_target
                && call.input.bytes(ecx) == active.callback_input
        })
    }

    fn finish_storage_hook_call(
        &mut self,
        ecx: &FoundryContextFor<'_, FEN>,
        call: &CallInputs,
        outcome: &CallOutcome,
    ) -> bool {
        let Some(active) = self.active_storage_hook.as_mut() else { return false };
        if active.outcome.is_some()
            || ecx.journal().depth() != active.parent_depth
            || call.transfer_from() != CHEATCODE_ADDRESS
            || call.transfer_to() != active.callback_target
            || call.input.bytes(ecx) != active.callback_input
        {
            return false;
        }
        active.outcome = Some((outcome.result.result, outcome.result.output.clone()));
        true
    }

    #[inline(always)]
    pub fn has_step_hooks(&self) -> bool {
        self.broadcast.is_some()
            || self.gas_metering.paused
            || self.gas_metering.reset
            || self.recording_accesses
            || self.recorded_account_diffs_stack.is_some()
            || !self.allowed_mem_writes.is_empty()
            || self.mapping_slots.is_some()
            || self.gas_metering.recording
            || self.has_active_env_overrides()
            || self.has_storage_hooks()
    }

    #[inline(always)]
    pub fn has_step_end_hooks(&self) -> bool {
        self.gas_metering.paused
            || self.gas_metering.touched
            || self.arbitrary_storage.is_some()
            || self.mapping_slots.is_some()
            || self.has_active_env_overrides()
            || self.has_storage_hooks()
    }

    #[inline(always)]
    pub fn has_log_hooks(&self) -> bool {
        !self.expected_emits.is_empty() || self.recorded_logs.is_some()
    }

    #[inline(always)]
    pub fn has_recording_accesses_only_step_hook(&self) -> bool {
        self.recording_accesses
            && self.broadcast.is_none()
            && !self.gas_metering.paused
            && !self.gas_metering.reset
            && self.recorded_account_diffs_stack.is_none()
            && self.allowed_mem_writes.is_empty()
            && self.mapping_slots.is_none()
            && !self.has_storage_hooks()
            && !self.gas_metering.recording
            && !self.has_active_env_overrides()
    }

    #[inline(always)]
    fn has_active_env_overrides(&self) -> bool {
        self.env_overrides.is_any_set()
    }

    /// Returns struct definitions from the analysis, if available.
    pub fn struct_defs(&self) -> Option<&foundry_common::fmt::StructDefinitions> {
        self.analysis.as_ref().and_then(|analysis| analysis.struct_defs().ok())
    }
}

const fn frame_gas(result: &InterpreterResult) -> Vm::Gas {
    let gas = &result.gas;
    // A halt consumes the regular gas restored while rolling back state gas.
    let regular_gas_spent = if result.is_halt() {
        gas.total_gas_spent()
    } else {
        gas.total_gas_spent().saturating_sub(gas.state_gas_spilled())
    };
    Vm::Gas {
        gasLimit: gas.limit(),
        gasTotalUsed: regular_gas_spent,
        gasMemoryUsed: 0,
        gasRefunded: gas.refunded(),
        gasRemaining: gas.remaining(),
        gasStateUsed: if result.is_ok() { gas.state_gas_spent() } else { 0 },
    }
}

impl<FEN: FoundryEvmNetwork> Inspector<FoundryContextFor<'_, FEN>> for Cheatcodes<FEN> {
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            ecx.set_block(block);
        }
        if let Some(gas_price) = self.gas_price.take() {
            ecx.tx_mut().set_gas_price(gas_price);
        }

        // Record gas for current frame.
        if self.gas_metering.paused {
            self.gas_metering.paused_frames.push(interpreter.gas);
        }

        // `expectRevert`: track the max call depth during `expectRevert`
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = max(ecx.journal().depth(), expected.max_depth);
        }
    }

    fn step(&mut self, interpreter: &mut Interpreter, ecx: &mut FoundryContextFor<'_, FEN>) {
        self.pc = interpreter.bytecode.pc();

        if !self.has_step_hooks() {
            return;
        }

        if self.finish_storage_hook_callback(interpreter, ecx) {
            return;
        }

        if self.broadcast.is_some() {
            self.set_gas_limit_type(interpreter);
        }

        // Broadcasting changes outgoing calls, not the caller of the script's current frame.
        // Only protect the broadcasting frame; callbacks into the script have their own caller.
        if interpreter.bytecode.opcode() == op::CALLER
            && let Some(broadcast) = &self.broadcast
            && let Some(script_address) = self.script_address
            && ecx.journal().depth() == broadcast.depth
            && interpreter.input.target_address == script_address
            && interpreter.input.bytecode_address == Some(script_address)
            && interpreter.input.caller_address != broadcast.new_origin
        {
            interpreter.bytecode.set_action(InterpreterAction::new_return(
                InstructionResult::Revert,
                Bytes::from(
                    format!(
                        "Usage of `msg.sender` inside a `broadcast` in script contract detected. \
                         `msg.sender` is `{:#x}`, not the broadcast sender `{:#x}`. \
                         Use the `--sender` flag or pass the deployer address directly instead.",
                        interpreter.input.caller_address, broadcast.new_origin,
                    )
                    .into_bytes(),
                ),
                interpreter.gas,
            ));
            return;
        }

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
                ecx.journal().depth().try_into().expect("journaled state depth exceeds u64"),
            );
        }

        if self.mapping_slots.is_some() || !self.mapping_storage_store_hooks.is_empty() {
            // `startMappingRecording`: record SSTORE.
            if let Some(mapping_slots) = &mut self.mapping_slots {
                mapping_step(mapping_slots, interpreter);
            }

            let account = interpreter.input.target_address;
            let mapping_hook_active = self.active_storage_hook.is_none()
                && self
                    .mapping_storage_store_hooks
                    .get(&account)
                    .is_some_and(|hooks| !hooks.is_empty());
            if mapping_hook_active {
                mapping_step(&mut self.storage_hook_mapping_slots, interpreter);
            }
            self.pending_mapping_hash = if self.mapping_slots.is_some() || mapping_hook_active {
                capture_mapping_hash(interpreter)
            } else {
                None
            };
        }

        // `snapshotGas*`: take a snapshot of the current gas.
        if self.gas_metering.recording {
            self.meter_gas_record(interpreter, ecx);
        }

        // Capture the opcode for `step_end` to use, since by the time
        // `step_end` runs the PC has already advanced past it. Also peek the
        // BLOBHASH index now (still on top of stack before execution) so we
        // can look up the override later.
        if !self.env_overrides.is_empty() {
            let fork_id = ecx.db().active_fork_id();
            if let Some(env_overrides) =
                self.env_overrides.get_mut(fork_id).filter(|o| o.is_any_set())
            {
                // Always clear stale pending state first so a leftover value from
                // a prior step (e.g. when `peek` failed, or when an override
                // wasn't actually used) cannot leak into the next opcode.
                env_overrides.pending_opcode = None;
                env_overrides.pending_blobhash_index = None;

                let opcode = interpreter.bytecode.opcode();
                match opcode {
                    op::BASEFEE | op::GASPRICE => {
                        env_overrides.pending_opcode = Some(opcode);
                    }
                    op::BLOBHASH => {
                        env_overrides.pending_opcode = Some(opcode);
                        env_overrides.pending_blobhash_index =
                            interpreter.stack.peek(0).ok().and_then(|index| index.try_into().ok());
                    }
                    _ => {}
                }
            }
        }

        if self.active_storage_hook.is_none() {
            self.capture_storage_hook(interpreter, ecx);
        }
    }

    fn step_end(&mut self, interpreter: &mut Interpreter, ecx: &mut FoundryContextFor<'_, FEN>) {
        if !self.has_step_end_hooks() {
            return;
        }

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

        if let Some(pending) = self.pending_mapping_hash.take()
            && interpreter
                .bytecode
                .action
                .as_ref()
                .and_then(InterpreterAction::instruction_result)
                .is_none()
        {
            if let Some(mapping_slots) = &mut self.mapping_slots {
                record_mapping_hash(mapping_slots, interpreter, pending);
            }
            if self
                .mapping_storage_store_hooks
                .get(&pending.address)
                .is_some_and(|hooks| !hooks.is_empty())
                && self.active_storage_hook.is_none()
            {
                record_mapping_hash(&mut self.storage_hook_mapping_slots, interpreter, pending);
            }
        }

        if self.active_storage_hook.is_none() {
            self.invoke_pending_storage_hook(interpreter, ecx);
        }

        // Apply opcode-level env overrides (basefee/gasprice/blobhash). Needed
        // in isolation mode where the actual tx/block env is zeroed for
        // fee-accounting; in non-isolation mode the override and the real env
        // are kept in sync by the cheatcode handlers, so this is a no-op fixup.
        //
        // We must only rewrite the stack if the opcode actually completed
        // successfully and pushed its result; otherwise (stack underflow on
        // BLOBHASH, OOG before push, etc.) the stack is in an error state and
        // a blind `pop()+push()` would corrupt the failing frame.
        if !self.env_overrides.is_empty() {
            let fork_id = ecx.db().active_fork_id();
            if self.env_overrides.get(fork_id).is_some_and(|o| o.is_any_set()) {
                // Mirrors the pattern used by `meter_gas_record`: when `action` is
                // `Some` with an `instruction_result`, the opcode has set a
                // non-continue result (halt/revert/error) — i.e. it didn't push
                // its normal result. `None` means "still running", which is the
                // success path for a stack-only opcode in `step_end`.
                let opcode_failed = interpreter
                    .bytecode
                    .action
                    .as_ref()
                    .and_then(|a| a.instruction_result())
                    .is_some();
                if opcode_failed {
                    if let Some(env_overrides) = self.env_overrides.get_mut(fork_id) {
                        env_overrides.pending_opcode = None;
                        env_overrides.pending_blobhash_index = None;
                    }
                } else {
                    self.apply_env_overrides(interpreter, fork_id);
                }
            }
        }
    }

    fn log(&mut self, _ecx: &mut FoundryContextFor<'_, FEN>, log: Log) {
        if !self.expected_emits.is_empty()
            && let Some(err) = expect::handle_expect_emit(self, &log, None)
        {
            // Because we do not have access to the interpreter here, we cannot fail the test
            // immediately. In most cases the failure will still be caught on `call_end`.
            // In the rare case it is not, we log the error here.
            let _ = sh_err!("{err:?}");
        }

        // `recordLogs`
        record_logs(&mut self.recorded_logs, &log);
    }

    fn log_full(
        &mut self,
        interpreter: &mut Interpreter,
        _ecx: &mut FoundryContextFor<'_, FEN>,
        log: Log,
    ) {
        if !self.expected_emits.is_empty() {
            expect::handle_expect_emit(self, &log, Some(interpreter));
        }

        // `recordLogs`
        record_logs(&mut self.recorded_logs, &log);
    }

    fn call(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        if self.is_storage_hook_callback(ecx, inputs) {
            return None;
        }
        Self::call_with_executor(self, ecx, inputs, &mut TransparentCheatcodesExecutor, false)
    }

    fn call_end(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        call: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        let isolated_snapshot_gas_used = self
            .gas_metering
            .take_isolated_snapshot_gas_used(ecx.journal().depth(), &outcome.result.gas);
        if self.finish_storage_hook_call(ecx, call, outcome) {
            return;
        }

        let cheatcode_call = call.transfer_to() == CHEATCODE_ADDRESS
            || call.transfer_to() == HARDHAT_CONSOLE_ADDRESS;
        #[cfg(feature = "monad")]
        let cheatcode_call = cheatcode_call
            || crate::monad::is_monad_cheatcode_call(
                self.extra_cheatcode_addresses,
                call.transfer_to(),
            );
        let curr_depth = ecx.journal().depth();

        self.finish_created_accounts_frame(
            outcome.result.is_ok(),
            CreatedAccountsFrameKind::Call,
            curr_depth,
        );

        // Clean up pranks/broadcasts if it's not a cheatcode call end. We shouldn't do
        // it for cheatcode calls because they are not applied for cheatcodes in the `call` hook.
        // This should be placed before the revert handling, because we might exit early there
        if !cheatcode_call {
            // Clean up pranks
            if let Some(prank) = &self.get_prank(curr_depth)
                && curr_depth == prank.depth
            {
                ecx.tx_mut().set_caller(prank.prank_origin);

                // Clean single-call prank once we have returned to the original depth
                if prank.single_call {
                    self.pranks.remove(&curr_depth);
                }
            }

            // Clean up broadcast
            if let Some(broadcast) = &self.broadcast
                && curr_depth == broadcast.depth
            {
                ecx.tx_mut().set_caller(broadcast.original_origin);

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
                assume_no_revert.reverted_by = Some(call.transfer_to());
            }

            // allow multiple cheatcode calls at the same depth
            let curr_depth = ecx.journal().depth();
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
                }
                // Call didn't revert, reset `assume_no_revert` state.
                self.assume_no_revert = None;
            }
        }

        // Handle expected reverts.
        if let Some(expected_revert) = &mut self.expected_revert {
            // Record current reverter address and call scheme before processing the expect revert
            // if call reverted.
            let call_failed = !outcome.result.result.is_ok();
            if call_failed {
                // Record current reverter address if expect revert is set with expected reverter
                // address and no actual reverter was set yet or if we're expecting more than one
                // revert.
                if expected_revert.reverter.is_some()
                    && (expected_revert.reverted_by.is_none() || expected_revert.count > 1)
                {
                    expected_revert.reverted_by = Some(call.transfer_to());
                }
            }

            let curr_depth = ecx.journal().depth();
            if curr_depth <= expected_revert.depth {
                let needs_processing = expected_revert.needs_processing(
                    cheatcode_call,
                    call_failed,
                    curr_depth,
                    self.config.internal_expect_revert,
                );

                if needs_processing {
                    let mut expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                    let clear_last_frame_gas =
                        matches!(expected_revert.kind, ExpectedRevertKind::Default);
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
                                self.expected_revert = Some(expected_revert);
                            }
                            if clear_last_frame_gas {
                                self.gas_metering.last_frame_gas = None;
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

        // Record the gas usage of the call, this allows the `lastFrameGas` cheatcode to
        // retrieve the gas usage of the last call or create.
        let frame_gas = frame_gas(&outcome.result);
        let snapshot_gas_used =
            isolated_snapshot_gas_used.unwrap_or_else(|| outcome.result.gas.total_gas_spent());
        self.gas_metering.last_call_gas = Some(frame_gas.clone());
        self.gas_metering.last_frame_gas = Some(frame_gas);
        self.gas_metering.last_call_snapshot_gas_used = snapshot_gas_used;
        self.gas_metering.last_frame_snapshot_gas_used = snapshot_gas_used;

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // The root call cannot be recorded.
            if ecx.journal().depth() > 0
                && let Some(mut last_recorded_depth) = recorded_account_diffs_stack.pop()
            {
                // Update the reverted status of all deeper calls if this call reverted, in
                // accordance with EVM behavior
                if outcome.result.is_revert() {
                    mark_account_accesses_reverted(&mut last_recorded_depth);
                }

                if let Some(call_access) = last_recorded_depth.first_mut() {
                    // Assert that we're at the correct depth before recording post-call state
                    // changes. Depending on the depth the cheat was
                    // called at, there may not be any pending
                    // calls to update if execution has percolated up to a higher depth.
                    let curr_depth = ecx.journal().depth();
                    if call_access.depth == curr_depth as u64
                        && let Ok(acc) = ecx.journal_mut().load_account(call.transfer_to())
                    {
                        debug_assert!(access_is_call(call_access.kind));
                        call_access.newBalance = acc.data.info.balance;
                        call_access.newNonce = acc.data.info.nonce;
                    }
                    merge_recorded_frame(recorded_account_diffs_stack, last_recorded_depth);
                }
            }
        }

        // this will ensure we don't have false positives when trying to diagnose reverts in fork
        // mode
        let diag = self.fork_revert_diagnostic.take();

        // If the call already reverted, preserve that primary failure and skip post-call
        // expect* validation so it cannot overwrite the original revert.
        if outcome.result.is_revert() {
            // if there's a revert and a previous call was diagnosed as fork related revert then we
            // can return a better error here
            if let Some(err) = diag {
                outcome.result.output = Error::encode(err.to_error_msg(&self.labels));
            }
            return;
        }

        if let Some(unmet) = expected_emit::check_call_emits(
            &mut self.expected_emits,
            ecx.journal().depth(),
            call.is_static,
            outcome.result.is_ok(),
        ) {
            outcome.result.result = InstructionResult::Revert;
            outcome.result.output = unmet.encode(|| self.signatures_identifier());
            return;
        }

        // try to diagnose reverts in multi-fork mode where a call is made to an address that does
        // not exist
        if let TxKind::Call(test_contract) = ecx.tx().kind() {
            // if a call to a different contract than the original test contract returned with
            // `Stop` we check if the contract actually exists on the active fork
            if ecx.db().is_forked_mode()
                && outcome.result.result == InstructionResult::Stop
                && call.transfer_to() != test_contract
            {
                self.fork_revert_diagnostic =
                    ecx.db().diagnose_revert(call.transfer_to(), ecx.journal().evm_state());
            }
        }

        // If the depth is 0, then this is the root call terminating
        if ecx.journal().depth() == 0 {
            // If we already have a revert, we shouldn't run the below logic as it can obfuscate an
            // earlier error that happened first with unrelated information about
            // another error when using cheatcodes.
            if outcome.result.is_revert() {
                return;
            }

            // If there's not a revert, we can continue on to run the last logic for expect*
            // cheatcodes.

            // Match expected calls
            if let Some(msg) =
                expect::first_unmet_call(&self.expected_calls, outcome.result.is_ok())
            {
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
                return;
            }

            // Check if we have any leftover expected emits
            if let Some(msg) = expected_emit::first_unmet_root_emit(
                &mut self.expected_emits,
                outcome.result.is_ok(),
            ) {
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
                return;
            }

            // Check for leftover expected creates
            if let Some(msg) = expect::first_unmet_create(&self.expected_creates) {
                outcome.result.result = InstructionResult::Revert;
                outcome.result.output = Error::encode(msg);
            }
        }
    }

    fn create(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        mut input: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        // Apply custom execution evm version.
        if let Some(spec_id) = self.execution_evm_version {
            EvmFactoryFor::<FEN>::set_execution_spec(ecx, spec_id);
        }

        let gas = Gas::new(input.gas_limit());
        let curr_depth = ecx.journal().depth();
        self.start_created_accounts_frame(
            curr_depth == 0,
            CreatedAccountsFrameKind::Create,
            curr_depth,
        );

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
                charged_create_state_gas: input.charged_create_state_gas(),
            });
        }

        // Apply our prank
        if let Some(prank) = self.get_prank(curr_depth)
            && let Some(changes) = prank.changes_for(curr_depth, input.caller())
        {
            if let Some(new_caller) = changes.caller {
                // Ensure new caller is loaded and touched
                let _ = journaled_account(ecx, new_caller);
                input.set_caller(new_caller);
            }
            if let Some(new_origin) = changes.origin {
                ecx.tx_mut().set_caller(new_origin);
            }
            if let Some(used) = changes.used {
                self.pranks.insert(curr_depth, used);
            }
        }

        // Apply EIP-2930 access list
        self.apply_accesslist(ecx);

        // Apply our broadcast
        if let Some(broadcast) = &mut self.broadcast
            && curr_depth >= broadcast.depth
            && input.caller() == broadcast.original_caller
        {
            if let Err(err) = ecx.journal_mut().load_account(broadcast.new_origin) {
                return Some(CreateOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: Error::encode(err),
                        gas,
                    },
                    address: None,
                    charged_create_state_gas: input.charged_create_state_gas(),
                });
            }

            ecx.tx_mut().set_caller(broadcast.new_origin);

            if curr_depth == broadcast.depth || broadcast.deploy_from_code {
                // Reset deploy from code flag for upcoming calls;
                broadcast.deploy_from_code = false;

                input.set_caller(broadcast.new_origin);

                let rpc = ecx.db().active_fork_url();
                let fee_token = ecx.tx().fee_token();
                let account = &ecx.journal().evm_state()[&broadcast.new_origin];
                let mut tx_req = TransactionRequestFor::<FEN>::default()
                    .with_from(broadcast.new_origin)
                    .with_kind(TxKind::Create)
                    .with_value(input.value())
                    .with_input(input.init_code())
                    .with_nonce(account.info.nonce);
                if let Some(fee_token) = fee_token {
                    tx_req.set_fee_token(fee_token);
                }
                self.broadcastable_transactions.push_back(BroadcastableTransaction {
                    rpc,
                    transaction: TransactionMaybeSigned::new(tx_req),
                });

                input.log_debug(self, &input.scheme().unwrap_or(CreateScheme::Create));
            }
        }

        // Allow cheatcodes from the address of the new contract
        let address = input.allow_cheatcodes(self, ecx);

        self.record_created_account(ecx.db().active_fork_id(), address);

        // If `recordAccountAccesses` has been called, record the create
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            recorded_account_diffs_stack.push(vec![AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: ecx.db().active_fork_id().unwrap_or_default(),
                    chainId: U256::from(ecx.cfg().chain_id()),
                },
                accessor: input.caller(),
                account: address,
                kind: crate::Vm::AccountAccessKind::Create,
                initialized: true,
                oldBalance: U256::ZERO, // updated on create_end
                newBalance: U256::ZERO, // updated on create_end
                oldNonce: 0,            // new contract starts with nonce 0
                newNonce: 1,            // updated on create_end (contracts start with nonce 1)
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

    fn create_end(
        &mut self,
        ecx: &mut FoundryContextFor<'_, FEN>,
        call: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        let isolated_snapshot_gas_used = self
            .gas_metering
            .take_isolated_snapshot_gas_used(ecx.journal().depth(), &outcome.result.gas);
        let call = Some(call);
        let curr_depth = ecx.journal().depth();

        self.finish_created_accounts_frame(
            outcome.result.is_ok(),
            CreatedAccountsFrameKind::Create,
            curr_depth,
        );

        // Clean up pranks
        if let Some(prank) = &self.get_prank(curr_depth)
            && curr_depth == prank.depth
        {
            ecx.tx_mut().set_caller(prank.prank_origin);

            // Clean single-call prank once we have returned to the original depth
            if prank.single_call {
                self.pranks.remove(&curr_depth);
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = &self.broadcast
            && curr_depth == broadcast.depth
        {
            ecx.tx_mut().set_caller(broadcast.original_origin);

            // Clean single-call broadcast once we have returned to the original depth
            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts.
        if let Some(expected_revert) = &mut self.expected_revert {
            // Record the would-be deployed address as the reverter, picking the innermost
            // reverting CREATE: this hook runs at every depth, the deepest frame fires
            // first, and the `is_none()` lock pins it. For `count > 1` the lock is
            // released after each successful iteration (see below) so each iteration
            // independently records its own innermost CREATE.
            //
            // This intentionally differs from `call_end` for `count > 1`, where
            // legacy nested CALL handling reports the outermost call per iteration.
            //
            // `outcome.address` is `None` for pre-frame rejection (depth/balance/nonce);
            // in that case the surrounding `call_end` records the caller as the reverter.
            if outcome.result.is_revert()
                && expected_revert.reverter.is_some()
                && expected_revert.reverted_by.is_none()
                && let Some(addr) = outcome.address
            {
                expected_revert.reverted_by = Some(addr);
            }

            if curr_depth <= expected_revert.depth
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
                            // Reset so the next iteration's innermost CREATE wins again.
                            expected_revert.reverted_by = None;
                            self.expected_revert = Some(expected_revert.clone());
                        }

                        outcome.result.result = InstructionResult::Return;
                        outcome.result.output = retdata;
                        outcome.address = address;
                        self.gas_metering.last_frame_gas = None;
                    }
                    Err(err) => {
                        outcome.result.result = InstructionResult::Revert;
                        outcome.result.output = err.abi_encode().into();
                    }
                };
            }
        }

        if curr_depth > 0 {
            // Record the gas usage of the create frame, this allows the `lastFrameGas` cheatcode to
            // retrieve the gas usage of the last call or create.
            self.gas_metering.last_frame_gas = Some(frame_gas(&outcome.result));
            self.gas_metering.last_frame_snapshot_gas_used =
                isolated_snapshot_gas_used.unwrap_or_else(|| outcome.result.gas.total_gas_spent());
        }

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any.
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack
            && let Some(mut last_depth) = recorded_account_diffs_stack.pop()
        {
            // Update the reverted status of all deeper calls if this call reverted, in
            // accordance with EVM behavior.
            if outcome.result.is_revert() {
                mark_account_accesses_reverted(&mut last_depth);
            }

            if let Some(create_access) = last_depth.first_mut() {
                // Update post-create state only if recording began before this frame.
                if create_access.depth == curr_depth as u64 {
                    debug_assert_eq!(
                        create_access.kind as u8,
                        crate::Vm::AccountAccessKind::Create as u8
                    );
                    if let Some(address) = outcome.address
                        && let Ok(created_acc) = ecx.journal_mut().load_account(address)
                    {
                        create_access.newBalance = created_acc.data.info.balance;
                        create_access.newNonce = created_acc.data.info.nonce;
                        create_access.deployedCode =
                            created_acc.data.info.code.clone().unwrap_or_default().original_bytes();
                    }
                }
            }
            merge_recorded_frame(recorded_account_diffs_stack, last_depth);
        }

        // Match the create against expected_creates
        if !self.expected_creates.is_empty()
            && let (Some(address), Some(call)) = (outcome.address, call)
            && let Ok(created_acc) = ecx.journal_mut().load_account(address)
        {
            let bytecode = created_acc.data.info.code.clone().unwrap_or_default().original_bytes();
            expect::observe_create(
                &mut self.expected_creates,
                call.caller(),
                || call.scheme().into(),
                &bytecode,
            );
        }
    }
}

impl<FEN: FoundryEvmNetwork> InspectorExt for Cheatcodes<FEN> {
    fn should_use_create2_factory(&mut self, depth: usize, inputs: &CreateInputs) -> bool {
        // `deployCode` executes its create frame in a nested EVM one level deeper, so match it
        // at the depth of the cheatcode call, as for native creates.
        let depth =
            if self.deploy_code_depth.is_some_and(|d| d + 1 == depth) { depth - 1 } else { depth };
        let target_depth = if let Some(prank) = &self.get_prank(depth) {
            prank.depth
        } else if let Some(broadcast) = &self.broadcast {
            broadcast.depth
        } else {
            1
        };

        if depth != target_depth {
            return false;
        }

        match inputs.scheme() {
            CreateScheme::Create2 { .. } => {
                self.broadcast.is_some() || self.config.always_use_create_2_factory
            }
            CreateScheme::Create => self.config.batch_rewrite_creates && self.broadcast.is_some(),
            _ => false,
        }
    }

    fn create2_deployer(&self) -> Address {
        self.config.evm_opts.create2_deployer
    }
}

impl<FEN: FoundryEvmNetwork> Cheatcodes<FEN> {
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
    fn meter_gas_record(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        if interpreter.bytecode.action.as_ref().and_then(|i| i.instruction_result()).is_none() {
            let curr_depth = ecx.journal().depth();
            let isolated_region_gas = match self.gas_metering.pending_isolated_region_gas {
                Some((depth, charged, used)) if depth == curr_depth => {
                    self.gas_metering.pending_isolated_region_gas = None;
                    Some((charged, used))
                }
                _ => None,
            };
            self.gas_metering.gas_records.iter_mut().for_each(|record| {
                if curr_depth == record.depth {
                    // Skip the first opcode of the first call frame as it includes the gas cost of
                    // creating the snapshot.
                    if self.gas_metering.last_gas_used != 0 {
                        let mut gas_diff = interpreter
                            .gas
                            .total_gas_spent()
                            .saturating_sub(self.gas_metering.last_gas_used);
                        if let Some((charged, used)) = isolated_region_gas {
                            gas_diff = gas_diff.saturating_sub(charged).saturating_add(used);
                        }
                        record.gas_used = record.gas_used.saturating_add(gas_diff);
                    }

                    // Update `last_gas_used` to the current spent gas for the next iteration to
                    // compare against.
                    self.gas_metering.last_gas_used = interpreter.gas.total_gas_spent();
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
    const fn meter_gas_reset(&mut self, interpreter: &mut Interpreter) {
        let mut gas = Gas::new(interpreter.gas.limit());
        gas.memory_mut().words_num = interpreter.gas.memory().words_num;
        gas.memory_mut().expansion_cost = interpreter.gas.memory().expansion_cost;
        interpreter.gas = gas;
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
            if interpreter.gas.total_gas_spent()
                < u64::try_from(interpreter.gas.refunded()).unwrap_or_default()
            {
                interpreter.gas = Gas::new(interpreter.gas.limit());
            }
        }
    }

    /// Applies opcode-level overrides for `BASEFEE`, `GASPRICE` and `BLOBHASH`.
    ///
    /// Called from `step_end` *after* the opcode has executed and only when the
    /// opcode succeeded (the caller checks `instruction_result`). The opcode
    /// pushed its (possibly zeroed) result onto the stack; we replace the top
    /// of stack with the cheatcode-set override. This is what makes `vm.fee`,
    /// `vm.txGasPrice` and `vm.blobhashes` visible to called contracts under
    /// `--isolate` / `--gas-report`, where the inner transaction zeroes the
    /// real fee fields for fee-accounting purposes.
    ///
    /// We can't read the just-executed opcode from `interpreter.bytecode.opcode()`
    /// here because the PC has already advanced; instead `step` stashes it in
    /// `env_overrides.pending_opcode` for us.
    #[cold]
    fn apply_env_overrides(&mut self, interpreter: &mut Interpreter, fork_id: Option<U256>) {
        let Some(env_overrides) = self.env_overrides.get_mut(fork_id) else { return };
        let Some(opcode) = env_overrides.pending_opcode.take() else { return };
        // Each overridden opcode pushed one value; replace it with the override.
        let value = match opcode {
            op::BASEFEE => env_overrides.basefee_override().map(U256::from),
            op::GASPRICE => env_overrides.gas_price_override().map(U256::from),
            // BLOBHASH popped the index captured in `step` and pushed the hash.
            op::BLOBHASH => env_overrides
                .pending_blobhash_index
                .take()
                .and_then(|index| env_overrides.blob_hash_override(index))
                .map(Into::into),
            _ => None,
        };
        if let Some(value) = value {
            Self::replace_top_of_stack(interpreter, value);
        }
    }

    /// Replaces the top of the interpreter stack with `value`.
    ///
    /// The caller must only invoke this after a successful opcode that pushed
    /// a value onto the stack; the `pop()` is therefore expected to succeed.
    /// If it does not (e.g. because of a bug in the caller's success gating)
    /// we bail out instead of pushing on top of an unexpected stack, which
    /// would silently grow the stack and corrupt the frame.
    fn replace_top_of_stack(interpreter: &mut Interpreter, value: U256) {
        if interpreter.stack.pop().is_err() {
            debug_assert!(false, "env override expected opcode result on stack");
            return;
        }
        let _ = interpreter.stack.push(value);
    }

    /// Generates or copies arbitrary values for storage slots.
    /// Invoked in inspector `step_end` (when the current opcode is not executed), if current opcode
    /// to execute is `SLOAD` and storage slot is cold.
    /// Ensures that in next step (when `SLOAD` opcode is executed) an arbitrary value is returned:
    /// - copies the existing arbitrary storage value (or the new generated one if no value in
    ///   cache) from mapped source address to the target address.
    /// - generates arbitrary value and saves it in target address storage.
    #[cold]
    fn arbitrary_storage_end(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        let (key, target_address) = if interpreter.bytecode.opcode() == op::SLOAD {
            (try_or_return!(interpreter.stack.peek(0)), interpreter.input.target_address)
        } else {
            return;
        };

        if self.is_arbitrary_storage_slot_explicit(target_address, key) {
            return;
        }

        let Some(value) = ecx.sload(target_address, key) else {
            return;
        };

        if (value.is_cold && value.data.is_zero())
            || self.should_overwrite_arbitrary_storage(&target_address, key)
        {
            if self.has_arbitrary_storage(&target_address) {
                let arbitrary_value = self
                    .cached_arbitrary_storage_value(target_address, key)
                    .unwrap_or_else(|| self.rng().random());
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

    /// Restores parent interpreter state after a synthetic storage-hook callback.
    ///
    /// Returns whether a failed callback was propagated to the parent frame.
    #[inline]
    pub fn finish_storage_hook_callback(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) -> bool {
        let Some(active) = self.active_storage_hook.as_ref() else { return false };
        let Some((result, output)) = active.outcome.clone() else { return false };

        let active = self.active_storage_hook.take().expect("active storage hook exists");
        Self::restore_storage_hook_access(ecx, active.journal_start);
        self.restore_storage_hook_inspector_state(active.inspector_state);
        let _ = interpreter.stack.pop();
        if let Some(item) = active.saved_stack_item {
            let result = interpreter.stack.push(item);
            debug_assert!(result, "reserved storage-hook stack slot must be available");
        }
        interpreter.gas = active.saved_gas;
        interpreter.return_data.set_buffer(active.saved_return_data);

        if result.is_ok() {
            false
        } else {
            interpreter.bytecode.set_action(InterpreterAction::new_return(
                InstructionResult::Revert,
                output,
                interpreter.gas,
            ));
            true
        }
    }

    fn take_storage_hook_inspector_state(&mut self) -> StorageHookInspectorState {
        StorageHookInspectorState {
            accesses: std::mem::take(&mut self.accesses),
            recording_accesses: std::mem::replace(&mut self.recording_accesses, false),
            mapping_slots: self.mapping_slots.take(),
            recorded_logs: self.recorded_logs.take(),
            mocked_calls: std::mem::take(&mut self.mocked_calls),
            mocked_functions: std::mem::take(&mut self.mocked_functions),
            expected_revert: self.expected_revert.take(),
            assume_no_revert: self.assume_no_revert.take(),
            expected_calls: std::mem::take(&mut self.expected_calls),
            expected_emits: std::mem::take(&mut self.expected_emits),
            expected_creates: std::mem::take(&mut self.expected_creates),
        }
    }

    fn restore_storage_hook_inspector_state(&mut self, state: StorageHookInspectorState) {
        self.accesses = state.accesses;
        self.recording_accesses = state.recording_accesses;
        self.mapping_slots = state.mapping_slots;
        self.recorded_logs = state.recorded_logs;
        self.mocked_calls = state.mocked_calls;
        self.mocked_functions = state.mocked_functions;
        self.expected_revert = state.expected_revert;
        self.assume_no_revert = state.assume_no_revert;
        self.expected_calls = state.expected_calls;
        self.expected_emits = state.expected_emits;
        self.expected_creates = state.expected_creates;
    }

    fn restore_storage_hook_access(ecx: &mut FoundryContextFor<'_, FEN>, journal_start: usize) {
        let (_, journal) = ecx.db_journal_inner_mut();
        let entries =
            journal.journal.drain(journal_start.min(journal.journal.len())..).collect_vec();
        for entry in entries {
            match entry {
                JournalEntry::AccountWarmed { address } => {
                    journal.state.get_mut(&address).expect("warmed account exists").mark_cold();
                }
                JournalEntry::StorageWarmed { address, key } => {
                    // TODO(@mablr): Preserve the EIP-2200 `original_value` when bumping the REVM
                    // family to 42. REVM 41's `mark_cold` resets it for a slot first warmed and
                    // modified by the callback.
                    journal
                        .state
                        .get_mut(&address)
                        .expect("warmed account exists")
                        .storage
                        .get_mut(&key)
                        .expect("warmed storage slot exists")
                        .mark_cold();
                }
                entry => journal.journal.push(entry),
            }
        }
    }

    fn capture_storage_hook(
        &mut self,
        interpreter: &Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        self.pending_storage_hook = None;
        if self.active_storage_hook.is_some() {
            return;
        }
        let account = interpreter.input.target_address;
        match interpreter.bytecode.opcode() {
            op::SLOAD => {
                let slot = try_or_return!(interpreter.stack.peek(0));
                let Some(hook) = self.storage_load_hooks.get(&account).copied() else { return };
                self.pending_storage_hook = Some(PendingStorageHook::Load { account, slot, hook });
            }
            op::SSTORE => {
                let slot = try_or_return!(interpreter.stack.peek(0));
                let (hook, mapping) = if let Some(hook) = self.storage_store_hooks.get(&account) {
                    (*hook, None)
                } else {
                    let Some(provenance) = self
                        .storage_hook_mapping_slots
                        .get(&account)
                        .and_then(|slots| slots.resolve(slot.into()))
                    else {
                        return;
                    };
                    let Some(hook) = self
                        .mapping_storage_store_hooks
                        .get(&account)
                        .and_then(|hooks| hooks.get(&provenance.root_slot))
                        .copied()
                    else {
                        return;
                    };
                    (hook, Some((provenance.root_slot, provenance.keys)))
                };
                let checkpoint = ecx.journal_mut().checkpoint();
                let old_value =
                    ecx.sload(account, slot).map(|value| value.data).unwrap_or_default();
                ecx.journal_mut().checkpoint_revert(checkpoint);
                self.pending_storage_hook =
                    Some(PendingStorageHook::Store { account, slot, old_value, mapping, hook });
            }
            _ => {}
        }
    }

    fn invoke_pending_storage_hook(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        let Some(pending) = self.pending_storage_hook.take() else { return };
        if interpreter
            .bytecode
            .action
            .as_ref()
            .and_then(InterpreterAction::instruction_result)
            .is_some()
        {
            return;
        }

        let (hook, input, saved_stack_item) = match pending {
            PendingStorageHook::Load { account, slot, hook } => {
                let value = try_or_return!(interpreter.stack.peek(0));
                let mut input = Vec::with_capacity(4 + 32 * 3);
                input.extend_from_slice(&hook.callback_selector);
                input.extend_from_slice(account.into_word().as_slice());
                input.extend_from_slice(&slot.to_be_bytes::<32>());
                input.extend_from_slice(&value.to_be_bytes::<32>());
                (hook, Bytes::from(input), Some(value))
            }
            PendingStorageHook::Store { account, slot, old_value, mapping, hook } => {
                let new_value =
                    ecx.sload(account, slot).map(|value| value.data).unwrap_or_default();
                let mut input = Vec::with_capacity(4 + 32 * 4);
                input.extend_from_slice(&hook.callback_selector);
                input.extend_from_slice(account.into_word().as_slice());
                input.extend_from_slice(&slot.to_be_bytes::<32>());
                if let Some((root, keys)) = mapping {
                    input.extend_from_slice(root.as_slice());
                    input.extend_from_slice(&U256::from(32 * 6).to_be_bytes::<32>());
                    input.extend_from_slice(&old_value.to_be_bytes::<32>());
                    input.extend_from_slice(&new_value.to_be_bytes::<32>());
                    input.extend_from_slice(&U256::from(keys.len()).to_be_bytes::<32>());
                    for key in keys {
                        input.extend_from_slice(key.as_slice());
                    }
                } else {
                    input.extend_from_slice(&old_value.to_be_bytes::<32>());
                    input.extend_from_slice(&new_value.to_be_bytes::<32>());
                }
                (hook, Bytes::from(input), None)
            }
        };

        let journal_start = ecx.db_journal_inner_mut().1.journal.len();
        let account = match ecx.journal_mut().load_account_with_code(hook.callback_target) {
            Ok(account) => account,
            Err(err) => {
                interpreter.bytecode.set_action(InterpreterAction::new_return(
                    InstructionResult::Revert,
                    Error::encode(err),
                    interpreter.gas,
                ));
                return;
            }
        };
        let known_bytecode =
            (account.info.code_hash(), account.info.code.clone().unwrap_or_default());
        let saved_gas = interpreter.gas;
        let saved_return_data = Bytes::copy_from_slice(interpreter.return_data.buffer());
        let gas_limit = interpreter.gas.remaining();
        let parent_depth = ecx.journal().depth();
        if saved_stack_item.is_some() {
            let result = interpreter.stack.pop();
            debug_assert!(result.is_ok(), "captured SLOAD result must be on the stack");
        }
        let inspector_state = self.take_storage_hook_inspector_state();

        self.active_storage_hook = Some(ActiveStorageHook {
            parent_depth,
            callback_target: hook.callback_target,
            callback_input: input.clone(),
            saved_gas,
            saved_return_data,
            saved_stack_item,
            journal_start,
            inspector_state,
            outcome: None,
        });
        interpreter.bytecode.set_action(InterpreterAction::NewFrame(FrameInput::Call(Box::new(
            CallInputs {
                input: CallInput::Bytes(input),
                return_memory_offset: 0..0,
                gas_limit,
                reservoir: 0,
                bytecode_address: hook.callback_target,
                known_bytecode,
                target_address: hook.callback_target,
                caller: CHEATCODE_ADDRESS,
                value: CallValue::Transfer(U256::ZERO),
                scheme: CallScheme::Call,
                is_static: false,
                charged_new_account_state_gas: false,
            },
        ))));
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
    fn record_state_diffs(
        &mut self,
        interpreter: &mut Interpreter,
        ecx: &mut FoundryContextFor<'_, FEN>,
    ) {
        let Some(account_accesses) = &mut self.recorded_account_diffs_stack else { return };
        match interpreter.bytecode.opcode() {
            op::SELFDESTRUCT => {
                // Ensure that we're not selfdestructing a context recording was initiated on
                let Some(last) = account_accesses.last_mut() else { return };

                // get previous balance, nonce and initialized status of the target account
                let target = try_or_return!(interpreter.stack.peek(0));
                let target = Address::from_word(B256::from(target));
                let (initialized, old_balance, old_nonce) = ecx
                    .journal_mut()
                    .load_account(target)
                    .map(|account| {
                        (
                            account.data.info.exists(),
                            account.data.info.balance,
                            account.data.info.nonce,
                        )
                    })
                    .unwrap_or_default();

                // load balance of this account
                let value = ecx
                    .balance(interpreter.input.target_address)
                    .map(|b| b.data)
                    .unwrap_or(U256::ZERO);

                // register access for the target account
                last.push(crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo {
                        forkId: ecx.db().active_fork_id().unwrap_or_default(),
                        chainId: U256::from(ecx.cfg().chain_id()),
                    },
                    accessor: interpreter.input.target_address,
                    account: target,
                    kind: crate::Vm::AccountAccessKind::SelfDestruct,
                    initialized,
                    oldBalance: old_balance,
                    newBalance: old_balance + value,
                    oldNonce: old_nonce,
                    newNonce: old_nonce, // nonce doesn't change on selfdestruct
                    value,
                    data: Bytes::new(),
                    reverted: false,
                    deployedCode: Bytes::new(),
                    storageAccesses: vec![],
                    depth: ecx
                        .journal()
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
                // it's not set (zero value). Revert the checkpoint so this read does not warm the
                // slot for the actual SLOAD opcode.
                let checkpoint = ecx.journal_mut().checkpoint();
                let present_value =
                    ecx.sload(address, key).map(|previous| previous.data).unwrap_or_default();
                ecx.journal_mut().checkpoint_revert(checkpoint);
                let access = crate::Vm::StorageAccess {
                    account: interpreter.input.target_address,
                    slot: key.into(),
                    isWrite: false,
                    previousValue: present_value.into(),
                    newValue: present_value.into(),
                    reverted: false,
                };
                let curr_depth =
                    ecx.journal().depth().try_into().expect("journaled state depth exceeds u64");
                append_storage_access(last, access, curr_depth);
            }
            op::SSTORE => {
                let Some(last) = account_accesses.last_mut() else { return };

                let key = try_or_return!(interpreter.stack.peek(0));
                let value = try_or_return!(interpreter.stack.peek(1));
                let address = interpreter.input.target_address;
                // Try to load the account and the slot's previous value, otherwise, assume it's
                // not set (zero value). Revert the checkpoint so this read does not warm the slot
                // for the actual SSTORE opcode.
                let checkpoint = ecx.journal_mut().checkpoint();
                let previous_value =
                    ecx.sload(address, key).map(|previous| previous.data).unwrap_or_default();
                ecx.journal_mut().checkpoint_revert(checkpoint);

                let access = crate::Vm::StorageAccess {
                    account: address,
                    slot: key.into(),
                    isWrite: true,
                    previousValue: previous_value.into(),
                    newValue: value.into(),
                    reverted: false,
                };
                let curr_depth =
                    ecx.journal().depth().try_into().expect("journaled state depth exceeds u64");
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
                let checkpoint = ecx.journal_mut().checkpoint();
                let (initialized, balance, nonce) = ecx
                    .journal_mut()
                    .load_account(address)
                    .map(|acc| (acc.data.info.exists(), acc.data.info.balance, acc.data.info.nonce))
                    .unwrap_or_default();
                ecx.journal_mut().checkpoint_revert(checkpoint);
                let curr_depth =
                    ecx.journal().depth().try_into().expect("journaled state depth exceeds u64");
                let account_access = crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo {
                        forkId: ecx.db().active_fork_id().unwrap_or_default(),
                        chainId: U256::from(ecx.cfg().chain_id()),
                    },
                    accessor: interpreter.input.target_address,
                    account: address,
                    kind,
                    initialized,
                    oldBalance: balance,
                    newBalance: balance,
                    oldNonce: nonce,
                    newNonce: nonce, // EXT* operations don't change nonce
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
                                // CALL has not expanded input memory yet.
                                if args_size >= SELECTOR_LEN
                                    && args_offset.saturating_add(args_size) <= interpreter.memory.size()
                                {
                                    let memory_word = interpreter.memory.slice_len(args_offset, args_size);
                                    if memory_word[..SELECTOR_LEN] == stopExpectSafeMemoryCall::SELECTOR {
                                        return
                                    }
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

    #[cold]
    fn set_gas_limit_type(&mut self, interpreter: &mut Interpreter) {
        match interpreter.bytecode.opcode() {
            op::CREATE2 => self.dynamic_gas_limit = true,
            op::CALL => {
                // If first element of the stack is close to current remaining gas then assume
                // dynamic gas limit.
                self.dynamic_gas_limit =
                    try_or_return!(interpreter.stack.peek(0)) >= interpreter.gas.remaining() - 100
            }
            _ => self.dynamic_gas_limit = false,
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
        ranges.iter().map(|r| format!("[0x{:02X}, 0x{:02X})", r.start, r.end)).join(" U ")
    );

    interpreter.bytecode.set_action(InterpreterAction::new_return(
        InstructionResult::Revert,
        Bytes::from(revert_string.into_bytes()),
        interpreter.gas,
    ));
}

/// Returns true if the kind of account access is a call.
const fn access_is_call(kind: crate::Vm::AccountAccessKind) -> bool {
    matches!(
        kind,
        crate::Vm::AccountAccessKind::Call
            | crate::Vm::AccountAccessKind::StaticCall
            | crate::Vm::AccountAccessKind::CallCode
            | crate::Vm::AccountAccessKind::DelegateCall
    )
}

/// Records a log into the recorded logs vector, if it exists.
fn record_logs(recorded_logs: &mut Option<Vec<Vm::Log>>, log: &Log) {
    if let Some(storage_recorded_logs) = recorded_logs {
        storage_recorded_logs.push(Vm::Log {
            topics: log.data.topics().to_vec(),
            data: log.data.data.clone(),
            emitter: log.address,
        });
    }
}

/// Returns the [`spec::Cheatcode`] definition for a given [`spec::CheatcodeDef`] implementor.
const fn cheatcode_of<T: spec::CheatcodeDef>(_: &T) -> &'static spec::Cheatcode<'static> {
    T::CHEATCODE
}

fn cheatcode_name(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheatcode_signature(cheat).split('(').next().unwrap()
}

const fn cheatcode_id(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheat.func.id
}

const fn cheatcode_signature(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheat.func.signature
}

/// Dispatches the cheatcode call to the appropriate function.
fn apply_dispatch<FEN: FoundryEvmNetwork>(
    calls: &Vm::VmCalls,
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    executor: &mut dyn CheatcodesExecutor<FEN>,
) -> Result {
    // Extract metadata for logging/deprecation via CheatcodeDef.
    macro_rules! get_cheatcode {
        ($($variant:ident),*) => {
            match calls {
                $(Vm::VmCalls::$variant(cheat) => cheatcode_of(cheat),)*
            }
        };
    }
    let cheat = vm_calls!(get_cheatcode);

    let _guard = debug_span!(target: "cheatcodes", "apply", id = %cheatcode_id(cheat)).entered();
    trace!(target: "cheatcodes", cheat = %cheatcode_signature(cheat), "applying");

    if let spec::Status::Deprecated(replacement) = cheat.status {
        ccx.state.deprecated.insert(cheatcode_signature(cheat), replacement);
    }

    // Monomorphized dispatch: calls apply_full directly, no trait objects.
    macro_rules! dispatch {
        ($($variant:ident),*) => {
            match calls {
                $(Vm::VmCalls::$variant(cheat) => Cheatcode::apply_full(cheat, ccx, executor),)*
            }
        };
    }
    let mut result = if ccx.state.config.blocked_cheatcodes.contains(&cheat.func.selector_bytes) {
        Err(fmt_err!("disabled during restricted execution"))
    } else {
        vm_calls!(dispatch)
    };

    // Format the error message to include the cheatcode name.
    if let Err(e) = &mut result
        && e.is_str()
    {
        let name = cheatcode_name(cheat);
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

/// Increments the nonce of every authority whose authorization would be applied on-chain.
///
/// Mirrors EIP-7702 processing: authorizations are checked in order after the transaction has
/// incremented the sender nonce, and invalid authorizations are skipped without changing the
/// authority nonce.
fn apply_authorization_nonces<FEN: FoundryEvmNetwork>(
    ecx: &mut FoundryContextFor<'_, FEN>,
    authorizations: &[SignedAuthorization],
    sender: Address,
    chain_id: u64,
) -> Result<()> {
    for auth in authorizations {
        if (!auth.chain_id.is_zero() && auth.chain_id != U256::from(chain_id))
            || auth.nonce() == u64::MAX
        {
            continue;
        }
        let Ok(authority) = auth.recover_authority() else { continue };
        // The authority code check is skipped because attaching the delegation already replaced
        // the local code that EIP-7702 validates.
        let account = journaled_account(ecx, authority)?;
        // The sender nonce has not been incremented for the transaction yet.
        if auth.nonce() == account.info.nonce + u64::from(authority == sender) {
            account.info.nonce += 1;
        }
    }
    Ok(())
}

/// Helper function to check if frame execution will exit.
const fn will_exit(action: &InterpreterAction) -> bool {
    match action {
        InterpreterAction::Return(result) => {
            result.result.is_ok_or_revert() || result.result.is_halt()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use env_overrides::EnvOverrides;

    fn cheats(flag: bool, broadcast: Option<Broadcast>) -> Cheatcodes {
        let config = CheatsConfig { batch_rewrite_creates: flag, ..Default::default() };
        let mut cheats = Cheatcodes::new(Arc::new(config));
        cheats.broadcast = broadcast;
        cheats
    }

    fn create_inputs() -> CreateInputs {
        CreateInputs::new(Address::ZERO, CreateScheme::Create, U256::ZERO, Bytes::new(), 100_000, 0)
    }

    fn broadcast_at(depth: usize) -> Broadcast {
        Broadcast { depth, ..Default::default() }
    }

    #[test]
    fn flag_off_with_broadcast_returns_false() {
        let mut cheats = cheats(false, Some(broadcast_at(1)));
        assert!(!cheats.should_use_create2_factory(1, &create_inputs()));
    }

    #[test]
    fn flag_on_without_broadcast_returns_false() {
        let mut cheats = cheats(true, None);
        assert!(!cheats.should_use_create2_factory(1, &create_inputs()));
    }

    #[test]
    fn flag_on_with_broadcast_depth_mismatch_returns_false() {
        let mut cheats = cheats(true, Some(broadcast_at(2)));
        assert!(!cheats.should_use_create2_factory(1, &create_inputs()));
    }

    #[test]
    fn flag_on_with_broadcast_depth_match_returns_true() {
        let mut cheats = cheats(true, Some(broadcast_at(1)));
        assert!(cheats.should_use_create2_factory(1, &create_inputs()));
    }

    #[test]
    fn default_cheatcodes_have_no_opcode_hooks() {
        let cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());
        assert!(!cheats.has_step_hooks());
        assert!(!cheats.has_step_end_hooks());
        assert!(!cheats.has_log_hooks());
    }

    #[test]
    fn active_cheatcode_state_enables_opcode_hooks() {
        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());

        cheats.recording_accesses = true;
        assert!(cheats.has_step_hooks());
        assert!(!cheats.has_step_end_hooks());
        assert!(cheats.has_recording_accesses_only_step_hook());

        cheats.recording_accesses = false;
        cheats.gas_metering.touched = true;
        assert!(!cheats.has_step_hooks());
        assert!(cheats.has_step_end_hooks());
        assert!(!cheats.has_recording_accesses_only_step_hook());

        cheats.gas_metering.touched = false;
        cheats.register_storage_load_hook(Address::ZERO, Address::ZERO, [0; 4]);
        assert!(cheats.has_step_hooks());
        assert!(cheats.has_step_end_hooks());
        assert!(!cheats.has_recording_accesses_only_step_hook());
    }

    #[test]
    fn mixed_step_hooks_disable_record_access_fast_path() {
        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());
        cheats.recording_accesses = true;

        cheats.gas_metering.reset = true;
        assert!(!cheats.has_recording_accesses_only_step_hook());

        cheats.gas_metering.reset = false;
        cheats.env_overrides.update(None, |o| o.basefee = Some(1));
        assert!(!cheats.has_recording_accesses_only_step_hook());
    }

    #[test]
    fn env_override_hook_predicates() {
        fn assert_hooks(cheats: &Cheatcodes, active: bool, case: &str) {
            assert_eq!(cheats.has_step_hooks(), active, "step hooks: {case}");
            assert_eq!(cheats.has_step_end_hooks(), active, "step_end hooks: {case}");
        }

        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());
        assert_hooks(&cheats, false, "empty map");
        let snapshot_id = U256::from(7);
        cheats.env_overrides.save_snapshot(snapshot_id, None, 0, 0, &[]);

        cheats.env_overrides.update(None, |_| {});
        assert_hooks(&cheats, false, "inactive entry");

        let active = [
            EnvOverrides { basefee: Some(1), ..Default::default() },
            EnvOverrides { implicit_basefee: Some(1), ..Default::default() },
            EnvOverrides { gas_price: Some(1), ..Default::default() },
            EnvOverrides { blob_hashes: Some(vec![B256::ZERO]), ..Default::default() },
        ];
        for overrides in active {
            let case = format!("{overrides:?}");
            cheats.env_overrides.update(None, |o| *o = overrides);
            assert_hooks(&cheats, true, &case);
        }

        // Overrides on a fork that isn't active still enable the hooks.
        cheats.env_overrides.restore_snapshot(snapshot_id, false);
        cheats.env_overrides.update(Some(U256::from(1)), |o| o.basefee = Some(1));
        assert_hooks(&cheats, true, "override on another fork");

        // Restoring a snapshot taken without overrides turns the hooks off.
        cheats.env_overrides.restore_snapshot(snapshot_id, false);
        assert_hooks(&cheats, false, "after restoring an empty snapshot");
    }

    #[test]
    fn active_log_state_enables_log_hooks() {
        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());

        cheats.recorded_logs = Some(Default::default());
        assert!(cheats.has_log_hooks());

        cheats.recorded_logs = None;
        cheats.expected_emits.push_back((
            crate::expected_emit::ExpectedEmit {
                depth: 0,
                log: None,
                checks: [false; 5],
                address: None,
                anonymous: false,
                found: false,
                count: 1,
                mismatch_error: None,
            },
            Default::default(),
        ));
        assert!(cheats.has_log_hooks());
    }

    #[test]
    fn frame_gas_reports_settled_components() {
        for mut gas in [Gas::new(100_000), Gas::new_with_regular_gas_and_reservoir(100_000, 50_000)]
        {
            assert!(gas.record_regular_cost(1_000));
            assert!(gas.record_state_cost(20_000));

            let mut result = InterpreterResult::new(InstructionResult::Stop, Bytes::new(), gas);
            let reported = frame_gas(&result);
            assert_eq!(reported.gasTotalUsed, 1_000);
            assert_eq!(reported.gasStateUsed, 20_000);

            result.result = InstructionResult::Revert;
            assert_eq!(frame_gas(&result).gasStateUsed, 0);
        }

        let mut gas = Gas::new(100_000);
        gas.refill_reservoir(20_000);
        let result = InterpreterResult::new(InstructionResult::Stop, Bytes::new(), gas);
        assert_eq!(frame_gas(&result).gasStateUsed, -20_000);

        let mut gas = Gas::new(100_000);
        assert!(gas.record_state_cost(20_000));
        gas.spend_all();
        let result = InterpreterResult::new(InstructionResult::OutOfGas, Bytes::new(), gas);
        let reported = frame_gas(&result);
        assert_eq!(reported.gasTotalUsed, 100_000);
        assert_eq!(reported.gasStateUsed, 0);
    }

    #[test]
    fn arbitrary_storage_cache_value_routes_copied_targets_to_source() {
        let mut storage = ArbitraryStorage::default();
        let source = Address::repeat_byte(0x11);
        let copied = Address::repeat_byte(0x22);
        let slot = U256::from(7);

        storage.mark_arbitrary(&source, false);
        storage.mark_copy(&source, &copied);
        storage.cache_value(copied, slot, U256::ZERO);

        assert_eq!(storage.cached_value(source, slot), Some(U256::ZERO));
    }
}
