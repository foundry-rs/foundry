//! Cheatcode EVM inspector.

use crate::{
    Cheatcode, CheatcodesBackend, CheatcodesCore, CheatsConfig, CheatsCtxt, Error, Result,
    Vm::{self, AccountAccess},
    evm::{
        DealRecord, GasRecord, RecordAccess, append_storage_access, mark_account_accesses_reverted,
        merge_recorded_frame,
        mock::{self, MockCallDataContext, MockCallReturnData},
        prank::Prank,
    },
    expected_emit::{self, ExpectedEmitTracker},
    script::Broadcast,
    test::{
        assume::AssumeNoRevert,
        expect::{self, ExpectedCallTracker, ExpectedCreate, ExpectedRevert, ExpectedRevertKind},
        revert_handlers,
    },
    utils::IgnoredTraces,
};
use alloy_consensus::{BlobTransactionSidecarVariant, transaction::Recovered};
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::{Ethereum, Network, TransactionBuilder};
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, hex,
    map::{AddressHashMap, AddressMap, HashMap, HashSet},
};
use alloy_rpc_types::AccessList;
use alloy_sol_types::{SolCall, SolInterface};
use evm2::{
    EvmTypesHost, ExecutionError, InspectorConfig, OpcodeSet, TxResultWithState,
    evm::{State, StateSnapshot},
    interpreter::{
        Gas, GasTracker, Host, InstrStop, Interpreter, MessageExt, MessageKind, MessageResultExt,
        derive_create_destination, op,
    },
};
use eyre::WrapErr;
use foundry_common::{
    FoundryTransactionBuilder, SELECTOR_LEN, TransactionMaybeSigned,
    mapping_slots::{
        MappingSlots, PendingMappingHash, capture_hash as capture_mapping_hash,
        record_hash as record_mapping_hash, step as mapping_step,
    },
};
use foundry_evm_core::{
    Breakpoints,
    abi::Vm::stopExpectSafeMemoryCall,
    backend::{LocalForkId, RevertDiagnostic},
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME},
    evm::{
        BlockEnvFor, EthEvmNetwork, EvmEnvFor, EvmFactoryFor, FoundryEvmFactory, FoundryEvmFor,
        FoundryEvmNetwork, InterpreterFor, MessageFor, MessageResultFor, PersistentWrite,
        SnapshotRestores, SpecFor, TransactionRequestFor, TxFor, TxResultFor, TypesFor, evm_env,
        release_inspector, restore_access_warmth,
    },
};
use foundry_evm_traces::{
    TracingInspector, TracingInspectorConfig, identifier::SignaturesIdentifier,
};
use itertools::Itertools;
use rand::Rng;
use std::{
    cmp::max,
    collections::{BTreeMap, VecDeque},
    fmt::Debug,
    ops::Range,
    sync::{Arc, OnceLock},
};

mod env_overrides;
pub use env_overrides::EnvOverrideState;

pub mod analysis;
pub use analysis::CheatcodeAnalysis;

/// Runs nested executions on behalf of cheatcodes.
///
/// The executor owns the inspectors besides [`Cheatcodes`], so nested executions are observed by
/// the whole inspector stack: tracing, logs, coverage and cheatcodes themselves.
pub trait CheatcodesExecutor<FEN: FoundryEvmNetwork> {
    /// Executes `message` as a nested frame of the transaction `evm` is running, with `cheats`
    /// active, such as for `deployCode` or storage-hook callbacks.
    fn exec_message(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Result<MessageResultFor<FEN>, ExecutionError>;

    /// Executes `tx` as a nested transaction of the transaction `evm` is running, with `env` or
    /// the running environment, and merges its changes so that they survive reverts of the
    /// enclosing calls, such as for `executeTransaction`. A failed transaction only keeps its nonce
    /// increment.
    ///
    /// `depth` is the depth of the cheatcode call; the nested transaction's root frame duplicates
    /// it, so its children run one level deeper.
    fn nested_transaction(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<TxResultFor<FEN>>;

    /// Replays a mined transaction of fork `fork_id`, or of the active fork, for `vm.transact`.
    ///
    /// The replayed transaction runs with `cheats` active over a throwaway copy of `backend`, so
    /// its cheatcodes cannot modify `backend`. Its root frame is a call below the cheatcode call
    /// at `depth`.
    fn transact(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<TxResultFor<FEN>>;

    /// Executes a signed transaction as a nested transaction, such as for
    /// `broadcastRawTransaction`.
    ///
    /// The transaction runs with `cheats` active over a throwaway copy of `backend`, in `env` or
    /// the running environment for `None`. Its root frame is a call below the cheatcode call at
    /// `depth`.
    fn transact_from_tx(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        depth: u16,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<TxResultFor<FEN>>;

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, msg: &str);

    /// Returns a mutable reference to the tracing inspector if it is available.
    fn tracing_inspector(&mut self) -> Option<&mut TracingInspector> {
        None
    }
}

/// Executes the given CREATE message as a nested frame with the whole inspector stack.
pub(crate) fn exec_create<FEN: FoundryEvmNetwork>(
    executor: &mut dyn CheatcodesExecutor<FEN>,
    mut message: MessageFor<FEN>,
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
) -> Result<MessageResultFor<FEN>> {
    let nonce = ccx.evm.state_mut().account(&message.caller)?.nonce();
    message.destination = derive_create_destination(
        message.kind,
        &message.caller,
        &message.salt,
        &message.input,
        nonce,
    );
    message.call_target = message.destination;
    Ok(executor.exec_message(ccx.state, ccx.backend, ccx.evm, &mut message)?)
}

/// [`CheatcodesExecutor`] that runs nested executions with only the [`Cheatcodes`] inspector.
#[derive(Debug, Default, Clone, Copy)]
struct TransparentCheatcodesExecutor;

impl<FEN: FoundryEvmNetwork> CheatcodesExecutor<FEN> for TransparentCheatcodesExecutor {
    fn exec_message(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Result<MessageResultFor<FEN>, ExecutionError> {
        let env = evm_env(evm);
        // SAFETY: `evm` runs over the lifecycle's owned databases, and so does the frame EVM,
        // which only lives for this call; its inspector is released before its state moves back.
        let mut frame = unsafe { backend.backend().begin_frame_evm(evm, env) };
        frame.set_inspector(CheatcodesInspector { cheats, backend: &mut *backend });
        let result = Host::execute_message(&mut frame, message);
        // SAFETY: See above.
        let frame = unsafe { release_inspector(frame) };
        // SAFETY: See above.
        unsafe { backend.backend().end_frame_evm(evm, frame) };
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
        EvmFactoryFor::<FEN>::default().sync_transaction(evm);
        let mut child = backend.backend().nested_evm(evm, tx, env);
        // The nested transaction's calls do not apply access lists to the running one.
        let applied_access_list = cheats.applied_access_list.take();
        child.set_inspector(CheatcodesInspector { cheats: &mut *cheats, backend: &mut *backend });
        let executed = child.transact(tx).map(|executed| executed.detach());
        let merged = match executed {
            Ok(TxResultWithState { result, pending_state, .. }) => {
                // A failed transaction keeps its nonce increment.
                foundry_evm_core::evm::merge_untracked_pending_changes_with_warmth(
                    evm.state_mut(),
                    &mut child,
                    pending_state,
                    // Without the inspector stack, destroyed accounts keep no pre-destruct state.
                    Default::default(),
                )
                .map(|persistent| (result, persistent))
                .map_err(Into::into)
            }
            Err(err) => Err(eyre::eyre!("{err:?}")),
        };
        drop(child);
        cheats.applied_access_list = applied_access_list;
        let (result, persistent) = merged?;
        cheats.record_persistent(usize::from(depth).saturating_sub(1), evm.state(), persistent);
        // The nested transaction's changes are not the running transaction's execution.
        EvmFactoryFor::<FEN>::default().refresh_transaction(evm);
        Ok(result)
    }

    // Without the inspector stack's nested-root handling, replayed transactions run without
    // cheatcodes, which would take their root frame for a top-level call; only their logs reach
    // `expectEmit` and `recordLogs`.
    fn transact(
        &mut self,
        cheats: &mut Cheatcodes<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        _depth: u16,
        fork_id: Option<U256>,
        transaction: B256,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let (prepared, _) = backend.backend().prepare_transact(
            fork_id,
            transaction,
            evm,
            0,
            cheats.isolation_env.as_ref(),
        )?;
        let executed = prepared
            .execute(evm, None, false)
            .wrap_err("backend: failed committing transaction")?;
        let result = backend.backend_mut().store_executed(evm, executed);
        result.logs.iter().for_each(|log| cheats.log_hook(log));
        Ok(result)
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
        let (result, persistent) = backend.backend_mut().transact_from_tx(tx, evm, env, None)?;
        cheats.record_persistent(usize::from(depth).saturating_sub(1), evm.state(), persistent);
        result.logs.iter().for_each(|log| cheats.log_hook(log));
        Ok(result)
    }

    fn console_log(&mut self, _msg: &str) {}
}

/// The [`Cheatcodes`] inspector together with the backend the execution runs over, as an evm2
/// inspector on its own.
///
/// Nested executions started by cheatcodes are only observed by the cheatcodes. Use the inspector
/// stack of `foundry-evm` to have them observed by tracing, coverage and other inspectors.
pub struct CheatcodesInspector<'a, FEN: FoundryEvmNetwork> {
    /// The cheatcode state.
    pub cheats: &'a mut Cheatcodes<FEN>,
    /// The backend the execution runs over.
    pub backend: &'a mut dyn CheatcodesBackend<FEN>,
}

impl<FEN: FoundryEvmNetwork> evm2::Inspector<TypesFor<FEN>> for CheatcodesInspector<'_, FEN> {
    fn initialize_interp(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.cheats.initialize_interp_hook(interp);
    }

    fn inspector_config(&self) -> InspectorConfig {
        InspectorConfig::new().with_steps(self.cheats.steps())
    }

    fn step(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.cheats.step_hook(interp, self.backend);
    }

    fn step_end(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.cheats.step_end_hook(interp, self.backend, &mut TransparentCheatcodesExecutor);
    }

    fn log(&mut self, log: &Log, _host: &mut FoundryEvmFor<'_, FEN>) {
        self.cheats.log_hook(log);
    }

    fn call(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        if self.cheats.is_storage_hook_callback(message) {
            return None;
        }
        self.cheats.call_with_executor(
            interp,
            message,
            self.backend,
            &mut TransparentCheatcodesExecutor,
            false,
        )
    }

    fn call_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        self.cheats.call_end_hook(interp, message, result, self.backend);
    }

    fn create(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &mut MessageFor<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        self.cheats.create_hook(interp, message, self.backend)
    }

    fn create_end(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        result: &mut MessageResultFor<FEN>,
    ) {
        self.cheats.create_end_hook(interp, message, result, self.backend);
    }
}

macro_rules! try_or_return {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return,
        }
    };
}

/// Returns the result of a message that did not execute, keeping its whole execution gas.
///
/// The result returns no state gas reservoir, which the caller adopts.
pub fn message_result<M, E: Default>(
    message: &MessageExt<M>,
    stop: InstrStop,
    output: Bytes,
) -> MessageResultExt<E> {
    MessageResultExt { stop, gas: GasTracker::new(message.gas_limit), output, ..Default::default() }
}

/// Stops the running frame with a revert carrying `output`, before its current instruction runs
/// or after it ran.
pub fn revert_frame<T: EvmTypesHost>(interp: &mut Interpreter<'_, '_, T>, output: &[u8]) {
    let offset = interp.memory().len();
    if interp.memory_mut().resize(offset, output.len()).is_err() {
        interp.set_stop(InstrStop::MemoryLimitOOG);
        return;
    }
    interp.memory_mut().set(offset, output);
    let (Ok(start), Ok(end)) = (u32::try_from(offset), u32::try_from(offset + output.len())) else {
        interp.set_stop(InstrStop::MemoryLimitOOG);
        return;
    };
    interp.set_output(start..end);
    interp.set_stop(InstrStop::Revert);
}

/// Returns the value a call message transfers, if it transfers one.
///
/// A static call transfers zero value, while a delegate call transfers none.
pub(crate) fn transfer_value<E>(message: &MessageExt<E>) -> Option<U256> {
    matches!(message.kind, MessageKind::Call | MessageKind::CallCode | MessageKind::StaticCall)
        .then_some(message.value)
}

/// Returns whether a message executes in a static context.
pub(crate) fn is_static<E>(message: &MessageExt<E>) -> bool {
    message.caller_is_static || message.kind == MessageKind::StaticCall
}

/// Helps collecting transactions from different forks.
#[derive(Clone, Debug)]
pub struct BroadcastableTransaction<N: Network = Ethereum> {
    /// The optional RPC URL.
    pub rpc: Option<String>,
    /// The transaction to broadcast.
    pub transaction: TransactionMaybeSigned<N>,
    /// The CREATE address captured from account state for a pre-signed transaction.
    pub contract_address: Option<Address>,
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

/// The storage-hook callback frame currently executing.
#[derive(Clone, Debug)]
struct ActiveStorageHook {
    depth: u16,
    callback_target: Address,
    callback_input: Bytes,
}

#[derive(Clone, Debug)]
struct StorageHookInspectorState {
    accesses: RecordAccess,
    recording_accesses: bool,
    mapping_slots: Option<AddressHashMap<MappingSlots>>,
    recorded_logs: Option<Vec<Vm::Log>>,
    recorded_logs_generation: u64,
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
    const fn take_isolated_snapshot_gas_used(
        &mut self,
        depth: usize,
        gas: &GasTracker,
    ) -> Option<u64> {
        let Some((gas_used, precharged_state)) = self.isolated_snapshot_gas_used.take() else {
            return None;
        };
        if self.recording {
            // The caller's sample already includes the precharged state gas.
            self.pending_isolated_region_gas =
                Some((depth, gas.spent(), gas_used.saturating_sub(precharged_state)));
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
    pub fn save(
        &mut self,
        state: &mut State<'_>,
        address: Address,
        slot: U256,
        data: U256,
    ) -> Result<(), evm2::DatabaseError> {
        self.values.get_mut(&address).expect("missing arbitrary address entry").insert(slot, data);
        state.storage_slot(&address, slot)?.set(data);
        Ok(())
    }

    /// Copies arbitrary storage value from source address to the given target address:
    /// - if a value is present in arbitrary values cache, then update target storage and return
    ///   existing value.
    /// - if no value was yet generated for given slot, then save new value in cache and update both
    ///   source and target storages.
    pub fn copy(
        &mut self,
        state: &mut State<'_>,
        target: Address,
        slot: U256,
        new_value: U256,
    ) -> Result<U256, evm2::DatabaseError> {
        let source = self.copies.get(&target).expect("missing arbitrary copy target entry");
        let storage_cache = self.values.get_mut(source).expect("missing arbitrary source storage");
        let value = match storage_cache.get(&slot) {
            Some(value) => *value,
            None => {
                storage_cache.insert(slot, new_value);
                // Update source storage with new value.
                state.storage_slot(source, slot)?.set(new_value);
                new_value
            }
        };
        // Update target storage with new value.
        state.storage_slot(&target, slot)?.set(value);
        Ok(value)
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
    /// Recorded log count when the frame started.
    recorded_logs: usize,
    /// Recorded log buffer generation when the frame started.
    recorded_logs_generation: u64,
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
    /// Engine-independent cheatcode state.
    pub core: CheatcodesCore,

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
    /// Executions run fee-less, so the `GASPRICE` opcode reads this value instead, unless
    /// `vm.txGasPrice` overrides it.
    pub gas_price: Option<u128>,

    /// The fee token of the executed transactions, recorded in broadcast requests.
    pub fee_token: Option<Address>,

    /// The `tx.origin` set by a prank or broadcast, which the `ORIGIN` opcode reads instead of
    /// the transaction's origin.
    ///
    /// evm2 borrows the transaction environment for the whole transaction, so origin changes are
    /// applied at the opcode level.
    pub origin_override: Option<Address>,

    /// Whether the executing `ORIGIN` opcode reads [`Self::origin_override`].
    pending_origin_override: bool,

    /// Whether the executing `GASPRICE` opcode reads [`Self::gas_price`].
    pending_gas_price: bool,

    /// An `expectEmit` mismatch observed by the log hook, which reverts the emitting frame once
    /// its `LOG` instruction finishes.
    pending_emit_failure: Option<&'static str>,

    /// Incremented whenever a cheatcode replaces the running state outside of a snapshot
    /// restoration, such as a fork switch, so nested executions know to adopt the replaced state.
    pub state_epoch: u64,

    /// Incremented whenever a cheatcode switches or rolls forks, bumped together with
    /// `state_epoch`.
    pub fork_epoch: u64,

    /// Running states replaced by snapshot restorations in the open frames of the running
    /// transaction, reinstated when a frame that was open at the restoration fails.
    pub snapshot_restores: SnapshotRestores,

    /// Target of the root call of the running transaction, if it is a call.
    ///
    /// While an isolated call or create runs as a nested transaction, this is the target of that
    /// transaction instead.
    pub root_call_target: Option<Address>,

    /// The EIP-3651 coinbase and access-list addresses the running transaction warmed when it
    /// started, which `vm.cool` keeps warm.
    ///
    /// While an isolated call or create runs as a nested transaction, these are that
    /// transaction's instead.
    pub tx_warm_addresses: Vec<Address>,

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

    /// Creation-list and recorded-log checkpoints for frames observed by this inspector.
    created_accounts_frames: Vec<CreatedAccountsFrame>,

    /// Creation lists captured by state snapshots.
    created_accounts_snapshots: HashMap<U256, CreatedAccountsSnapshot>,

    /// The information of the debug step recording.
    pub record_debug_steps_info: Option<RecordDebugStepInfo>,

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

    /// Additional addresses recognized as cheatcode contracts by this executor.
    pub extra_cheatcode_addresses: &'static [Address],

    /// Revert payloads minted by the `skip` cheatcode during the current test call.
    ///
    /// A top-level revert is only classified as a skip when its data byte-equals one of these
    /// payloads, so user-crafted `FOUNDRY::SKIP` revert data never skips a test on its own.
    pub skip_payloads: Vec<Bytes>,

    /// All recorded ETH `deal`s.
    pub eth_deals: Vec<DealRecord>,

    /// Writes that survive reverts of the enclosing calls, with the depth of the frame that made
    /// them and the journal length right after them, replayed when that frame or an enclosing one
    /// reverts and its rollback undid them.
    pub persistent_writes: Vec<(usize, usize, PersistentWrite)>,

    /// The depth of the frame that ended last and the journal length of its checkpoint, which a
    /// failure rolls the frame back to, recorded by the inspector stack.
    pub frame_rollback: Option<(usize, usize)>,

    /// The number of persistent writes recorded when each snapshot of the running transaction
    /// was taken.
    pub persistent_write_snapshots: HashMap<U256, usize>,

    /// Gas metering state.
    pub gas_metering: GasMetering,

    /// Contains gas snapshots made over the course of a test suite.
    // **Note**: both must a BTreeMap to ensure the order of the keys is deterministic.
    pub gas_snapshots: BTreeMap<String, BTreeMap<String, String>>,

    /// The current program counter.
    pub pc: usize,
    /// Breakpoints supplied by the `breakpoint` cheatcode.
    /// `char -> (address, pc)`
    pub breakpoints: Breakpoints,

    /// Ignored traces.
    pub ignored_traces: IgnoredTraces,

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
    /// Synthetic callback frame currently executing.
    active_storage_hook: Option<ActiveStorageHook>,

    /// Deprecated cheatcodes mapped to the reason. Used to report warnings on test results.
    pub deprecated: HashMap<&'static str, Option<&'static str>>,
    /// Main script contract, when script execution protection is enabled.
    pub script_address: Option<Address>,
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

    /// Whether we are currently executing inside an isolation context, i.e. the nested
    /// transaction an isolated call (`--isolate`, `--gas-report`) executes as.
    ///
    /// Cheatcodes that mutate the tx/block env consult this flag and route the change through
    /// `EnvOverrides` instead of the actual env when `true`, so they don't fight with the
    /// fee-accounting zeroing.
    pub in_isolation_context: bool,

    /// Environment of the transaction an isolated call runs in, while it runs.
    ///
    /// The isolated call's own nested transaction pays no fees; transactions that cheatcodes
    /// execute inside it start from this environment instead.
    pub isolation_env: Option<EvmEnvFor<FEN>>,

    /// Whether a replay is running whose fork-context changes cannot be merged into its parent.
    pub in_replay: bool,

    /// Depth of the in-flight `deployCode` call, whose create frame runs one level deeper.
    pub deploy_code_depth: Option<usize>,

    /// Account-creation state gas the in-flight `deployCode` call already charged its caller for
    /// its create frame.
    pub deploy_code_create_state_gas: u64,

    /// Output the in-flight `deployCode` call's constructor returned, which the EVM drops if
    /// depositing it fails, and whether the constructor ran in an isolated call.
    pub deploy_code_output: Option<(Bytes, bool)>,

    /// The `vm.accessList` the running transaction last applied to a call or create. As on
    /// master, it replaces the transaction's access list when computing its stipend.
    pub applied_access_list: Option<AccessList>,
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
            core: CheatcodesCore::new(config),
            extra_cheatcode_addresses: &[],
            block: Default::default(),
            fork_block_number_override: Default::default(),
            active_delegations: Default::default(),
            active_blob_sidecar: Default::default(),
            gas_price: Default::default(),
            fee_token: Default::default(),
            origin_override: Default::default(),
            pending_origin_override: Default::default(),
            pending_gas_price: Default::default(),
            pending_emit_failure: Default::default(),
            state_epoch: Default::default(),
            fork_epoch: Default::default(),
            snapshot_restores: Default::default(),
            root_call_target: Default::default(),
            tx_warm_addresses: Default::default(),
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
            record_debug_steps_info: Default::default(),
            expected_emits: Default::default(),
            expected_creates: Default::default(),
            allowed_mem_writes: Default::default(),
            broadcast: Default::default(),
            broadcastable_transactions: Default::default(),
            skip_payloads: Default::default(),
            eth_deals: Default::default(),
            persistent_writes: Default::default(),
            frame_rollback: Default::default(),
            persistent_write_snapshots: Default::default(),
            gas_metering: Default::default(),
            gas_snapshots: Default::default(),
            pc: Default::default(),
            breakpoints: Default::default(),
            ignored_traces: Default::default(),
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
            signatures_identifier: Default::default(),
            dynamic_gas_limit: Default::default(),
            execution_evm_version: None,
            env_overrides: Default::default(),
            fork_block_number_override_snapshots: Default::default(),
            in_isolation_context: false,
            isolation_env: None,
            in_replay: false,
            deploy_code_depth: None,
            deploy_code_create_state_gas: 0,
            deploy_code_output: None,
            applied_access_list: None,
        }
    }

    /// Sets additional addresses recognized as cheatcode contracts.
    #[inline]
    pub const fn set_extra_cheatcode_addresses(&mut self, addresses: &'static [Address]) {
        self.extra_cheatcode_addresses = addresses;
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
            recorded_logs: self.core.recorded_logs.as_ref().map_or(0, Vec::len),
            recorded_logs_generation: self.core.recorded_logs_generation,
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
            // Logs of a failed frame are reverted. If the buffer was replaced inside the frame,
            // every log it holds was recorded by the frame.
            if let Some(logs) = &mut self.core.recorded_logs {
                logs.truncate(
                    if frame.recorded_logs_generation == self.core.recorded_logs_generation {
                        frame.recorded_logs
                    } else {
                        0
                    },
                );
            }
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

    /// Adds a delegation to the active delegations list.
    pub fn add_delegation(&mut self, authorization: SignedAuthorization) {
        self.active_delegations.push(authorization);
    }

    /// Returns the signatures identifier.
    pub fn signatures_identifier(&self) -> Option<&SignaturesIdentifier> {
        self.signatures_identifier
            .get_or_init(|| {
                if let Some(artifacts) = &self.core.config.available_artifacts {
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
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        gas: &mut GasTracker,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        // decode the cheatcode call
        let decoded = Vm::VmCalls::abi_decode(&message.input).map_err(|e| {
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

        let caller = message.caller;

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        backend.backend().ensure_cheatcode_access_forking_mode(&caller)?;

        apply_dispatch(
            &decoded,
            &mut CheatsCtxt {
                state: self,
                evm: interp.host(),
                backend,
                gas,
                caller,
                is_static: is_static(message),
                depth: message.depth.into(),
            },
            executor,
        )
    }

    /// Decodes the input data and applies Monad-specific cheatcodes.
    #[cfg(feature = "monad")]
    fn apply_monad_cheatcode(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        message: &MessageFor<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        gas: &mut GasTracker,
    ) -> Result {
        let caller = message.caller;

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        backend.backend().ensure_cheatcode_access_forking_mode(&caller)?;

        crate::monad::apply_monad_cheatcode(
            &mut CheatsCtxt {
                state: self,
                evm: interp.host(),
                backend,
                gas,
                caller,
                is_static: is_static(message),
                depth: message.depth.into(),
            },
            &message.input,
        )
    }

    /// Grants cheat code access for new contracts if the caller also has
    /// cheatcode access or the new contract is created in top most call.
    ///
    /// There may be cheatcodes in the constructor of the new contract, in order to allow them
    /// automatically we need to determine the new address.
    fn allow_cheatcodes_on_create(
        backend: &mut dyn CheatcodesBackend<FEN>,
        depth: usize,
        caller: Address,
        created_address: Address,
    ) {
        let db = backend.backend();
        if (depth <= 1 || db.has_cheatcode_access(&caller))
            && !db.has_cheatcode_access(&created_address)
        {
            backend.backend_mut().allow_cheatcode_access(created_address);
        }
    }

    /// Maps the depth of the synthetic `deployCode` create frame, which runs in a nested EVM one
    /// level deeper, to the depth of the cheatcode call, where a native create would run.
    fn deploy_code_caller_depth(&self, depth: usize) -> usize {
        if self.deploy_code_depth.is_some_and(|d| d + 1 == depth) { depth - 1 } else { depth }
    }

    /// Sets the `tx.origin` the `ORIGIN` opcode reads, given the transaction's own origin.
    pub(crate) fn set_origin(&mut self, tx_origin: Address, origin: Address) {
        self.origin_override = (origin != tx_origin).then_some(origin);
    }

    /// Applies the configured `vm.accessList` to the running transaction, as master did on every
    /// call and create. Access lists do not exist before Berlin, and nested transactions, such as
    /// isolated calls, keep their own.
    fn apply_access_list(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        if let Some(access_list) = &self.core.access_list
            && !self.in_isolation_context
            && !self.in_replay
            && interp.host().spec_id() >= evm2::SpecId::BERLIN
        {
            self.applied_access_list = Some(access_list.clone());
        }
    }

    /// Handles a call, accounting for whether the executor will isolate it as a transaction.
    ///
    /// If `isolate_call` is true, the executor owns the transaction nonce increment when the call
    /// proceeds to execution.
    pub fn call_with_executor(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        call: &mut MessageFor<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
        isolate_call: bool,
    ) -> Option<MessageResultFor<FEN>> {
        let tx_origin = interp.host().tx_env().origin;
        let curr_depth = usize::from(call.depth);
        self.start_created_accounts_frame(
            curr_depth == 0,
            CreatedAccountsFrameKind::Call,
            curr_depth,
        );
        if curr_depth == 0 {
            self.root_call_target = Some(call.destination);
            self.applied_access_list = None;
        }

        // At the root call to test function or script `run()`/`setUp()` functions, we are
        // decreasing sender nonce to ensure that it matches on-chain nonce once we start
        // broadcasting.
        if curr_depth == 0 {
            let result = interp.host().state_mut().account(&tx_origin).map(|mut account| {
                let prev = account.nonce();
                account.set_nonce(prev.saturating_sub(1));
                (prev, account.nonce())
            });
            match result {
                Ok((prev, nonce)) => {
                    trace!(target: "cheatcodes", sender=%tx_origin, nonce, prev, "corrected nonce");
                }
                Err(err) => {
                    return Some(message_result(
                        call,
                        InstrStop::Revert,
                        Error::from(err).abi_encode().into(),
                    ));
                }
            }
        }

        let target = call.destination;
        // Cheatcodes return the gas they did not spend, as `message_result` does.
        let mut gas = GasTracker::new(call.gas_limit);
        if target == CHEATCODE_ADDRESS {
            // `vm.breakpoint` records the calling instruction, which frames running without
            // inspector steps do not track.
            self.pc = interp.pc();
            let (stop, output) =
                match self.apply_cheatcode(interp, call, backend, &mut gas, executor) {
                    Ok(retdata) => (InstrStop::Return, retdata.into()),
                    Err(err) => (InstrStop::Revert, err.abi_encode().into()),
                };
            return Some(MessageResultExt { stop, gas, output, ..Default::default() });
        }

        #[cfg(feature = "monad")]
        if crate::monad::is_monad_cheatcode_call(self.extra_cheatcode_addresses, target) {
            let checkpoint = interp.host().state().checkpoint();
            let (stop, output) = match self.apply_monad_cheatcode(interp, call, backend, &mut gas) {
                Ok(retdata) => (InstrStop::Return, retdata.into()),
                Err(err) => {
                    let evm = interp.host();
                    let features = evm.version().features;
                    evm.state_mut().rollback(checkpoint, features);
                    (InstrStop::Revert, err.abi_encode().into())
                }
            };
            return Some(MessageResultExt { stop, gas, output, ..Default::default() });
        }

        if target == HARDHAT_CONSOLE_ADDRESS {
            return None;
        }

        // `expectRevert`: track max call depth. This is also done in `initialize_interp`, but
        // precompile calls don't create an interpreter frame so we must also track it here.
        // The callee executes at `curr_depth + 1`.
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = max(curr_depth + 1, expected.max_depth);
        }

        // Handle expected calls
        if let Some(expected) = self.core.expected_calls.get_mut(&call.call_target) {
            expect::observe_call(
                expected,
                &call.input,
                transfer_value(call),
                call.gas_limit,
                call.kind.into(),
            );
        }

        // Apply our prank
        if let Some(prank) = self.get_prank(curr_depth).copied() {
            // Apply delegate call, `call.caller`` will not equal `prank.prank_caller`
            if prank.delegate_call
                && curr_depth == prank.depth
                && call.kind == MessageKind::DelegateCall
            {
                call.destination = prank.new_caller;
                call.caller = prank.new_caller;
                if let Some(new_origin) = prank.new_origin {
                    self.set_origin(tx_origin, new_origin);
                }
                if let Some(used) = prank.first_time_applied() {
                    self.pranks.insert(curr_depth, used);
                }
            }

            if let Some(changes) = prank.changes_for(curr_depth, call.caller) {
                if let Some(new_caller) = changes.caller {
                    // Ensure new caller is loaded, warm and touched
                    if let Ok(mut account) = interp.host().state_mut().account(&new_caller) {
                        account.touch();
                        account.warm();
                    }
                    call.caller = new_caller;
                }
                if let Some(new_origin) = changes.origin {
                    self.set_origin(tx_origin, new_origin);
                }
                if let Some(used) = changes.used {
                    self.pranks.insert(curr_depth, used);
                }
            }
        }

        // Handle mocked calls
        if let Some(mocks) = self.core.mocked_calls.get_mut(&call.call_target)
            && let Some(return_data_queue) =
                mock::find_mock_returns(mocks, &call.input, transfer_value(call))
            && let Some(return_data) = return_data_queue.front().map(|x| x.to_owned())
        {
            if let Some(value) = transfer_value(call)
                && !return_data.reverts
            {
                let state = interp.host().state_mut();
                match state.transfer(&call.caller, &call.destination, &value) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Some(message_result(call, InstrStop::OutOfFunds, Bytes::new()));
                    }
                    Err(err) => {
                        return Some(message_result(
                            call,
                            InstrStop::Revert,
                            Error::from(err).abi_encode().into(),
                        ));
                    }
                }
            }

            mock::advance_mock_returns(return_data_queue);

            let stop = if return_data.reverts { InstrStop::Revert } else { InstrStop::Return };
            return Some(message_result(call, stop, return_data.data));
        }

        self.apply_access_list(interp);

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
                && call.caller == broadcast.original_caller
            {
                // Delegatecalls and callcodes run in the caller's context and cannot be
                // represented as a transaction from the broadcaster.
                if matches!(call.kind, MessageKind::DelegateCall | MessageKind::CallCode) {
                    let msg = "`delegatecall`s and `callcode`s cannot be broadcast; use a regular `call` instead";
                    return Some(message_result(call, InstrStop::Revert, Error::encode(msg)));
                }

                // Reset deploy from code flag for upcoming calls.
                broadcast.deploy_from_code = false;

                // At the target depth we set `msg.sender` & tx.origin.
                // We are simulating the caller as being an EOA, so *both* must be set to the
                // broadcast.origin.
                let new_origin = broadcast.new_origin;
                let single_call = broadcast.single_call;
                self.set_origin(tx_origin, new_origin);
                call.caller = new_origin;

                // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                // because we only need the from, to, value, and data. We can later change this
                // into 1559, in the cli package, relatively easily once we
                // know the target chain supports EIP-1559.
                if !is_static(call) {
                    let evm = interp.host();
                    // Reading the nonce warms the broadcaster.
                    let nonce = match evm.state_mut().account(&new_origin) {
                        Ok(mut account) => {
                            account.warm();
                            account.nonce()
                        }
                        Err(err) => {
                            return Some(message_result(
                                call,
                                InstrStop::Revert,
                                Error::encode(err),
                            ));
                        }
                    };
                    let chain_id = self
                        .env_overrides
                        .get(backend.backend().active_fork_id())
                        .and_then(|overrides| overrides.chain_id)
                        .unwrap_or(evm.version().chain_id);
                    let rpc = backend.backend().active_fork_url();

                    let mut tx_req = TransactionRequestFor::<FEN>::default()
                        .with_from(new_origin)
                        .with_to(target)
                        .with_value(transfer_value(call).unwrap_or_default())
                        .with_input(call.input.clone())
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
                            return Some(message_result(
                                call,
                                InstrStop::Revert,
                                Error::encode(msg),
                            ));
                        }
                        tx_req.set_blob_sidecar(blob_sidecar);
                    }

                    // Apply active EIP-7702 delegations, if any.
                    if !active_delegations.is_empty() {
                        match apply_authorization_nonces::<FEN>(
                            evm,
                            &active_delegations,
                            new_origin,
                            chain_id,
                        ) {
                            Ok(writes) => self.record_persistent(
                                curr_depth.saturating_sub(1),
                                evm.state(),
                                writes,
                            ),
                            Err(err) => {
                                return Some(message_result(
                                    call,
                                    InstrStop::Revert,
                                    err.abi_encode().into(),
                                ));
                            }
                        }
                        tx_req.set_authorization_list(active_delegations);
                    }
                    if let Some(fee_token) = self.fee_token {
                        tx_req.set_fee_token(fee_token);
                    }
                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc,
                        transaction: TransactionMaybeSigned::new(tx_req),
                        contract_address: None,
                    });
                    debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable call");

                    // Isolated transactions increment the nonce during execution. Nested
                    // broadcasts do not start a separate transaction and need this increment.
                    // The increment survives reverts of the enclosing calls. evm2 runs this hook
                    // before the call's checkpoint, so the increment belongs to the caller's frame.
                    let bump = if !isolate_call
                        && let Ok(mut account) = evm.state_mut().account(&new_origin)
                    {
                        let prev = account.nonce();
                        account.set_nonce(prev + 1);
                        debug!(target: "cheatcodes", address=%new_origin, nonce=prev+1, prev, "incremented nonce");
                        Some(PersistentWrite::nonce(&account, prev))
                    } else {
                        None
                    };
                    if let Some(write) = bump {
                        self.record_persistent(curr_depth.saturating_sub(1), evm.state(), [write]);
                    }
                } else if single_call {
                    let msg = "`staticcall`s are not allowed after `broadcast`; use `startBroadcast` instead";
                    return Some(message_result(call, InstrStop::Revert, Error::encode(msg)));
                }
            }
        }

        // Record called accounts if `startStateDiffRecording` has been called
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // Determine if account is "initialized," ie, it has a non-zero balance, a non-zero
            // nonce, a non-zero KECCAK_EMPTY codehash, or non-empty code
            let evm = interp.host();
            let (initialized, old_balance, old_nonce) = match evm.state_mut().account(&target) {
                Ok(mut account) => {
                    account.warm();
                    (
                        account.get().is_some_and(|info| !info.is_empty()),
                        account.balance(),
                        account.nonce(),
                    )
                }
                Err(_) => (false, U256::ZERO, 0),
            };
            // Fork switches and `vm.chainId` take effect before the execution version changes.
            let chain_id = self
                .env_overrides
                .get(backend.backend().active_fork_id())
                .and_then(|overrides| overrides.chain_id)
                .unwrap_or(evm.version().chain_id);

            let kind = match call.kind {
                MessageKind::CallCode => crate::Vm::AccountAccessKind::CallCode,
                MessageKind::DelegateCall => crate::Vm::AccountAccessKind::DelegateCall,
                MessageKind::StaticCall => crate::Vm::AccountAccessKind::StaticCall,
                _ => crate::Vm::AccountAccessKind::Call,
            };

            // Record this call by pushing it to a new pending vector; all subsequent calls at
            // that depth will be pushed to the same vector. When the call ends, the
            // RecordedAccountAccess (and all subsequent RecordedAccountAccesses) will be
            // updated with the revert status of this call, since the EVM does not mark accounts
            // as "warm" if the call from which they were accessed is reverted
            let access = AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: backend.backend().active_fork_id().unwrap_or_default(),
                    chainId: U256::from(chain_id),
                },
                accessor: call.caller,
                account: call.call_target,
                kind,
                initialized,
                oldBalance: old_balance,
                newBalance: U256::ZERO, // updated on call_end
                oldNonce: old_nonce,
                newNonce: 0, // updated on call_end
                value: call.value,
                data: call.input.clone(),
                reverted: false,
                deployedCode: Bytes::new(),
                storageAccesses: vec![], // updated on step
                depth: curr_depth as u64,
            };
            recorded_account_diffs_stack.push(vec![access]);
        }

        None
    }

    /// Returns addresses explicitly marked with arbitrary storage.
    pub fn arbitrary_storage_targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.core.arbitrary_storage.as_ref().into_iter().flat_map(ArbitraryStorage::targets)
    }

    /// Returns addresses explicitly marked with arbitrary storage and whether nonzero slots are
    /// overwritten.
    pub fn arbitrary_storage_target_overwrite_modes(
        &self,
    ) -> impl Iterator<Item = (Address, bool)> + '_ {
        self.core
            .arbitrary_storage
            .as_ref()
            .into_iter()
            .flat_map(ArbitraryStorage::target_overwrite_modes)
    }

    /// Returns addresses that copy storage from arbitrary-storage targets.
    pub fn arbitrary_storage_copied_targets(&self) -> impl Iterator<Item = Address> + '_ {
        self.core.arbitrary_storage.as_ref().into_iter().flat_map(ArbitraryStorage::copied_targets)
    }

    /// Returns copied arbitrary-storage targets and their source address.
    pub fn arbitrary_storage_copied_target_sources(
        &self,
    ) -> impl Iterator<Item = (Address, Address)> + '_ {
        self.core
            .arbitrary_storage
            .as_ref()
            .into_iter()
            .flat_map(ArbitraryStorage::copied_target_sources)
    }

    /// Caches a concrete replay value for a slot on an arbitrary-storage address or copied target.
    pub fn cache_arbitrary_storage_value(&mut self, address: Address, slot: U256, value: U256) {
        if let Some(storage) = &mut self.core.arbitrary_storage {
            storage.cache_value(address, slot, value);
        }
    }

    /// Marks a slot as explicitly written with `vm.store`.
    pub fn mark_arbitrary_storage_slot_explicit(&mut self, address: Address, slot: U256) {
        if let Some(storage) = &mut self.core.arbitrary_storage {
            storage.mark_explicit(address, slot);
        }
    }

    /// Returns whether a slot was explicitly written with `vm.store`.
    pub fn is_arbitrary_storage_slot_explicit(&self, address: Address, slot: U256) -> bool {
        self.core
            .arbitrary_storage
            .as_ref()
            .is_some_and(|storage| storage.is_explicit(address, slot))
    }

    /// Returns a cached arbitrary-storage replay value for a slot.
    pub fn cached_arbitrary_storage_value(&self, address: Address, slot: U256) -> Option<U256> {
        self.core.arbitrary_storage.as_ref().and_then(|storage| storage.cached_value(address, slot))
    }

    /// Whether the given address has arbitrary storage.
    pub fn has_arbitrary_storage(&self, address: &Address) -> bool {
        match &self.core.arbitrary_storage {
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
        match &self.core.arbitrary_storage {
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
        match &self.core.arbitrary_storage {
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
    pub fn is_storage_hook_callback(&self, call: &MessageFor<FEN>) -> bool {
        self.active_storage_hook.as_ref().is_some_and(|active| {
            call.depth == active.depth
                && call.caller == CHEATCODE_ADDRESS
                && call.destination == active.callback_target
                && call.input == active.callback_input
        })
    }

    /// Returns the opcodes for which [`Self::step_hook`] and [`Self::step_end_hook`] must run.
    ///
    /// This is the only place that decides which instructions the cheatcodes step: the inspector
    /// asks the EVM for these opcodes, and skips the hooks for every other instruction.
    pub fn steps(&self) -> OpcodeSet {
        if self.is_storage_hook_active()
            || self.has_storage_hooks()
            || self.broadcast.is_some()
            || self.gas_metering.paused
            || self.gas_metering.reset
            || self.gas_metering.touched
            || self.gas_metering.recording
            || self.recorded_account_diffs_stack.is_some()
            || !self.allowed_mem_writes.is_empty()
            || self.core.mapping_slots.is_some()
            || self.core.arbitrary_storage.is_some()
            || self.deploy_code_depth.is_some()
        {
            return OpcodeSet::ALL;
        }
        let mut steps = OpcodeSet::EMPTY;
        // A log that fails `expectEmit` reverts its frame in the `step_end` of the instruction
        // that emitted it.
        if !self.expected_emits.is_empty() || self.pending_emit_failure.is_some() {
            steps = steps.union(LOG_EMITTING_OPCODES);
        }
        if self.recording_accesses {
            steps = steps.union(opcodes([op::SLOAD, op::SSTORE]));
        }
        // Origin and gas price overrides rewrite what these instructions push.
        if self.origin_override.is_some() || self.gas_price.is_some() {
            steps = steps.union(opcodes([op::ORIGIN, op::GASPRICE]));
        }
        if self.has_active_env_overrides() {
            steps = steps.union(opcodes([op::BASEFEE, op::GASPRICE, op::CHAINID, op::BLOBHASH]));
        }
        steps
    }

    #[inline(always)]
    pub fn has_log_hooks(&self) -> bool {
        !self.expected_emits.is_empty() || self.core.recorded_logs.is_some()
    }

    #[inline(always)]
    const fn has_active_env_overrides(&self) -> bool {
        self.env_overrides.is_any_set()
    }

    /// Called after a frame interpreter has been initialized.
    pub fn initialize_interp_hook(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            interp.host().set_block(block);
        }

        // Record gas for current frame.
        if self.gas_metering.paused {
            self.gas_metering.paused_frames.push(interp.gas());
        }

        // `expectRevert`: track the max call depth during `expectRevert`
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = max(usize::from(interp.message().depth) + 1, expected.max_depth);
        }
    }

    /// Called before each instruction in [`Self::steps`] executes.
    ///
    /// Kept out of line: it runs only while a cheatcode needs per-instruction work, and inlining
    /// it into the inspector's per-instruction hook enlarges that hook's frame for every opcode.
    #[inline(never)]
    pub fn step_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
    ) {
        self.pc = interp.pc();

        let depth = usize::from(interp.message().depth) + 1;
        let opcode = interp.opcode();

        // Marks an `ORIGIN` or `GASPRICE` instruction whose pushed value `step_end` overrides.
        self.pending_origin_override = opcode == op::ORIGIN && self.origin_override.is_some();
        self.pending_gas_price = opcode == op::GASPRICE && self.gas_price.is_some();

        if self.broadcast.is_some() {
            self.set_gas_limit_type(interp);
        }

        // Broadcasting changes outgoing calls, not the caller of the script's current frame.
        // Only protect the broadcasting frame; callbacks into the script have their own caller.
        if opcode == op::CALLER
            && let Some(broadcast) = &self.broadcast
            && let Some(script_address) = self.script_address
            && depth == broadcast.depth
            && interp.message().destination == script_address
            && interp.message().code_address == script_address
            && interp.message().caller != broadcast.new_origin
        {
            let message = format!(
                "Usage of `msg.sender` inside a `broadcast` in script contract detected. \
                 `msg.sender` is `{:#x}`, not the broadcast sender `{:#x}`. \
                 Use the `--sender` flag or pass the deployer address directly instead.",
                interp.message().caller,
                broadcast.new_origin,
            );
            revert_frame(interp, message.as_bytes());
            return;
        }

        // `pauseGasMetering`: pause / resume interpreter gas.
        if self.gas_metering.paused {
            self.meter_gas(interp);
        }

        // `resetGasMetering`: reset interpreter gas.
        if self.gas_metering.reset {
            self.meter_gas_reset(interp);
        }

        // `record`: record storage reads and writes.
        if self.recording_accesses {
            self.record_accesses(interp);
        }

        // `startStateDiffRecording`: record granular ordered storage accesses.
        if self.recorded_account_diffs_stack.is_some() {
            self.record_state_diffs(interp, backend, depth);
        }

        // `expectSafeMemory`: check if the current opcode is allowed to interact with memory.
        if !self.allowed_mem_writes.is_empty() {
            self.check_mem_opcodes(interp, depth as u64);
        }

        if self.core.mapping_slots.is_some() || !self.mapping_storage_store_hooks.is_empty() {
            // `startMappingRecording`: record SSTORE.
            if let Some(mapping_slots) = &mut self.core.mapping_slots {
                mapping_step(mapping_slots, interp);
            }

            let account = interp.message().destination;
            let mapping_hook_active = self.active_storage_hook.is_none()
                && self
                    .mapping_storage_store_hooks
                    .get(&account)
                    .is_some_and(|hooks| !hooks.is_empty());
            if mapping_hook_active {
                mapping_step(&mut self.storage_hook_mapping_slots, interp);
            }
            self.pending_mapping_hash = if self.core.mapping_slots.is_some() || mapping_hook_active
            {
                capture_mapping_hash(interp)
            } else {
                None
            };
        }

        // `snapshotGas*`: take a snapshot of the current gas.
        if self.gas_metering.recording {
            self.meter_gas_record(interp, depth);
        }

        // Capture the opcode for `step_end` to use, since by the time
        // `step_end` runs the PC has already advanced past it. Also peek the
        // BLOBHASH index now (still on top of stack before execution) so we
        // can look up the override later.
        if !self.env_overrides.is_empty() {
            let fork_id = backend.backend().active_fork_id();
            if let Some(env_overrides) =
                self.env_overrides.get_mut(fork_id).filter(|o| o.is_any_set())
            {
                // Always clear stale pending state first so a leftover value from
                // a prior step (e.g. when `peek` failed, or when an override
                // wasn't actually used) cannot leak into the next opcode.
                env_overrides.pending_opcode = None;
                env_overrides.pending_blobhash_index = None;

                match opcode {
                    op::BASEFEE | op::GASPRICE | op::CHAINID => {
                        env_overrides.pending_opcode = Some(opcode);
                    }
                    op::BLOBHASH => {
                        env_overrides.pending_opcode = Some(opcode);
                        env_overrides.pending_blobhash_index =
                            interp.stack().peek(0).and_then(|index| index.try_into().ok());
                    }
                    _ => {}
                }
            }
        }

        if self.active_storage_hook.is_none() {
            self.capture_storage_hook(interp);
        }
    }

    /// Called after each instruction in [`Self::steps`] executes.
    ///
    /// Kept out of line for the same reason as [`Self::step_hook`].
    #[inline(never)]
    pub fn step_end_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) {
        // `deployCode`: keep the constructor's output, which it reports if depositing it fails.
        if let Some(depth) = self.deploy_code_depth
            && usize::from(interp.message().depth) == depth + 1
            && interp.message().kind.is_create()
            && interp.result() == Err(InstrStop::Return)
        {
            self.deploy_code_output =
                Some((Bytes::copy_from_slice(interp.output()), self.in_isolation_context));
        }

        // `expectEmit`: a mismatched log reverts the frame that emitted it.
        if let Some(reason) = self.pending_emit_failure.take()
            && interp.result().is_ok()
        {
            revert_frame(interp, &Error::encode(reason));
            return;
        }

        if self.gas_metering.paused {
            self.meter_gas_end(interp);
        }

        if self.gas_metering.touched {
            self.meter_gas_check(interp);
        }

        // `setArbitraryStorage` and `copyStorage`: add arbitrary values to storage.
        if self.core.arbitrary_storage.is_some() {
            self.arbitrary_storage_end(interp);
        }

        if let Some(pending) = self.pending_mapping_hash.take()
            && interp.result().is_ok()
        {
            if let Some(mapping_slots) = &mut self.core.mapping_slots {
                record_mapping_hash(mapping_slots, interp, pending);
            }
            if self
                .mapping_storage_store_hooks
                .get(&pending.address)
                .is_some_and(|hooks| !hooks.is_empty())
                && self.active_storage_hook.is_none()
            {
                record_mapping_hash(&mut self.storage_hook_mapping_slots, interp, pending);
            }
        }

        if self.active_storage_hook.is_none() {
            self.invoke_pending_storage_hook(interp, backend, executor);
        }

        // Apply opcode-level overrides. We must only rewrite the stack if the opcode actually
        // completed successfully and pushed its result; otherwise (stack underflow on BLOBHASH,
        // OOG before push, etc.) the stack is in an error state and a blind `pop()+push()` would
        // corrupt the failing frame.
        let opcode_failed = interp.result().is_err();
        if std::mem::take(&mut self.pending_origin_override)
            && !opcode_failed
            && let Some(origin) = self.origin_override
        {
            Self::replace_top_of_stack(interp, origin.into_word().into());
        }
        // `vm.txGasPrice` overrides below take precedence.
        if std::mem::take(&mut self.pending_gas_price)
            && !opcode_failed
            && let Some(gas_price) = self.gas_price
        {
            Self::replace_top_of_stack(interp, U256::from(gas_price));
        }
        if !self.env_overrides.is_empty() {
            let fork_id = backend.backend().active_fork_id();
            if self.env_overrides.get(fork_id).is_some_and(|o| o.is_any_set()) {
                if opcode_failed {
                    if let Some(env_overrides) = self.env_overrides.get_mut(fork_id) {
                        env_overrides.pending_opcode = None;
                        env_overrides.pending_blobhash_index = None;
                    }
                } else {
                    self.apply_env_overrides(interp, fork_id);
                }
            }
        }
    }

    /// Called when a log is emitted.
    pub fn log_hook(&mut self, log: &Log) {
        if !self.expected_emits.is_empty()
            && let Some(reason) = expect::handle_expect_emit(self, log)
        {
            // The emitting frame reverts once its `LOG` instruction finishes.
            self.pending_emit_failure = Some(reason);
        }

        // `recordLogs`
        record_logs(&mut self.core.recorded_logs, log);
    }

    /// Records `writes` the frame at `depth` just made to `state`, see
    /// [`Self::persistent_writes`].
    pub fn record_persistent(
        &mut self,
        depth: usize,
        state: &State<'_>,
        writes: impl IntoIterator<Item = PersistentWrite>,
    ) {
        let len = state.checkpoint().journal_len();
        self.persistent_writes.extend(writes.into_iter().map(|write| (depth, len, write)));
    }

    /// Retains `replaced`, the running state a revert to `snapshot_id` replaced, and drops the
    /// persistent writes recorded since the snapshot, which the restored state does not hold.
    pub(crate) fn restore_persistent_writes(
        &mut self,
        snapshot_id: U256,
        replaced: StateSnapshot,
        remove: bool,
    ) {
        self.snapshot_restores.capture(replaced, &self.persistent_writes);
        let len = if remove {
            self.persistent_write_snapshots.remove(&snapshot_id)
        } else {
            self.persistent_write_snapshots.get(&snapshot_id).copied()
        };
        // A snapshot of an earlier transaction holds none of this transaction's writes. Under the
        // inspector stack a failing frame reinstates the captured ledger instead; this covers the
        // stackless executor.
        self.persistent_writes.truncate(len.unwrap_or_default());
    }

    /// Replays the persistent writes made in the frame at `depth` or deeper after the frame
    /// failed and the writes were rolled back, as writes of the enclosing frame. A transaction's
    /// writes are dropped when its root frame ends; a nested one merges its changes itself.
    fn replay_persistent_writes(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        depth: usize,
        success: bool,
    ) {
        if self.persistent_writes.is_empty() {
            return;
        }
        let root = usize::from(interp.version().initial_call_depth);
        let state = interp.host().state_mut();
        let mut failed = None;
        if !success {
            // Without the frame's rollback, such as without the inspector stack, every write of
            // the frame was undone. A context switch in the frame can leave later writes below
            // its checkpoint, which the rollback keeps.
            let rollback =
                self.frame_rollback.take().filter(|(end, _)| *end == depth).map(|(_, len)| len);
            let undone = |&(write_depth, len, _): &(usize, usize, PersistentWrite)| {
                write_depth >= depth && rollback.is_none_or(|rollback| len > rollback)
            };
            // The nonces after the rollback, to collapse each account's replayed nonce writes into
            // the one change the replay made, so an enclosing replay does not undo the saturation.
            let mut nonces = AddressMap::<u64>::default();
            for (.., write) in self.persistent_writes.iter().filter(|entry| undone(entry)) {
                if let PersistentWrite::Nonce { address, .. } = write
                    && let Ok(account) = state.account(address)
                {
                    nonces.entry(*address).or_insert(account.nonce());
                }
            }
            let writes = self.persistent_writes.iter().filter(|entry| undone(entry));
            failed = PersistentWrite::apply_all(writes.map(|(_, _, write)| write), state).err();
            let len = state.checkpoint().journal_len();
            self.persistent_writes.retain_mut(|entry| {
                if !undone(entry) {
                    return true;
                }
                entry.1 = len;
                !matches!(entry.2, PersistentWrite::Nonce { .. })
            });
            for (address, previous) in nonces {
                if let Ok(account) = state.account(&address) {
                    let write = PersistentWrite::nonce(&account, previous);
                    self.persistent_writes.push((depth, len, write));
                }
            }
        }
        for (write_depth, ..) in &mut self.persistent_writes {
            // The parent owns the writes once the frame ends, so a failing sibling at the same
            // depth does not replay them again.
            if *write_depth >= depth && depth > root {
                *write_depth = depth - 1;
            }
        }
        if depth == root {
            self.persistent_writes.retain(|(write_depth, ..)| *write_depth < root);
        }
        if let Some(err) = failed {
            let _ = interp.fail(err);
        }
    }

    /// Called after a call message executes.
    pub fn call_end_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        call: &MessageFor<FEN>,
        outcome: &mut MessageResultFor<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
    ) {
        let curr_depth = usize::from(call.depth);
        self.replay_persistent_writes(interp, curr_depth, outcome.is_success());
        // Depth at which the caller resumes and applies the region snapshot correction.
        let isolated_snapshot_gas_used = self.gas_metering.take_isolated_snapshot_gas_used(
            self.deploy_code_caller_depth(curr_depth),
            &outcome.gas,
        );
        if self.is_storage_hook_callback(call) {
            return;
        }

        let target = call.destination;
        let cheatcode_call = target == CHEATCODE_ADDRESS || target == HARDHAT_CONSOLE_ADDRESS;
        #[cfg(feature = "monad")]
        let cheatcode_call = cheatcode_call
            || crate::monad::is_monad_cheatcode_call(self.extra_cheatcode_addresses, target);
        let tx_origin = interp.host().tx_env().origin;

        self.finish_created_accounts_frame(
            outcome.is_success(),
            CreatedAccountsFrameKind::Call,
            curr_depth,
        );

        // Clean up pranks/broadcasts if it's not a cheatcode call end. We shouldn't do
        // it for cheatcode calls because they are not applied for cheatcodes in the `call` hook.
        // This should be placed before the revert handling, because we might exit early there
        if !cheatcode_call {
            // Clean up pranks
            if let Some(prank) = self.get_prank(curr_depth).copied()
                && curr_depth == prank.depth
            {
                self.set_origin(tx_origin, prank.prank_origin);

                // Clean single-call prank once we have returned to the original depth
                if prank.single_call {
                    self.pranks.remove(&curr_depth);
                }
            }

            // Clean up broadcast
            if let Some(broadcast) = self.broadcast.clone()
                && curr_depth == broadcast.depth
            {
                self.set_origin(tx_origin, broadcast.original_origin);

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
            if outcome.stop.is_revert() && assume_no_revert.reverted_by.is_none() {
                assume_no_revert.reverted_by = Some(target);
            }

            // allow multiple cheatcode calls at the same depth
            if curr_depth <= assume_no_revert.depth && !cheatcode_call {
                // Discard run if we're at the same depth as cheatcode, call reverted, and no
                // specific reason was supplied
                if outcome.stop.is_revert() {
                    let assume_no_revert = std::mem::take(&mut self.assume_no_revert).unwrap();
                    return match revert_handlers::handle_assume_no_revert(
                        &assume_no_revert,
                        outcome.stop,
                        &outcome.output,
                        &self.core.config.available_artifacts,
                    ) {
                        // if result is Ok, it was an anticipated revert; return an "assume" error
                        // to reject this run
                        Ok(_) => {
                            outcome.output = Error::from(MAGIC_ASSUME).abi_encode().into();
                        }
                        // if result is Error, it was an unanticipated revert; should revert
                        // normally
                        Err(error) => {
                            trace!(expected=?assume_no_revert, ?error, status=?outcome.stop, "Expected revert mismatch");
                            set_revert(outcome, error.abi_encode().into());
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
            let call_failed = !outcome.is_success();
            if call_failed {
                // Record current reverter address if expect revert is set with expected reverter
                // address and no actual reverter was set yet or if we're expecting more than one
                // revert.
                if expected_revert.reverter.is_some()
                    && (expected_revert.reverted_by.is_none() || expected_revert.count > 1)
                {
                    expected_revert.reverted_by = Some(target);
                }
            }

            if curr_depth <= expected_revert.depth {
                let needs_processing = expected_revert.needs_processing(
                    cheatcode_call,
                    call_failed,
                    curr_depth,
                    self.core.config.internal_expect_revert,
                );

                if needs_processing {
                    let mut expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                    let clear_last_frame_gas =
                        matches!(expected_revert.kind, ExpectedRevertKind::Default);
                    return match revert_handlers::handle_expect_revert(
                        cheatcode_call,
                        false,
                        self.core.config.internal_expect_revert,
                        &expected_revert,
                        outcome.stop,
                        outcome.output.clone(),
                        &self.core.config.available_artifacts,
                    ) {
                        Err(error) => {
                            trace!(expected=?expected_revert, ?error, status=?outcome.stop, "Expected revert mismatch");
                            set_revert(outcome, error.abi_encode().into());
                        }
                        Ok((_, retdata)) => {
                            expected_revert.actual_count += 1;
                            if expected_revert.actual_count < expected_revert.count {
                                self.expected_revert = Some(expected_revert);
                            }
                            if clear_last_frame_gas {
                                self.gas_metering.last_frame_gas = None;
                            }
                            set_return(outcome, retdata);
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
        let frame_gas = frame_gas(outcome.stop, &outcome.gas);
        let snapshot_gas_used = isolated_snapshot_gas_used.unwrap_or_else(|| outcome.gas.spent());
        self.gas_metering.last_call_gas = Some(frame_gas.clone());
        self.gas_metering.last_frame_gas = Some(frame_gas);
        self.gas_metering.last_call_snapshot_gas_used = snapshot_gas_used;
        self.gas_metering.last_frame_snapshot_gas_used = snapshot_gas_used;

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            // The root call cannot be recorded.
            if curr_depth > 0
                && let Some(mut last_recorded_depth) = recorded_account_diffs_stack.pop()
            {
                // Update the reverted status of all deeper calls if this call reverted, in
                // accordance with EVM behavior
                if outcome.stop.is_revert() {
                    mark_account_accesses_reverted(&mut last_recorded_depth);
                }

                if let Some(call_access) = last_recorded_depth.first_mut() {
                    // Assert that we're at the correct depth before recording post-call state
                    // changes. Depending on the depth the cheat was
                    // called at, there may not be any pending
                    // calls to update if execution has percolated up to a higher depth.
                    if call_access.depth == curr_depth as u64
                        && let Ok(mut account) = interp.host().state_mut().account(&target)
                    {
                        debug_assert!(access_is_call(call_access.kind));
                        account.warm();
                        call_access.newBalance = account.balance();
                        call_access.newNonce = account.nonce();
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
        if outcome.stop.is_revert() {
            // if there's a revert and a previous call was diagnosed as fork related revert then we
            // can return a better error here
            if let Some(err) = diag {
                outcome.output = Error::encode(err.to_error_msg(&self.core.labels));
            }
            return;
        }

        if let Some(unmet) = expected_emit::check_call_emits(
            &mut self.expected_emits,
            curr_depth,
            is_static(call),
            outcome.is_success(),
        ) {
            set_revert(outcome, unmet.encode(|| self.signatures_identifier()));
            return;
        }

        // try to diagnose reverts in multi-fork mode where a call is made to an address that does
        // not exist
        if let Some(test_contract) = self.root_call_target {
            // if a call to a different contract than the original test contract returned with
            // `Stop` we check if the contract actually exists on the active fork
            if backend.backend().is_forked_mode()
                && outcome.stop == InstrStop::Stop
                && target != test_contract
            {
                self.fork_revert_diagnostic =
                    backend.backend().diagnose_revert(target, interp.host());
            }
        }

        // If the depth is 0, then this is the root call terminating
        if curr_depth == 0 {
            // Match expected calls
            if let Some(msg) =
                expect::first_unmet_call(&self.core.expected_calls, outcome.is_success())
            {
                set_revert(outcome, Error::encode(msg));
                return;
            }

            // Check if we have any leftover expected emits
            if let Some(msg) =
                expected_emit::first_unmet_root_emit(&mut self.expected_emits, outcome.is_success())
            {
                set_revert(outcome, Error::encode(msg));
                return;
            }

            // Check for leftover expected creates
            if let Some(msg) = expect::first_unmet_create(&self.expected_creates) {
                set_revert(outcome, Error::encode(msg));
            }
        }
    }

    /// Called before a create message executes.
    pub fn create_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        input: &mut MessageFor<FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
    ) -> Option<MessageResultFor<FEN>> {
        let tx_origin = interp.host().tx_env().origin;
        let curr_depth = usize::from(input.depth);
        self.start_created_accounts_frame(
            curr_depth == 0,
            CreatedAccountsFrameKind::Create,
            curr_depth,
        );
        if curr_depth == 0 {
            self.root_call_target = None;
            self.applied_access_list = None;
        }

        // Check if we should intercept this create
        if self.core.intercept_next_create_call {
            // Reset the flag
            self.core.intercept_next_create_call = false;

            // Return a revert with the initcode as error data
            return Some(message_result(input, InstrStop::Revert, input.input.clone()));
        }

        let mut caller_changed = false;

        // Apply our prank
        if let Some(prank) = self.get_prank(curr_depth)
            && let Some(changes) = prank.changes_for(curr_depth, input.caller)
        {
            if let Some(new_caller) = changes.caller {
                // Ensure new caller is loaded, warm and touched
                if let Ok(mut account) = interp.host().state_mut().account(&new_caller) {
                    account.touch();
                    account.warm();
                }
                input.caller = new_caller;
                caller_changed = true;
            }
            if let Some(new_origin) = changes.origin {
                self.set_origin(tx_origin, new_origin);
            }
            if let Some(used) = changes.used {
                self.pranks.insert(curr_depth, used);
            }
        }

        self.apply_access_list(interp);

        // Apply our broadcast
        if let Some(broadcast) = self.broadcast.clone()
            && curr_depth >= broadcast.depth
            && input.caller == broadcast.original_caller
        {
            // Reading the nonce warms the broadcaster.
            let nonce = match interp.host().state_mut().account(&broadcast.new_origin) {
                Ok(mut account) => {
                    account.warm();
                    account.nonce()
                }
                Err(err) => {
                    return Some(message_result(input, InstrStop::Revert, Error::encode(err)));
                }
            };

            self.set_origin(tx_origin, broadcast.new_origin);

            if curr_depth == broadcast.depth || broadcast.deploy_from_code {
                // Reset deploy from code flag for upcoming calls;
                if let Some(broadcast) = &mut self.broadcast {
                    broadcast.deploy_from_code = false;
                }

                input.caller = broadcast.new_origin;
                caller_changed = true;

                let rpc = backend.backend().active_fork_url();
                let mut tx_req = TransactionRequestFor::<FEN>::default()
                    .with_from(broadcast.new_origin)
                    .with_kind(TxKind::Create)
                    .with_value(input.value)
                    .with_input(input.input.clone())
                    .with_nonce(nonce);
                if let Some(fee_token) = self.fee_token {
                    tx_req.set_fee_token(fee_token);
                }
                self.broadcastable_transactions.push_back(BroadcastableTransaction {
                    rpc,
                    transaction: TransactionMaybeSigned::new(tx_req),
                    contract_address: None,
                });

                let kind = if input.kind == MessageKind::Create2 { "create2" } else { "create" };
                debug!(target: "cheatcodes", tx=?self.broadcastable_transactions.back().unwrap(), "broadcastable {kind}");
            }
        }

        // The creator changed, so the created address derives from the new creator.
        if caller_changed && curr_depth > 0 {
            let nonce = match interp.host().state_mut().account(&input.caller) {
                Ok(account) => account.nonce(),
                Err(err) => {
                    return Some(message_result(input, InstrStop::Revert, Error::encode(err)));
                }
            };
            input.destination = derive_create_destination(
                input.kind,
                &input.caller,
                &input.salt,
                &input.input,
                nonce,
            );
            input.call_target = input.destination;
        }

        // Allow cheatcodes from the address of the new contract
        let address = input.destination;
        Self::allow_cheatcodes_on_create(backend, curr_depth, input.caller, address);

        self.record_created_account(backend.backend().active_fork_id(), address);

        // If `recordAccountAccesses` has been called, record the create
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack {
            recorded_account_diffs_stack.push(vec![AccountAccess {
                chainInfo: crate::Vm::ChainInfo {
                    forkId: backend.backend().active_fork_id().unwrap_or_default(),
                    chainId: self
                        .env_overrides
                        .get(backend.backend().active_fork_id())
                        .and_then(|overrides| overrides.chain_id)
                        .map(U256::from)
                        .unwrap_or(interp.host().tx_env().chain_id),
                },
                accessor: input.caller,
                account: address,
                kind: crate::Vm::AccountAccessKind::Create,
                initialized: true,
                oldBalance: U256::ZERO, // updated on create_end
                newBalance: U256::ZERO, // updated on create_end
                oldNonce: 0,            // new contract starts with nonce 0
                newNonce: 1,            // updated on create_end (contracts start with nonce 1)
                value: input.value,
                data: input.input.clone(),
                reverted: false,
                deployedCode: Bytes::new(), // updated on create_end
                storageAccesses: vec![],    // updated on create_end
                depth: curr_depth as u64,
            }]);
        }

        None
    }

    /// Called after a create message executes.
    pub fn create_end_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        call: &MessageFor<FEN>,
        outcome: &mut MessageResultFor<FEN>,
        _backend: &mut dyn CheatcodesBackend<FEN>,
    ) {
        let curr_depth = usize::from(call.depth);
        self.replay_persistent_writes(interp, curr_depth, outcome.is_success());
        // Depth at which the caller resumes and applies the region snapshot correction.
        let isolated_snapshot_gas_used = self.gas_metering.take_isolated_snapshot_gas_used(
            self.deploy_code_caller_depth(curr_depth),
            &outcome.gas,
        );
        let tx_origin = interp.host().tx_env().origin;

        self.finish_created_accounts_frame(
            outcome.is_success(),
            CreatedAccountsFrameKind::Create,
            curr_depth,
        );

        // Clean up pranks
        if let Some(prank) = self.get_prank(curr_depth).copied()
            && curr_depth == prank.depth
        {
            self.set_origin(tx_origin, prank.prank_origin);

            // Clean single-call prank once we have returned to the original depth
            if prank.single_call {
                self.pranks.remove(&curr_depth);
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = self.broadcast.clone()
            && curr_depth == broadcast.depth
        {
            self.set_origin(tx_origin, broadcast.original_origin);

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
            if outcome.stop.is_revert()
                && expected_revert.reverter.is_some()
                && expected_revert.reverted_by.is_none()
            {
                expected_revert.reverted_by =
                    Some(outcome.created_address.unwrap_or(call.destination));
            }

            if curr_depth <= expected_revert.depth
                && matches!(expected_revert.kind, ExpectedRevertKind::Default)
            {
                let mut expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match revert_handlers::handle_expect_revert(
                    false,
                    true,
                    self.core.config.internal_expect_revert,
                    &expected_revert,
                    outcome.stop,
                    outcome.output.clone(),
                    &self.core.config.available_artifacts,
                ) {
                    Ok((address, retdata)) => {
                        expected_revert.actual_count += 1;
                        if expected_revert.actual_count < expected_revert.count {
                            // Reset so the next iteration's innermost CREATE wins again.
                            expected_revert.reverted_by = None;
                            self.expected_revert = Some(expected_revert.clone());
                        }

                        set_return(outcome, retdata);
                        outcome.created_address = address;
                        self.gas_metering.last_frame_gas = None;
                    }
                    Err(err) => {
                        set_revert(outcome, err.abi_encode().into());
                    }
                };
            }
        }

        if curr_depth > 0 {
            // Record the gas usage of the create frame, this allows the `lastFrameGas` cheatcode to
            // retrieve the gas usage of the last call or create.
            self.gas_metering.last_frame_gas = Some(frame_gas(outcome.stop, &outcome.gas));
            self.gas_metering.last_frame_snapshot_gas_used =
                isolated_snapshot_gas_used.unwrap_or_else(|| outcome.gas.spent());
        }

        // If `startStateDiffRecording` has been called, update the `reverted` status of the
        // previous call depth's recorded accesses, if any.
        if let Some(recorded_account_diffs_stack) = &mut self.recorded_account_diffs_stack
            && let Some(mut last_depth) = recorded_account_diffs_stack.pop()
        {
            // Update the reverted status of all deeper calls if this call reverted, in
            // accordance with EVM behavior.
            if outcome.stop.is_revert() {
                mark_account_accesses_reverted(&mut last_depth);
            }

            if let Some(create_access) = last_depth.first_mut() {
                // Update post-create state only if recording began before this frame.
                if create_access.depth == curr_depth as u64 {
                    debug_assert_eq!(
                        create_access.kind as u8,
                        crate::Vm::AccountAccessKind::Create as u8
                    );
                    if let Some(address) = outcome.created_address
                        && let Ok(mut account) = interp.host().state_mut().account(&address)
                    {
                        create_access.newBalance = account.balance();
                        create_access.newNonce = account.nonce();
                        create_access.deployedCode = account
                            .load_code()
                            .map(|code| code.original_bytes())
                            .unwrap_or_default();
                    }
                }
            }
            merge_recorded_frame(recorded_account_diffs_stack, last_depth);
        }

        // Match the create against expected_creates
        if !self.expected_creates.is_empty()
            && let Some(address) = outcome.created_address
            && let Ok(mut account) = interp.host().state_mut().account(&address)
        {
            let bytecode =
                account.load_code().map(|code| code.original_bytes()).unwrap_or_default();
            expect::observe_create(
                &mut self.expected_creates,
                call.caller,
                || call.kind.into(),
                &bytecode,
            );
        }
    }

    /// Determines whether the `DEFAULT_CREATE2_DEPLOYER` should be used for a CREATE message at
    /// `depth`.
    pub fn should_use_create2_factory(&mut self, depth: usize, message: &MessageFor<FEN>) -> bool {
        // Match the synthetic `deployCode` create at the depth of the cheatcode call, as for
        // native creates.
        let depth = self.deploy_code_caller_depth(depth);
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

        match message.kind {
            MessageKind::Create2 => {
                self.broadcast.is_some() || self.core.config.always_use_create_2_factory
            }
            MessageKind::Create => {
                self.core.config.batch_rewrite_creates && self.broadcast.is_some()
            }
            _ => false,
        }
    }

    /// Returns the CREATE2 deployer address.
    pub fn create2_deployer(&self) -> Address {
        self.core.config.evm_opts.create2_deployer
    }

    #[cold]
    fn meter_gas(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        if let Some(paused_gas) = self.gas_metering.paused_frames.last() {
            // Keep gas constant if paused.
            // Make sure we record the memory changes so that memory expansion is not paused.
            let memory = *interp.gas().memory();
            let mut gas = *paused_gas;
            gas.memory_mut().words_num = memory.words_num;
            gas.memory_mut().expansion_cost = memory.expansion_cost;
            interp.set_gas(gas);
        } else {
            // Record frame paused gas.
            self.gas_metering.paused_frames.push(interp.gas());
        }
    }

    #[cold]
    fn meter_gas_record(&mut self, interp: &InterpreterFor<'_, '_, FEN>, curr_depth: usize) {
        if interp.result().is_ok() {
            let isolated_region_gas = match self.gas_metering.pending_isolated_region_gas {
                Some((depth, charged, used)) if depth == curr_depth => {
                    self.gas_metering.pending_isolated_region_gas = None;
                    Some((charged, used))
                }
                _ => None,
            };
            let spent = interp.gas().tracker().spent();
            self.gas_metering.gas_records.iter_mut().for_each(|record| {
                if curr_depth == record.depth {
                    // Skip the first opcode of the first call frame as it includes the gas cost of
                    // creating the snapshot.
                    if self.gas_metering.last_gas_used != 0 {
                        let mut gas_diff = spent.saturating_sub(self.gas_metering.last_gas_used);
                        if let Some((charged, used)) = isolated_region_gas {
                            gas_diff = gas_diff.saturating_sub(charged).saturating_add(used);
                        }
                        record.gas_used = record.gas_used.saturating_add(gas_diff);
                    }

                    // Update `last_gas_used` to the current spent gas for the next iteration to
                    // compare against.
                    self.gas_metering.last_gas_used = spent;
                }
            });
        }
    }

    #[cold]
    fn meter_gas_end(&mut self, interp: &InterpreterFor<'_, '_, FEN>) {
        // Remove recorded gas if we exit frame.
        if interp.result().is_err() {
            self.gas_metering.paused_frames.pop();
        }
    }

    #[cold]
    const fn meter_gas_reset(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        let current = interp.gas();
        let mut gas =
            Gas::new_with_execution_gas_and_reservoir(current.limit(), current.reservoir());
        gas.memory_mut().words_num = current.memory().words_num;
        gas.memory_mut().expansion_cost = current.memory().expansion_cost;
        interp.set_gas(gas);
        self.gas_metering.reset = false;
    }

    #[cold]
    fn meter_gas_check(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        if interp.result().is_err() {
            // Reset gas if spent is less than refunded.
            // This can happen if gas was paused / resumed or reset.
            // https://github.com/foundry-rs/foundry/issues/4370
            let gas = interp.gas();
            if gas.spent() < u64::try_from(gas.refunded()).unwrap_or_default() {
                interp.set_gas(Gas::new_with_execution_gas_and_reservoir(
                    gas.limit(),
                    gas.reservoir(),
                ));
            }
        }
    }

    /// Applies opcode-level overrides for `BASEFEE`, `GASPRICE`, `BLOBHASH` and `CHAINID`.
    ///
    /// Called from `step_end` *after* the opcode has executed and only when the
    /// opcode succeeded. The opcode pushed its result onto the stack; we replace the top
    /// of stack with the cheatcode-set override. evm2 borrows the transaction environment for
    /// the whole transaction, so `vm.txGasPrice`, `vm.blobhashes` and `vm.chainId` become visible
    /// to the running transaction this way; inside isolated calls `vm.fee` is applied this way
    /// as well, since their nested transactions zero the real fee fields.
    ///
    /// We can't read the just-executed opcode from the interpreter here because the PC has already
    /// advanced; instead `step` stashes it in `env_overrides.pending_opcode` for us.
    #[cold]
    fn apply_env_overrides(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        fork_id: Option<U256>,
    ) {
        let Some(env_overrides) = self.env_overrides.get_mut(fork_id) else { return };
        let Some(opcode) = env_overrides.pending_opcode.take() else { return };
        // Each overridden opcode pushed one value; replace it with the override.
        let value = match opcode {
            op::BASEFEE => env_overrides.basefee_override().map(U256::from),
            op::GASPRICE => env_overrides.gas_price_override().map(U256::from),
            op::CHAINID => env_overrides.chain_id.map(U256::from),
            // BLOBHASH popped the index captured in `step` and pushed the hash.
            op::BLOBHASH => env_overrides
                .pending_blobhash_index
                .take()
                .and_then(|index| env_overrides.blob_hash_override(index))
                .map(Into::into),
            _ => None,
        };
        if let Some(value) = value {
            Self::replace_top_of_stack(interp, value);
        }
    }

    /// Replaces the top of the interpreter stack with `value`.
    ///
    /// The caller must only invoke this after a successful opcode that pushed
    /// a value onto the stack; the `pop()` is therefore expected to succeed.
    /// If it does not (e.g. because of a bug in the caller's success gating)
    /// we bail out instead of pushing on top of an unexpected stack, which
    /// would silently grow the stack and corrupt the frame.
    fn replace_top_of_stack(interp: &mut InterpreterFor<'_, '_, FEN>, value: U256) {
        let mut stack = interp.stack_mut();
        if stack.pop().is_err() {
            debug_assert!(false, "env override expected opcode result on stack");
            return;
        }
        let _ = stack.push(value);
    }

    /// Generates or copies arbitrary values for storage slots.
    /// Invoked in inspector `step_end` (when the current opcode is not executed), if current opcode
    /// to execute is `SLOAD` and storage slot is cold.
    /// Ensures that in next step (when `SLOAD` opcode is executed) an arbitrary value is returned:
    /// - copies the existing arbitrary storage value (or the new generated one if no value in
    ///   cache) from mapped source address to the target address.
    /// - generates arbitrary value and saves it in target address storage.
    #[cold]
    fn arbitrary_storage_end(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        if interp.result().is_err() || interp.opcode() != op::SLOAD {
            return;
        }
        let key = try_or_return!(interp.stack().peek(0));
        let target_address = interp.message().destination;

        if self.is_arbitrary_storage_slot_explicit(target_address, key) {
            return;
        }

        let state = interp.host().state_mut();
        let Ok(slot) = state.storage_slot(&target_address, key) else { return };
        let (is_cold, value) = (!slot.is_warm(), slot.current());

        if (is_cold && value.is_zero())
            || self.should_overwrite_arbitrary_storage(&target_address, key)
        {
            let result = if self.has_arbitrary_storage(&target_address) {
                let arbitrary_value = self
                    .cached_arbitrary_storage_value(target_address, key)
                    .unwrap_or_else(|| self.core.rng().random());
                self.core.arbitrary_storage.as_mut().unwrap().save(
                    state,
                    target_address,
                    key,
                    arbitrary_value,
                )
            } else if self.is_arbitrary_storage_copy(&target_address) {
                let arbitrary_value = self.core.rng().random();
                self.core
                    .arbitrary_storage
                    .as_mut()
                    .unwrap()
                    .copy(state, target_address, key, arbitrary_value)
                    .map(drop)
            } else {
                Ok(())
            };
            if let Err(err) = result {
                let _ = interp.fail(err);
            }
        }
    }

    fn take_storage_hook_inspector_state(&mut self) -> StorageHookInspectorState {
        StorageHookInspectorState {
            accesses: std::mem::take(&mut self.accesses),
            recording_accesses: std::mem::replace(&mut self.recording_accesses, false),
            mapping_slots: self.core.mapping_slots.take(),
            recorded_logs: self.core.recorded_logs.take(),
            recorded_logs_generation: self.core.recorded_logs_generation,
            mocked_calls: std::mem::take(&mut self.core.mocked_calls),
            mocked_functions: std::mem::take(&mut self.core.mocked_functions),
            expected_revert: self.expected_revert.take(),
            assume_no_revert: self.assume_no_revert.take(),
            expected_calls: std::mem::take(&mut self.core.expected_calls),
            expected_emits: std::mem::take(&mut self.expected_emits),
            expected_creates: std::mem::take(&mut self.expected_creates),
        }
    }

    fn restore_storage_hook_inspector_state(&mut self, state: StorageHookInspectorState) {
        self.accesses = state.accesses;
        self.recording_accesses = state.recording_accesses;
        self.core.mapping_slots = state.mapping_slots;
        self.core.recorded_logs = state.recorded_logs;
        self.core.recorded_logs_generation = state.recorded_logs_generation;
        self.core.mocked_calls = state.mocked_calls;
        self.core.mocked_functions = state.mocked_functions;
        self.expected_revert = state.expected_revert;
        self.assume_no_revert = state.assume_no_revert;
        self.core.expected_calls = state.expected_calls;
        self.expected_emits = state.expected_emits;
        self.expected_creates = state.expected_creates;
    }

    fn capture_storage_hook(&mut self, interp: &mut InterpreterFor<'_, '_, FEN>) {
        self.pending_storage_hook = None;
        if self.active_storage_hook.is_some() {
            return;
        }
        let account = interp.message().destination;
        match interp.opcode() {
            op::SLOAD => {
                let slot = try_or_return!(interp.stack().peek(0));
                let Some(hook) = self.storage_load_hooks.get(&account).copied() else { return };
                self.pending_storage_hook = Some(PendingStorageHook::Load { account, slot, hook });
            }
            op::SSTORE => {
                let slot = try_or_return!(interp.stack().peek(0));
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
                let old_value =
                    match interp.host().state_mut().storage_slot_untracked(&account, &slot) {
                        Ok(value) => value,
                        Err(err) => {
                            let _ = interp.fail(err);
                            return;
                        }
                    };
                self.pending_storage_hook =
                    Some(PendingStorageHook::Store { account, slot, old_value, mapping, hook });
            }
            _ => {}
        }
    }

    /// Calls the storage hook captured for the instruction that just executed, as a nested frame
    /// whose gas, return data and access warmth do not affect the hooked frame.
    fn invoke_pending_storage_hook(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) {
        let Some(pending) = self.pending_storage_hook.take() else { return };
        if interp.result().is_err() {
            return;
        }

        let (hook, input) = match pending {
            PendingStorageHook::Load { account, slot, hook } => {
                let value = try_or_return!(interp.stack().peek(0));
                let mut input = Vec::with_capacity(4 + 32 * 3);
                input.extend_from_slice(&hook.callback_selector);
                input.extend_from_slice(account.into_word().as_slice());
                input.extend_from_slice(&slot.to_be_bytes::<32>());
                input.extend_from_slice(&value.to_be_bytes::<32>());
                (hook, Bytes::from(input))
            }
            PendingStorageHook::Store { account, slot, old_value, mapping, hook } => {
                let new_value =
                    interp.host().state().get_storage(&account, &slot).unwrap_or_default();
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
                (hook, Bytes::from(input))
            }
        };

        let depth = interp.message().depth.saturating_add(1);
        let gas_limit = interp.gas().remaining();
        let evm = interp.host();
        let checkpoint = evm.state().checkpoint();
        let code = match evm
            .state_mut()
            .account(&hook.callback_target)
            .and_then(|mut account| account.load_code())
        {
            Ok(code) => code,
            Err(err) => {
                revert_frame(interp, &Error::encode(err));
                return;
            }
        };
        let mut message = MessageFor::<FEN> {
            kind: MessageKind::Call,
            depth,
            gas_limit,
            destination: hook.callback_target,
            call_target: hook.callback_target,
            caller: CHEATCODE_ADDRESS,
            input: input.clone(),
            code,
            code_address: hook.callback_target,
            ..Default::default()
        };

        let inspector_state = self.take_storage_hook_inspector_state();
        self.active_storage_hook = Some(ActiveStorageHook {
            depth,
            callback_target: hook.callback_target,
            callback_input: input,
        });
        let result = executor.exec_message(self, backend, evm, &mut message);
        self.active_storage_hook = None;
        self.restore_storage_hook_inspector_state(inspector_state);
        // The callback is instrumentation: accesses it warmed stay cold for the hooked frame.
        restore_access_warmth(evm.state_mut(), &checkpoint);

        match result {
            Ok(result) if result.is_success() => {}
            Ok(result) => revert_frame(interp, &result.output),
            Err(err) => {
                let _ = interp.fail(err);
            }
        }
    }

    /// Records storage slots reads and writes.
    #[cold]
    fn record_accesses(&mut self, interp: &InterpreterFor<'_, '_, FEN>) {
        let access = &mut self.accesses;
        match interp.opcode() {
            op::SLOAD => {
                let key = try_or_return!(interp.stack().peek(0));
                access.record_read(interp.message().destination, key);
            }
            op::SSTORE => {
                let key = try_or_return!(interp.stack().peek(0));
                access.record_write(interp.message().destination, key);
            }
            _ => {}
        }
    }

    #[cold]
    fn record_state_diffs(
        &mut self,
        interp: &mut InterpreterFor<'_, '_, FEN>,
        backend: &mut dyn CheatcodesBackend<FEN>,
        curr_depth: usize,
    ) {
        let Some(account_accesses) = &mut self.recorded_account_diffs_stack else { return };
        let opcode = interp.opcode();
        let accessor = interp.message().destination;
        let fork_id = backend.backend().active_fork_id();
        let chain_id = self
            .env_overrides
            .get(fork_id)
            .and_then(|overrides| overrides.chain_id)
            .map(U256::from)
            .unwrap_or(interp.host().tx_env().chain_id);
        let fork_id = fork_id.unwrap_or_default();
        let depth = curr_depth as u64;
        match opcode {
            op::SELFDESTRUCT => {
                // Ensure that we're not selfdestructing a context recording was initiated on
                let Some(last) = account_accesses.last_mut() else { return };

                // get previous balance, nonce and initialized status of the target account
                let target = try_or_return!(interp.stack().peek(0));
                let target = Address::from_word(B256::from(target));
                let state = interp.host().state_mut();
                let (initialized, old_balance, old_nonce) = state
                    .account(&target)
                    .map(|mut account| {
                        account.warm();
                        (
                            account.get().is_some_and(|info| !info.is_empty()),
                            account.balance(),
                            account.nonce(),
                        )
                    })
                    .unwrap_or_default();

                // load balance of this account
                let value = state
                    .account_info_untracked(&accessor)
                    .ok()
                    .flatten()
                    .map_or(U256::ZERO, |info| info.balance);

                // register access for the target account
                last.push(crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo { forkId: fork_id, chainId: chain_id },
                    accessor,
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
                    depth,
                });
            }

            op::SLOAD => {
                let Some(last) = account_accesses.last_mut() else { return };

                let key = try_or_return!(interp.stack().peek(0));

                // Try to include present value for informational purposes, otherwise assume
                // it's not set (zero value). The read does not warm the slot for the actual
                // SLOAD opcode.
                let present_value = interp
                    .host()
                    .state_mut()
                    .storage_slot_untracked(&accessor, &key)
                    .unwrap_or_default();
                let access = crate::Vm::StorageAccess {
                    account: accessor,
                    slot: key.into(),
                    isWrite: false,
                    previousValue: present_value.into(),
                    newValue: present_value.into(),
                    reverted: false,
                };
                append_storage_access(last, access, depth);
            }
            op::SSTORE => {
                let Some(last) = account_accesses.last_mut() else { return };

                let key = try_or_return!(interp.stack().peek(0));
                let value = try_or_return!(interp.stack().peek(1));
                // Try to load the slot's previous value, otherwise, assume it's not set (zero
                // value). The read does not warm the slot for the actual SSTORE opcode.
                let previous_value = interp
                    .host()
                    .state_mut()
                    .storage_slot_untracked(&accessor, &key)
                    .unwrap_or_default();

                let access = crate::Vm::StorageAccess {
                    account: accessor,
                    slot: key.into(),
                    isWrite: true,
                    previousValue: previous_value.into(),
                    newValue: value.into(),
                    reverted: false,
                };
                append_storage_access(last, access, depth);
            }

            // Record account accesses via the EXT family of opcodes
            op::EXTCODECOPY | op::EXTCODESIZE | op::EXTCODEHASH | op::BALANCE => {
                let kind = match opcode {
                    op::EXTCODECOPY => crate::Vm::AccountAccessKind::Extcodecopy,
                    op::EXTCODESIZE => crate::Vm::AccountAccessKind::Extcodesize,
                    op::EXTCODEHASH => crate::Vm::AccountAccessKind::Extcodehash,
                    op::BALANCE => crate::Vm::AccountAccessKind::Balance,
                    _ => unreachable!(),
                };
                let address =
                    Address::from_word(B256::from(try_or_return!(interp.stack().peek(0))));
                let (initialized, balance, nonce) = interp
                    .host()
                    .state_mut()
                    .account_info_untracked(&address)
                    .ok()
                    .flatten()
                    .map(|info| (!info.is_empty(), info.balance, info.nonce))
                    .unwrap_or_default();
                let account_access = crate::Vm::AccountAccess {
                    chainInfo: crate::Vm::ChainInfo { forkId: fork_id, chainId: chain_id },
                    accessor,
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
                    depth,
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
    fn check_mem_opcodes(&self, interpreter: &mut InterpreterFor<'_, '_, FEN>, depth: u64) {
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
                match interpreter.opcode() {
                    ////////////////////////////////////////////////////////////////
                    //    OPERATIONS THAT CAN EXPAND/MUTATE MEMORY BY WRITING     //
                    ////////////////////////////////////////////////////////////////

                    op::MSTORE => {
                        // The offset of the mstore operation is at the top of the stack.
                        let offset = try_or_return!(interpreter.stack().peek(0)).saturating_to::<u64>();

                        // If none of the allowed ranges contain [offset, offset + 32), memory has been
                        // unexpectedly mutated.
                        if !ranges.iter().any(|range| {
                            range.contains(&offset) && range.contains(&(offset + 31))
                        }) {
                            // SPECIAL CASE: When the compiler attempts to store the selector for
                            // `stopExpectSafeMemory`, this is allowed. It will do so at the current free memory
                            // pointer, which could have been updated to the exclusive upper bound during
                            // execution.
                            let value = try_or_return!(interpreter.stack().peek(1)).to_be_bytes::<32>();
                            if value[..SELECTOR_LEN] == stopExpectSafeMemoryCall::SELECTOR {
                                return
                            }

                            disallowed_mem_write(offset, 32, interpreter, ranges);
                            return
                        }
                    }
                    op::MSTORE8 => {
                        // The offset of the mstore8 operation is at the top of the stack.
                        let offset = try_or_return!(interpreter.stack().peek(0)).saturating_to::<u64>();

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
                        let offset = try_or_return!(interpreter.stack().peek(0)).saturating_to::<u64>();

                        // If the offset being loaded is >= than the memory size, the
                        // memory is being expanded. If none of the allowed ranges contain
                        // [offset, offset + 32), memory has been unexpectedly mutated.
                        if offset >= interpreter.memory().len() as u64 && !ranges.iter().any(|range| {
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
                        let dest_offset = try_or_return!(interpreter.stack().peek(5)).saturating_to::<u64>();

                        // The size of the data that will be copied is the sixth element on the stack.
                        let size = try_or_return!(interpreter.stack().peek(6)).saturating_to::<u64>();

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
                            let to = Address::from_word(try_or_return!(interpreter.stack().peek(1)).to_be_bytes::<32>().into());
                            if to == CHEATCODE_ADDRESS {
                                let args_offset = try_or_return!(interpreter.stack().peek(3)).saturating_to::<usize>();
                                let args_size = try_or_return!(interpreter.stack().peek(4)).saturating_to::<usize>();
                                // CALL has not expanded input memory yet.
                                if args_size >= SELECTOR_LEN
                                    && args_offset.saturating_add(args_size) <= interpreter.memory().len()
                                {
                                    let memory_word = interpreter.memory().slice(args_offset, args_size);
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
                        let dest_offset = try_or_return!(interpreter.stack().peek($offset_depth)).saturating_to::<u64>();

                        // The size of the data that will be copied.
                        let size = try_or_return!(interpreter.stack().peek($size_depth)).saturating_to::<u64>();

                        // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                        // memory outside of the expected ranges has been touched. If the opcode
                        // only reads from memory, this is okay as long as the memory is not expanded.
                        let fail_cond = !ranges.iter().any(|range| {
                                range.contains(&dest_offset) &&
                                    range.contains(&(dest_offset + size.saturating_sub(1)))
                            }) && ($writes ||
                                [dest_offset, (dest_offset + size).saturating_sub(1)].into_iter().any(|offset| {
                                    offset >= interpreter.memory().len() as u64
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
    fn set_gas_limit_type(&mut self, interpreter: &InterpreterFor<'_, '_, FEN>) {
        match interpreter.opcode() {
            // Both creation opcodes forward remaining gas, even when rewritten to factory calls.
            op::CREATE | op::CREATE2 => self.dynamic_gas_limit = true,
            op::CALL => {
                // If first element of the stack is close to current remaining gas then assume
                // dynamic gas limit.
                self.dynamic_gas_limit = try_or_return!(interpreter.stack().peek(0))
                    >= U256::from(interpreter.gas().remaining().saturating_sub(100))
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
fn disallowed_mem_write<T: EvmTypesHost>(
    dest_offset: u64,
    size: u64,
    interpreter: &mut Interpreter<'_, '_, T>,
    ranges: &[Range<u64>],
) {
    let revert_string = format!(
        "memory write at offset 0x{:02X} of size 0x{:02X} not allowed; safe range: {}",
        dest_offset,
        size,
        ranges.iter().map(|r| format!("[0x{:02X}, 0x{:02X})", r.start, r.end)).join(" U ")
    );

    revert_frame(interpreter, revert_string.as_bytes());
}

/// The instructions during which the running frame can emit a log: `LOG*`, and the message and
/// self-destruct instructions, such as for logs of precompiles and value transfers.
const LOG_EMITTING_OPCODES: OpcodeSet = opcodes([
    op::LOG0,
    op::LOG1,
    op::LOG2,
    op::LOG3,
    op::LOG4,
    op::CALL,
    op::CALLCODE,
    op::DELEGATECALL,
    op::STATICCALL,
    op::CREATE,
    op::CREATE2,
    op::SELFDESTRUCT,
]);

/// Returns the set of `ops`.
const fn opcodes<const N: usize>(ops: [u8; N]) -> OpcodeSet {
    let mut set = OpcodeSet::EMPTY;
    let mut i = 0;
    while i < N {
        set.insert(ops[i]);
        i += 1;
    }
    set
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
    let result = if ccx.state.core.config.blocked_cheatcodes.contains(&cheat.func.selector_bytes) {
        Err(fmt_err!("disabled during restricted execution"))
    } else {
        vm_calls!(dispatch)
    }
    .map_err(|error| error.with_cheatcode_name(cheat));

    trace!(
        target: "cheatcodes",
        return = %match &result {
            Ok(b) => hex::encode(b),
            Err(e) => e.to_string(),
        }
    );

    result
}

/// Returns the gas a frame used, as reported by `lastCallGas` and `lastFrameGas`.
const fn frame_gas(stop: InstrStop, gas: &GasTracker) -> Vm::Gas {
    // A halt consumes the regular gas restored while rolling back state gas.
    let regular_gas_spent = if stop.is_halt() {
        gas.spent()
    } else {
        gas.spent().saturating_sub(gas.state_gas_spilled())
    };
    Vm::Gas {
        gasLimit: gas.limit(),
        gasTotalUsed: regular_gas_spent,
        gasMemoryUsed: 0,
        gasRefunded: gas.refunded(),
        gasRemaining: gas.remaining(),
        gasStateUsed: if stop.is_success() { gas.state_gas_spent() } else { 0 },
    }
}

/// Turns `outcome` into a revert with `output`.
fn set_revert<E>(outcome: &mut MessageResultExt<E>, output: Bytes) {
    outcome.stop = InstrStop::Revert;
    outcome.output = output;
}

/// Turns `outcome` into a successful return with `output`.
fn set_return<E>(outcome: &mut MessageResultExt<E>, output: Bytes) {
    outcome.stop = InstrStop::Return;
    outcome.output = output;
}

/// Bumps the nonces of the authorities of `authorizations` that the transaction will apply, as
/// EIP-7702 does, for a transaction that is broadcast but not executed.
fn apply_authorization_nonces<FEN: FoundryEvmNetwork>(
    evm: &mut FoundryEvmFor<'_, FEN>,
    authorizations: &[SignedAuthorization],
    sender: Address,
    chain_id: u64,
) -> Result<Vec<PersistentWrite>> {
    let mut writes = Vec::new();
    for auth in authorizations {
        if (!auth.chain_id.is_zero() && auth.chain_id != U256::from(chain_id))
            || auth.nonce() == u64::MAX
        {
            continue;
        }
        let Ok(authority) = auth.recover_authority() else { continue };
        // The authority code check is skipped because attaching the delegation already replaced
        // the local code that EIP-7702 validates.
        let mut account = evm.state_mut().account(&authority)?;
        account.touch();
        account.warm();
        // The sender nonce has not been incremented for the transaction yet.
        let nonce = account.nonce();
        if auth.nonce() == nonce + u64::from(authority == sender) {
            account.set_nonce(nonce + 1);
            writes.push(PersistentWrite::nonce(&account, nonce));
        }
    }
    Ok(writes)
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

    fn create_inputs() -> MessageFor<EthEvmNetwork> {
        MessageFor::<EthEvmNetwork> {
            kind: MessageKind::Create,
            gas_limit: 100_000,
            ..Default::default()
        }
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
        assert_eq!(cheats.steps(), OpcodeSet::EMPTY);
        assert!(!cheats.has_log_hooks());
    }

    #[test]
    fn active_cheatcode_state_enables_opcode_hooks() {
        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());

        cheats.recording_accesses = true;
        assert_eq!(cheats.steps(), [op::SLOAD, op::SSTORE].into_iter().collect());

        cheats.recording_accesses = false;
        cheats.gas_metering.touched = true;
        assert_eq!(cheats.steps(), OpcodeSet::ALL);

        cheats.gas_metering.touched = false;
        cheats.register_storage_load_hook(Address::ZERO, Address::ZERO, [0; 4]);
        assert_eq!(cheats.steps(), OpcodeSet::ALL);
    }

    #[test]
    fn combined_cheatcodes_step_every_opcode_any_needs() {
        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());
        cheats.recording_accesses = true;
        cheats.gas_price = Some(1);
        cheats.env_overrides.update(None, |o| o.chain_id = Some(1));
        assert_eq!(
            cheats.steps(),
            [
                op::SLOAD,
                op::SSTORE,
                op::ORIGIN,
                op::GASPRICE,
                op::BASEFEE,
                op::CHAINID,
                op::BLOBHASH
            ]
            .into_iter()
            .collect()
        );

        cheats.gas_metering.reset = true;
        assert_eq!(cheats.steps(), OpcodeSet::ALL);
    }

    #[test]
    fn env_override_hook_predicates() {
        fn assert_hooks(cheats: &Cheatcodes, active: bool, case: &str) {
            let steps = if active {
                [op::BASEFEE, op::GASPRICE, op::CHAINID, op::BLOBHASH].into_iter().collect()
            } else {
                OpcodeSet::EMPTY
            };
            assert_eq!(cheats.steps(), steps, "{case}");
        }

        let mut cheats = Cheatcodes::<EthEvmNetwork>::new(Arc::default());
        assert_hooks(&cheats, false, "empty map");
        let snapshot_id = U256::from(7);
        cheats.env_overrides.save_snapshot(snapshot_id);

        cheats.env_overrides.update(None, |_| {});
        assert_hooks(&cheats, false, "inactive entry");

        let active = [
            EnvOverrides { basefee: Some(1), ..Default::default() },
            EnvOverrides { implicit_basefee: Some(1), ..Default::default() },
            EnvOverrides { gas_price: Some(1), ..Default::default() },
            EnvOverrides { blob_hashes: Some(vec![B256::ZERO]), ..Default::default() },
            EnvOverrides { chain_id: Some(1), ..Default::default() },
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

        cheats.core.recorded_logs = Some(Default::default());
        assert!(cheats.has_log_hooks());

        cheats.core.recorded_logs = None;
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
        for mut gas in [
            GasTracker::new(100_000),
            GasTracker::new_with_execution_gas_and_reservoir(100_000, 50_000),
        ] {
            gas.spend(1_000).unwrap();
            gas.spend_state(20_000).unwrap();

            let reported = frame_gas(InstrStop::Stop, &gas);
            assert_eq!(reported.gasTotalUsed, 1_000);
            assert_eq!(reported.gasStateUsed, 20_000);

            assert_eq!(frame_gas(InstrStop::Revert, &gas).gasStateUsed, 0);
        }

        let mut gas = GasTracker::new(100_000);
        gas.refill_reservoir(20_000);
        assert_eq!(frame_gas(InstrStop::Stop, &gas).gasStateUsed, -20_000);

        let mut gas = GasTracker::new(100_000);
        gas.spend_state(20_000).unwrap();
        gas.settle_gas(InstrStop::OutOfGas);
        let reported = frame_gas(InstrStop::OutOfGas, &gas);
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

    #[test]
    fn set_arbitrary_storage_applies_through_core_context() {
        let mut cheats = cheats(false, None);
        let target = Address::repeat_byte(0x11);
        let slot = U256::from(7);
        assert_eq!(cheats.steps(), OpcodeSet::EMPTY);

        Vm::setArbitraryStorage_1Call { target, overwrite: true }
            .apply_context(&mut cheats.core)
            .unwrap();
        assert_eq!(cheats.steps(), OpcodeSet::ALL);
        assert!(cheats.has_arbitrary_storage(&target));
        assert!(cheats.should_overwrite_arbitrary_storage(&target, slot));

        cheats.mark_arbitrary_storage_slot_explicit(target, slot);
        cheats.cache_arbitrary_storage_value(target, slot, U256::from(42));
        assert!(cheats.is_arbitrary_storage_slot_explicit(target, slot));
        assert_eq!(cheats.cached_arbitrary_storage_value(target, slot), Some(U256::from(42)));

        // Re-registering clears the overwrite mode, explicit slots, and cached values.
        Vm::setArbitraryStorage_0Call { target }.apply_context(&mut cheats.core).unwrap();
        assert_eq!(
            cheats.arbitrary_storage_target_overwrite_modes().collect_vec(),
            [(target, false)]
        );
        assert!(!cheats.is_arbitrary_storage_slot_explicit(target, slot));
        assert_eq!(cheats.cached_arbitrary_storage_value(target, slot), None);
    }
}
