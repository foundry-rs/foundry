//! Implementations of [`Evm`](spec::Group::Evm) cheatcodes.

use crate::{
    BroadcastableTransaction, Cheatcode, Cheatcodes, CheatcodesCore, CheatcodesExecutor,
    CheatsCtxt, Error, Result, Vm::*, env::FORGE_CONTEXT, inspector::RecordDebugStepInfo,
};
use alloy_consensus::{Transaction as _, Typed2718, transaction::SignerRecoverable};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{
    Address, B256, Bytes, KECCAK256_EMPTY, U256, hex, keccak256,
    map::{AddressMap, AddressSet, B256Map, HashMap},
};
use alloy_rlp::Decodable;
use alloy_sol_types::SolValue;
use evm2::{
    EvmFeatures, SpecId,
    bytecode::Bytecode,
    evm::{DbResult, State, StorageSlotHandle},
    interpreter::Host,
    registry::HandlerError,
};
use foundry_common::{
    TransactionMaybeSigned,
    fs::{read_json_file, write_json_file},
    slot_identifier::{
        ENCODING_BYTES, ENCODING_DYN_ARRAY, ENCODING_INPLACE, ENCODING_MAPPING, SlotIdentifier,
        SlotInfo,
    },
    tempo::{TIP20_MAX_LOGO_URI_BYTES, Tip20LogoUriValidationError, validate_tip20_logo_uri},
};
use foundry_evm_core::{
    backend::{RevertStateSnapshotAction, transaction_error},
    constants::{CALLER, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, TEST_CONTRACT_ADDRESS},
    eip2935::{
        HISTORY_SERVE_WINDOW, HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE, forward_fill_start,
        history_storage_slot, history_storage_value,
    },
    evm::{
        EvmEnvFor, EvmFactoryFor, ExecutionPhase, FoundryEvmFactory, FoundryEvmNetwork,
        PersistentWrite, TxEnvelopeFor, TxPayload, evm_env,
    },
};
use foundry_evm_traces::TraceRequirements;
use itertools::Itertools;
use rand::Rng;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fmt::Display,
    path::Path,
    str::FromStr,
};

mod record_debug_step;
use foundry_common::fmt::format_token_raw;
use foundry_config::{ExecutionSpec, evm_spec_id_from_str, fs_permissions::FsAccessKind};
use record_debug_step::{convert_call_trace_ctx_to_debug_step, flatten_call_trace};
use serde::{Serialize, Serializer, ser::SerializeMap};

mod fork;
pub(crate) mod mapping;
pub(crate) mod mock;
pub(crate) mod prank;

/// JSON-serializable log entry for `getRecordedLogsJson`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogJson {
    /// The topics of the log, including the signature, if any.
    topics: Vec<String>,
    /// The raw data of the log, hex-encoded with 0x prefix.
    data: String,
    /// The address of the log's emitter.
    emitter: String,
}

struct StateDump<'a>(&'a [(Address, GenesisAccount)]);

impl Serialize for StateDump<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (address, account) in self.0 {
            map.serialize_entry(address, account)?;
        }
        map.end()
    }
}

/// Records storage slots reads and writes.
#[derive(Clone, Debug, Default)]
pub struct RecordAccess {
    /// Storage slots reads.
    pub reads: HashMap<Address, Vec<U256>>,
    /// Storage slots writes.
    pub writes: HashMap<Address, Vec<U256>>,
}

impl RecordAccess {
    /// Records a read access to a storage slot.
    pub fn record_read(&mut self, target: Address, slot: U256) {
        self.reads.entry(target).or_default().push(slot);
    }

    /// Records a write access to a storage slot.
    ///
    /// This also records a read internally as `SSTORE` does an implicit `SLOAD`.
    pub fn record_write(&mut self, target: Address, slot: U256) {
        self.record_read(target, slot);
        self.writes.entry(target).or_default().push(slot);
    }

    /// Clears the recorded reads and writes.
    pub fn clear(&mut self) {
        // Also frees memory.
        *self = Default::default();
    }
}

/// Records the `snapshotGas*` cheatcodes.
#[derive(Clone, Debug)]
pub struct GasRecord {
    /// The group name of the gas snapshot.
    pub group: String,
    /// The name of the gas snapshot.
    pub name: String,
    /// The total gas used in the gas snapshot.
    pub gas_used: u64,
    /// Depth at which the gas snapshot was taken.
    pub depth: usize,
}

/// Storage slot diff info.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SlotStateDiff {
    /// Initial storage value.
    previous_value: B256,
    /// Current storage value.
    new_value: B256,
    /// Storage layout metadata (variable name, type, offset).
    /// Only present when contract has storage layout output.
    /// This includes decoded values when available.
    #[serde(skip_serializing_if = "Option::is_none", flatten)]
    slot_info: Option<SlotInfo>,
}

/// Balance diff info.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct BalanceDiff {
    /// Initial storage value.
    previous_value: U256,
    /// Current storage value.
    new_value: U256,
}

/// Nonce diff info.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct NonceDiff {
    /// Initial nonce value.
    previous_value: u64,
    /// Current nonce value.
    new_value: u64,
}

/// Account state diff info.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AccountStateDiffs {
    /// Address label, if any set.
    label: Option<String>,
    /// Contract identifier from artifact. e.g "src/Counter.sol:Counter"
    contract: Option<String>,
    /// Account balance changes.
    balance_diff: Option<BalanceDiff>,
    /// Account nonce changes.
    nonce_diff: Option<NonceDiff>,
    /// State changes, per slot.
    state_diff: BTreeMap<B256, SlotStateDiff>,
}

impl Display for AccountStateDiffs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> eyre::Result<(), std::fmt::Error> {
        // Print changed account.
        if let Some(label) = &self.label {
            writeln!(f, "label: {label}")?;
        }
        if let Some(contract) = &self.contract {
            writeln!(f, "contract: {contract}")?;
        }
        // Print balance diff if changed.
        if let Some(balance_diff) = &self.balance_diff
            && balance_diff.previous_value != balance_diff.new_value
        {
            writeln!(
                f,
                "- balance diff: {} → {}",
                balance_diff.previous_value, balance_diff.new_value
            )?;
        }
        // Print nonce diff if changed.
        if let Some(nonce_diff) = &self.nonce_diff
            && nonce_diff.previous_value != nonce_diff.new_value
        {
            writeln!(f, "- nonce diff: {} → {}", nonce_diff.previous_value, nonce_diff.new_value)?;
        }
        // Print state diff if any.
        if !&self.state_diff.is_empty() {
            writeln!(f, "- state diff:")?;
            for (slot, slot_changes) in &self.state_diff {
                match &slot_changes.slot_info {
                    Some(slot_info)
                        if slot_info.members.as_ref().is_some_and(|members| {
                            members.iter().any(|member| member.decoded.is_some())
                        }) =>
                    {
                        // Keep raw values because decoded members may omit unsupported fields.
                        writeln!(
                            f,
                            "@ {slot} ({}, {}): {} → {}",
                            slot_info.label,
                            slot_info.slot_type.dyn_sol_type,
                            slot_changes.previous_value,
                            slot_changes.new_value
                        )?;
                        for member in slot_info.members.iter().flatten() {
                            let Some(decoded) = &member.decoded else { continue };
                            writeln!(
                                f,
                                "    {} ({}): {} → {}",
                                member.label,
                                member.slot_type.dyn_sol_type,
                                format_token_raw(&decoded.previous_value),
                                format_token_raw(&decoded.new_value)
                            )?;
                        }
                    }
                    Some(slot_info) => {
                        if let Some(decoded) = &slot_info.decoded {
                            // Have slot info with decoded values - show decoded values
                            writeln!(
                                f,
                                "@ {slot} ({}, {}): {} → {}",
                                slot_info.label,
                                slot_info.slot_type.dyn_sol_type,
                                format_token_raw(&decoded.previous_value),
                                format_token_raw(&decoded.new_value)
                            )?;
                        } else {
                            // Have slot info but no decoded values - show raw hex values
                            writeln!(
                                f,
                                "@ {slot} ({}, {}): {} → {}",
                                slot_info.label,
                                slot_info.slot_type.dyn_sol_type,
                                slot_changes.previous_value,
                                slot_changes.new_value
                            )?;
                        }
                    }
                    None => {
                        // No slot info - show raw hex values
                        writeln!(
                            f,
                            "@ {slot}: {} → {}",
                            slot_changes.previous_value, slot_changes.new_value
                        )?;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Cheatcode for addrCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { privateKey } = self;
        super::crypto::with_private_key_signer(state, privateKey, |wallet| {
            Ok(wallet.address().abi_encode())
        })
    }
}

impl Cheatcode for getNonce_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        get_nonce(ccx, account)
    }
}

impl Cheatcode for getNonce_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { wallet } = self;
        get_nonce(ccx, &wallet.addr)
    }
}

impl Cheatcode for loadCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot } = *self;

        ccx.evm.state_mut().account(&target)?.warm();
        let (is_cold, mut value) = {
            let mut slot = ccx
                .evm
                .state_mut()
                .storage_slot(&target, slot.into())
                .map_err(|e| fmt_err!("failed to load storage slot: {e}"))?;
            (slot.warm(), slot.current())
        };

        if is_cold
            && value.is_zero()
            && !ccx.state.is_arbitrary_storage_slot_explicit(target, slot.into())
        {
            if ccx.state.has_arbitrary_storage(&target) {
                // If storage slot is untouched and load from a target with arbitrary storage,
                // then set random value for current slot.
                let rand_value = ccx
                    .state
                    .cached_arbitrary_storage_value(target, slot.into())
                    .unwrap_or_else(|| ccx.state.core.rng().random());
                ccx.state.core.arbitrary_storage.as_mut().unwrap().save(
                    ccx.evm.state_mut(),
                    target,
                    slot.into(),
                    rand_value,
                )?;
                value = rand_value;
            } else if ccx.state.is_arbitrary_storage_copy(&target) {
                // If storage slot is untouched and load from a target that copies storage from
                // a source address with arbitrary storage, then copy existing arbitrary value.
                // If no arbitrary value generated yet, then the random one is saved and set.
                let rand_value = ccx.state.core.rng().random();
                value = ccx.state.core.arbitrary_storage.as_mut().unwrap().copy(
                    ccx.evm.state_mut(),
                    target,
                    slot.into(),
                    rand_value,
                )?;
            }
        }

        Ok(value.abi_encode())
    }
}

impl Cheatcode for loadTransientCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot } = *self;
        let value = ccx.evm.state_mut().tload(&target, &slot.into());
        Ok(B256::from(value).abi_encode())
    }
}

impl Cheatcode for loadAllocsCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { pathToAllocsJson } = self;

        let path = Path::new(pathToAllocsJson);
        ensure!(path.exists(), "allocs file does not exist: {pathToAllocsJson}");

        // Let's first assume we're reading a file with only the allocs.
        let allocs: BTreeMap<Address, GenesisAccount> = match read_json_file(path) {
            Ok(allocs) => allocs,
            Err(_) => {
                // Let's try and read from a genesis file, and extract allocs.
                let genesis = read_json_file::<Genesis>(path)?;
                genesis.alloc
            }
        };

        // Then, load the allocs into the database.
        EvmFactoryFor::<FEN>::default().sync_transaction(ccx.evm);
        let (evm, db) = ccx.evm_db_mut();
        db.load_allocs(&allocs, evm).map_err(|e| fmt_err!("failed to load allocs: {e}"))?;
        EvmFactoryFor::<FEN>::default().refresh_transaction(ccx.evm);
        Ok(Default::default())
    }
}

impl Cheatcode for cloneAccountCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { source, target } = self;

        ccx.evm.state_mut().account(source)?.warm();
        let slots = ccx.backend.backend().lifecycle().loaded_storage(ccx.evm.state(), source);
        let genesis = genesis_account(ccx.evm.state_mut(), source, slots)?;
        EvmFactoryFor::<FEN>::default().sync_transaction(ccx.evm);
        let (evm, db) = ccx.evm_db_mut();
        db.clone_account(&genesis, target, evm)?;
        EvmFactoryFor::<FEN>::default().refresh_transaction(ccx.evm);
        // Cloned account should persist in forked envs.
        ccx.db_mut().add_persistent_account(*target);
        Ok(Default::default())
    }
}

impl Cheatcode for dumpStateCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { pathToStateJson } = self;
        let path =
            ccx.state.core.config.ensure_path_allowed(pathToStateJson, FsAccessKind::Write)?;
        ccx.state.core.config.ensure_not_foundry_toml(&path)?;

        let fork_id = ccx.active_fork_id();
        let state = ccx.evm.state_mut();
        let created_accounts = ccx.state.created_accounts(fork_id);

        // Do not include system account or empty accounts in the dump.
        let skip = |key: &Address| {
            key == &CHEATCODE_ADDRESS
                || key == &CALLER
                || key == &HARDHAT_CONSOLE_ADDRESS
                || key == &TEST_CONTRACT_ADDRESS
                || key == &ccx.caller
                || key == &ccx.state.core.config.evm_opts.sender
        };

        let accounts = ccx.backend.backend().lifecycle().loaded_state(state);
        let mut alloc = BTreeMap::new();
        for (address, slots) in accounts {
            if skip(&address)
                || state.account_info_untracked(&address)?.is_none_or(|info| info.is_empty())
            {
                continue;
            }
            alloc.insert(address, genesis_account(state, &address, slots)?);
        }

        let mut ordered_alloc = Vec::with_capacity(alloc.len());
        for address in created_accounts {
            if let Some(account) = alloc.remove(&address) {
                ordered_alloc.push((address, account));
            }
        }
        ordered_alloc.extend(alloc);

        write_json_file(&path, &StateDump(&ordered_alloc))?;
        Ok(Default::default())
    }
}

impl Cheatcode for recordCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.recording_accesses = true;
        ccx.state.accesses.clear();
        Ok(Default::default())
    }
}

impl Cheatcode for stopRecordCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        ccx.state.recording_accesses = false;
        Ok(Default::default())
    }
}

impl Cheatcode for accessesCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target } = *self;
        let result = (
            ccx.state.accesses.reads.entry(target).or_default().as_slice(),
            ccx.state.accesses.writes.entry(target).or_default().as_slice(),
        );
        Ok(result.abi_encode_params())
    }
}

impl Cheatcode for registerSloadHookCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, callback } = *self;
        ccx.state.register_storage_load_hook(target, ccx.caller, callback.0);
        Ok(Default::default())
    }
}

impl Cheatcode for registerSstoreHookCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, callback } = *self;
        if ccx.state.has_mapping_storage_store_hooks(target) {
            bail!("cannot register raw SSTORE hook: mapping SSTORE hooks already exist for target");
        }
        ccx.state.register_storage_store_hook(target, ccx.caller, callback.0);
        Ok(Default::default())
    }
}

impl Cheatcode for registerMappingSstoreHookCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, rootSlot, callback } = *self;
        if !ccx.state.register_mapping_storage_store_hook(target, rootSlot, ccx.caller, callback.0)
        {
            bail!("cannot register mapping SSTORE hook: raw SSTORE hook already exists for target");
        }
        Ok(Default::default())
    }
}

impl Cheatcode for recordLogsCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self {} = self;
        state.recorded_logs = Some(Default::default());
        state.recorded_logs_generation += 1;
        Ok(Default::default())
    }
}

impl Cheatcode for getRecordedLogsCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self {} = self;
        state.recorded_logs_generation += 1;
        Ok(state.recorded_logs.replace(Default::default()).unwrap_or_default().abi_encode())
    }
}

impl Cheatcode for getRecordedLogsJsonCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self {} = self;
        state.recorded_logs_generation += 1;
        let logs = state.recorded_logs.replace(Default::default()).unwrap_or_default();
        let json_logs: Vec<_> = logs
            .into_iter()
            .map(|log| LogJson {
                topics: log.topics.iter().map(|t| format!("{t}")).collect(),
                data: hex::encode_prefixed(&log.data),
                emitter: format!("{}", log.emitter),
            })
            .collect();
        Ok(serde_json::to_string(&json_logs)?.abi_encode())
    }
}

impl Cheatcode for pauseGasMeteringCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.gas_metering.paused = true;
        Ok(Default::default())
    }
}

impl Cheatcode for resumeGasMeteringCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.gas_metering.resume();
        Ok(Default::default())
    }
}

impl Cheatcode for resetGasMeteringCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.gas_metering.reset();
        Ok(Default::default())
    }
}

impl Cheatcode for lastCallGasCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        let Some(last_call_gas) = &ccx.state.gas_metering.last_call_gas else {
            bail!("no external call was made yet");
        };
        Ok(last_call_gas.abi_encode())
    }
}

impl Cheatcode for lastFrameGasCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        let Some(last_frame_gas) = &ccx.state.gas_metering.last_frame_gas else {
            bail!("no external call or create was made yet");
        };
        Ok(last_frame_gas.abi_encode())
    }
}

impl Cheatcode for getChainIdCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        Ok(U256::from(ccx.chain_id()).abi_encode())
    }
}

impl Cheatcode for chainIdCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newChainId } = self;
        ensure!(*newChainId <= U256::from(u64::MAX), "chain ID must be less than 2^64");
        let chain_id = newChainId.to();
        // `CHAINID` reads the new chain ID right away; transaction validation uses it from the
        // next transaction on.
        let fork_id = ccx.active_fork_id();
        ccx.state.env_overrides.update(fork_id, |o| o.chain_id = Some(chain_id));
        let mut env = evm_env(ccx.evm);
        env.set_chain_id(chain_id);
        let (evm, db) = ccx.evm_db_mut();
        // A chain ID change is deferred to the next transaction while one is in flight.
        db.set_env(evm, env, ExecutionPhase::InFlight);
        Ok(Default::default())
    }
}

impl Cheatcode for coinbaseCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newCoinbase } = self;
        ccx.update_block(|block| block.beneficiary = *newCoinbase);
        Ok(Default::default())
    }
}

impl Cheatcode for difficultyCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newDifficulty } = self;
        ensure!(
            ccx.spec().into() < SpecId::MERGE,
            "`difficulty` is not supported after the Paris hard fork, use `prevrandao` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.update_block(|block| block.difficulty = *newDifficulty);
        Ok(Default::default())
    }
}

impl Cheatcode for feeCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newBasefee } = self;
        ensure!(*newBasefee <= U256::from(u64::MAX), "base fee must be less than 2^64");
        let basefee: u64 = newBasefee.saturating_to();
        // Always record the override so `BASEFEE` reads the cheatcode value
        // even inside the synthetic isolation transaction (which zeroes
        // `block.basefee` for fee-accounting). See `EnvOverrides`.
        let fork_id = ccx.active_fork_id();
        ccx.state.env_overrides.update(fork_id, |o| o.basefee = Some(basefee));
        // Outside isolation, also mutate the real env to preserve the
        // historical behavior other code paths rely on.
        if !ccx.state.in_isolation_context {
            ccx.update_block(|block| block.basefee = U256::from(basefee));
        }
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.spec().into() >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.update_block(|block| block.prevrandao = (*newPrevrandao).into());
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.spec().into() >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.update_block(|block| block.prevrandao = *newPrevrandao);
        Ok(Default::default())
    }
}

impl Cheatcode for blobhashesCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { hashes } = self;
        ensure!(
            ccx.spec().into() >= SpecId::CANCUN,
            "`blobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        // The transaction environment is fixed for the running transaction, so `BLOBHASH` reads
        // the cheatcode value through an override. See `EnvOverrides`.
        let fork_id = ccx.active_fork_id();
        ccx.state.env_overrides.update(fork_id, |o| o.blob_hashes = Some(hashes.clone()));
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobhashesCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ensure!(
            ccx.spec().into() >= SpecId::CANCUN,
            "`getBlobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        let fork_id = ccx.active_fork_id();
        let hashes = ccx
            .state
            .env_overrides
            .get(fork_id)
            .and_then(|o| o.blob_hashes.clone())
            .unwrap_or_else(|| ccx.tx_blob_hashes());
        Ok(hashes.abi_encode())
    }
}

impl Cheatcode for rollCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newHeight } = self;
        let current_height = ccx.block_number();
        if ccx.spec().into() >= SpecId::PRAGUE && *newHeight > current_height {
            let mut block_number = forward_fill_start(current_height, *newHeight);
            while block_number < *newHeight {
                let block_hash = Host::block_hash(ccx.evm, &block_number).unwrap_or_default();
                set_eip2935_blockhash(ccx.evm.state_mut(), block_number, block_hash)?;
                block_number += U256::ONE;
            }
        }
        ccx.update_block(|block| block.number = *newHeight);
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockNumberCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        Ok(ccx.block_number().abi_encode())
    }
}

impl Cheatcode for rollSlotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        ensure!(
            ccx.spec().into() >= SpecId::AMSTERDAM,
            "`rollSlot` is not supported before the Amsterdam hard fork; \
             see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
        );
        ccx.update_block(|block| block.slot_num = U256::from(self.newSlotNumber));
        Ok(Default::default())
    }
}

impl Cheatcode for getSlotNumberCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        ensure!(
            ccx.spec().into() >= SpecId::AMSTERDAM,
            "`getSlotNumber` is not supported before the Amsterdam hard fork; \
             see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
        );
        Ok(ccx.slot_num().abi_encode())
    }
}

impl Cheatcode for txGasPriceCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newGasPrice } = self;
        ensure!(*newGasPrice <= U256::from(u64::MAX), "gas price must be less than 2^64");
        let gas_price: u128 = newGasPrice.saturating_to();
        // The transaction environment is fixed for the running transaction, so `GASPRICE` reads
        // the cheatcode value through an override; fee accounting keeps the transaction's gas
        // price. See `EnvOverrides`.
        let fork_id = ccx.active_fork_id();
        ccx.state.env_overrides.update(fork_id, |o| o.gas_price = Some(gas_price));
        Ok(Default::default())
    }
}

impl Cheatcode for warpCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newTimestamp } = self;
        ccx.update_block(|block| block.timestamp = *newTimestamp);
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockTimestampCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        Ok(ccx.timestamp().abi_encode())
    }
}

impl Cheatcode for blobBaseFeeCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { newBlobBaseFee } = self;
        ensure!(
            ccx.spec().into() >= SpecId::CANCUN,
            "`blobBaseFee` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );

        ensure!(*newBlobBaseFee <= U256::from(u128::MAX), "blob base fee must be less than 2^128");
        ccx.update_block(|block| block.blob_basefee = *newBlobBaseFee);
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobBaseFeeCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        Ok(ccx.evm.block().blob_basefee.abi_encode())
    }
}

impl Cheatcode for dealCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account: address, newBalance: new_balance } = *self;
        EvmFactoryFor::<FEN>::default().sync_transaction(ccx.evm);
        let old_balance = {
            let mut account = ccx.evm.state_mut().account(&address)?;
            let old_balance = account.balance();
            account.warm();
            account.set_balance(new_balance);
            old_balance
        };
        // The balance survives reverts of the enclosing calls.
        let write = PersistentWrite::Balance(address, new_balance.wrapping_sub(old_balance));
        ccx.state.record_persistent(ccx.caller_depth(), ccx.evm.state(), [write]);
        EvmFactoryFor::<FEN>::default().refresh_transaction(ccx.evm);
        Ok(Default::default())
    }
}

impl Cheatcode for etchCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, newRuntimeBytecode } = self;
        ccx.ensure_not_precompile(target)?;
        let bytecode = Bytecode::new_raw_checked(newRuntimeBytecode.clone())
            .map_err(|e| fmt_err!("failed to create bytecode: {e}"))?;
        let is_forked = ccx.is_forked_mode();
        let features = ccx.evm.version().features;
        let state = ccx.evm.state_mut();
        // Clear synthetic history only in local mode. On forks, preserve storage as for other
        // etched accounts; clearing the ring can require one synchronous RPC read per uncached
        // slot.
        if *target == HISTORY_STORAGE_ADDRESS
            && !is_forked
            && bytecode.hash_slow() != keccak256(&HISTORY_STORAGE_CODE)
            && state.account(target)?.code_hash() == keccak256(&HISTORY_STORAGE_CODE)
        {
            clear_eip2935_history_storage(state, features)?;
        }
        let mut account = state.account(target)?;
        account.warm();
        account.set_code_slow(bytecode);
        Ok(Default::default())
    }
}

impl Cheatcode for resetNonceCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account } = self;
        let mut account = ccx.evm.state_mut().account(account)?;
        // Per EIP-161, contract nonces start at 1. EOAs, including EIP-7702 delegated ones, start
        // at 0.
        let code_hash = account.code_hash();
        let is_eoa = code_hash == KECCAK256_EMPTY
            || code_hash.is_zero()
            || account.load_code()?.is_eip7702();
        let nonce = if is_eoa { 0 } else { 1 };
        account.touch();
        account.warm();
        let previous = account.nonce();
        account.set_nonce(nonce);
        let write = PersistentWrite::nonce(&account, previous);
        drop(account);
        // Nonce cheatcodes survive reverts of the enclosing calls.
        ccx.state.record_persistent(ccx.caller_depth(), ccx.evm.state(), [write]);
        debug!(target: "cheatcodes", nonce, "reset");
        Ok(Default::default())
    }
}

impl Cheatcode for setNonceCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account, newNonce } = *self;
        let mut account = ccx.evm.state_mut().account(&account)?;
        // nonce must increment only
        let current = account.nonce();
        ensure!(
            newNonce >= current,
            "new nonce ({newNonce}) must be strictly equal to or higher than the \
             account's current nonce ({current})"
        );
        account.touch();
        account.warm();
        let previous = account.nonce();
        account.set_nonce(newNonce);
        let write = PersistentWrite::nonce(&account, previous);
        drop(account);
        // Nonce cheatcodes survive reverts of the enclosing calls.
        ccx.state.record_persistent(ccx.caller_depth(), ccx.evm.state(), [write]);
        Ok(Default::default())
    }
}

impl Cheatcode for setNonceUnsafeCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { account, newNonce } = *self;
        let mut account = ccx.evm.state_mut().account(&account)?;
        account.touch();
        account.warm();
        let previous = account.nonce();
        account.set_nonce(newNonce);
        let write = PersistentWrite::nonce(&account, previous);
        drop(account);
        // Nonce cheatcodes survive reverts of the enclosing calls.
        ccx.state.record_persistent(ccx.caller_depth(), ccx.evm.state(), [write]);
        Ok(Default::default())
    }
}

impl Cheatcode for storeCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot, value } = *self;
        ccx.ensure_not_precompile(&target)?;
        let state = ccx.evm.state_mut();
        touch_account(state, target)?;
        warm_storage_slot(state, &target, slot.into())
            .map_err(|e| fmt_err!("failed to store storage slot: {e}"))?
            .set(value.into());
        ccx.state.mark_arbitrary_storage_slot_explicit(target, slot.into());
        Ok(Default::default())
    }
}

impl Cheatcode for storeTransientCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot, value } = *self;
        ccx.ensure_not_precompile(&target)?;
        ccx.evm.state_mut().tstore(&target, &slot.into(), &value.into());
        Ok(Default::default())
    }
}

impl Cheatcode for setTip20LogoURICall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { token, newLogoURI } = self;
        set_tip20_logo_uri(ccx, token, newLogoURI)
    }
}

impl Cheatcode for setLogoURICall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { token, newLogoURI } = self;
        set_tip20_logo_uri(ccx, token, newLogoURI)
    }
}

impl Cheatcode for coolCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target } = self;
        let slots = ccx.backend.backend().lifecycle().loaded_storage(ccx.evm.state(), target);
        // Like REVM's `WarmAddresses`, precompiles and the addresses the transaction warmed for
        // its coinbase, access list and chain stay warm. The rest of the transaction-base
        // warmth, such as the recipient, is cooled.
        let keep = ccx.is_precompile(target) || ccx.state.tx_warm_addresses.contains(target);
        if !keep {
            let (evm, db) = ccx.evm_db_mut();
            db.remove_prewarm(evm.state_mut(), target);
        }
        let state = ccx.evm.state_mut();
        state.set_account_warm(target, false);
        for slot in slots {
            state.set_storage_warm(target, slot, false);
        }
        Ok(Default::default())
    }
}

impl Cheatcode for accessListCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self { access } = self;
        let access_list = access
            .iter()
            .map(|item| {
                let keys = item.storageKeys.iter().map(|key| B256::from(*key)).collect_vec();
                alloy_rpc_types::AccessListItem { address: item.target, storage_keys: keys }
            })
            .collect_vec();
        state.access_list = Some(alloy_rpc_types::AccessList::from(access_list));
        Ok(Default::default())
    }
}

impl Cheatcode for noAccessListCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self {} = self;
        // Set to empty option in order to override previous applied access list.
        if state.access_list.is_some() {
            state.access_list = Some(alloy_rpc_types::AccessList::default());
        }
        Ok(Default::default())
    }
}

impl Cheatcode for warmSlotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot } = *self;
        set_cold_slot(ccx, target, slot.into(), false);
        Ok(Default::default())
    }
}

impl Cheatcode for coolSlotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, slot } = *self;
        set_cold_slot(ccx, target, slot.into(), true);
        Ok(Default::default())
    }
}

impl Cheatcode for isIsolateModeCall {
    fn apply_context(&self, state: &mut CheatcodesCore) -> Result {
        let Self {} = self;
        Ok(state.config.isolate.abi_encode())
    }
}

impl Cheatcode for readCallersCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        read_callers(ccx.state, &ccx.tx_caller(), ccx.depth())
    }
}

impl Cheatcode for snapshotValue_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name, value } = self;
        inner_value_snapshot(ccx, None, Some(name.clone()), value.to_string())
    }
}

impl Cheatcode for snapshotValue_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { group, name, value } = self;
        inner_value_snapshot(ccx, Some(group.clone()), Some(name.clone()), value.to_string())
    }
}

impl Cheatcode for snapshotGasLastCall_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name } = self;
        if ccx.state.gas_metering.last_call_gas.is_none() {
            bail!("no external call was made yet");
        }
        let gas_used = ccx.state.gas_metering.last_call_snapshot_gas_used;
        inner_last_gas_snapshot(ccx, None, Some(name.clone()), gas_used)
    }
}

impl Cheatcode for snapshotGasLastCall_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name, group } = self;
        if ccx.state.gas_metering.last_call_gas.is_none() {
            bail!("no external call was made yet");
        }
        let gas_used = ccx.state.gas_metering.last_call_snapshot_gas_used;
        inner_last_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()), gas_used)
    }
}

impl Cheatcode for snapshotGasLastFrame_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name } = self;
        if ccx.state.gas_metering.last_frame_gas.is_none() {
            bail!("no external call or create was made yet");
        }
        let gas_used = ccx.state.gas_metering.last_frame_snapshot_gas_used;
        inner_last_gas_snapshot(ccx, None, Some(name.clone()), gas_used)
    }
}

impl Cheatcode for snapshotGasLastFrame_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name, group } = self;
        if ccx.state.gas_metering.last_frame_gas.is_none() {
            bail!("no external call or create was made yet");
        }
        let gas_used = ccx.state.gas_metering.last_frame_snapshot_gas_used;
        inner_last_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()), gas_used)
    }
}

impl Cheatcode for startSnapshotGas_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name } = self;
        inner_start_gas_snapshot(ccx, None, Some(name.clone()))
    }
}

impl Cheatcode for startSnapshotGas_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { group, name } = self;
        inner_start_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()))
    }
}

impl Cheatcode for stopSnapshotGas_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        inner_stop_gas_snapshot(ccx, None, None)
    }
}

impl Cheatcode for stopSnapshotGas_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { name } = self;
        inner_stop_gas_snapshot(ccx, None, Some(name.clone()))
    }
}

impl Cheatcode for stopSnapshotGas_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { group, name } = self;
        inner_stop_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()))
    }
}

// Deprecated in favor of `snapshotStateCall`
impl Cheatcode for snapshotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

impl Cheatcode for snapshotStateCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

// Deprecated in favor of `revertToStateCall`
impl Cheatcode for revertToCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

// Deprecated in favor of `revertToStateAndDeleteCall`
impl Cheatcode for revertToAndDeleteCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateAndDeleteCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

// Deprecated in favor of `deleteStateSnapshotCall`
impl Cheatcode for deleteSnapshotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        let result = ccx.db_mut().delete_state_snapshot(*snapshotId);
        ccx.state.env_overrides.delete_snapshot(*snapshotId);
        ccx.state.fork_block_number_override_snapshots.remove(snapshotId);
        ccx.state.delete_created_accounts_snapshot(*snapshotId);
        Ok(result.abi_encode())
    }
}

impl Cheatcode for deleteStateSnapshotCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { snapshotId } = self;
        let result = ccx.db_mut().delete_state_snapshot(*snapshotId);
        ccx.state.env_overrides.delete_snapshot(*snapshotId);
        ccx.state.fork_block_number_override_snapshots.remove(snapshotId);
        ccx.state.delete_created_accounts_snapshot(*snapshotId);
        Ok(result.abi_encode())
    }
}

// Deprecated in favor of `deleteStateSnapshotsCall`
impl Cheatcode for deleteSnapshotsCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.db_mut().delete_state_snapshots();
        ccx.state.env_overrides.clear_snapshots();
        ccx.state.fork_block_number_override_snapshots.clear();
        ccx.state.clear_created_accounts_snapshots();
        Ok(Default::default())
    }
}

impl Cheatcode for deleteStateSnapshotsCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.db_mut().delete_state_snapshots();
        ccx.state.env_overrides.clear_snapshots();
        ccx.state.fork_block_number_override_snapshots.clear();
        ccx.state.clear_created_accounts_snapshots();
        Ok(Default::default())
    }
}

impl Cheatcode for startStateDiffRecordingCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.start_state_diff_recording();
        // Enable mapping recording to track mapping slot accesses
        ccx.state.core.mapping_slots.get_or_insert_default();
        Ok(Default::default())
    }
}

impl Cheatcode for stopAndReturnStateDiffCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        get_state_diff(ccx.state)
    }
}

impl Cheatcode for getStateDiffCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let mut diffs = String::new();
        let state_diffs = get_recorded_state_diffs(ccx);
        for (address, state_diffs) in state_diffs {
            diffs.push_str(&format!("{address}\n"));
            diffs.push_str(&format!("{state_diffs}\n"));
        }
        Ok(diffs.abi_encode())
    }
}

impl Cheatcode for getStateDiffJsonCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let state_diffs = get_recorded_state_diffs(ccx);
        Ok(serde_json::to_string(&state_diffs)?.abi_encode())
    }
}

impl Cheatcode for getStorageSlotsCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { target, variableName } = self;

        let storage_layout = get_contract_data(ccx, *target, true)
            .and_then(|(_, data)| data.storage_layout.as_ref().map(|layout| layout.clone()))
            .ok_or_else(|| fmt_err!("Storage layout not available for contract at {target}. Try compiling contracts with `--extra-output storageLayout`"))?;

        trace!(storage = ?storage_layout.storage, "fetched storage");

        let variable_name_lower = variableName.to_lowercase();
        let storage = storage_layout
            .storage
            .iter()
            .find(|s| s.label.to_lowercase() == variable_name_lower)
            .ok_or_else(|| fmt_err!("variable '{variableName}' not found in storage layout"))?;

        let storage_type = storage_layout
            .types
            .get(&storage.storage_type)
            .ok_or_else(|| fmt_err!("storage type not found for variable {variableName}"))?;

        if storage_type.encoding == ENCODING_MAPPING || storage_type.encoding == ENCODING_DYN_ARRAY
        {
            return Err(fmt_err!(
                "cannot get storage slots for variables with mapping or dynamic array types"
            ));
        }

        let slot = U256::from_str(&storage.slot).map_err(|_| {
            fmt_err!("invalid slot {} format for variable {variableName}", storage.slot)
        })?;

        let mut slots = Vec::new();

        // Always push the base slot
        slots.push(slot);

        if storage_type.encoding == ENCODING_INPLACE {
            // For inplace encoding, calculate the number of slots needed
            let num_bytes = U256::from_str(&storage_type.number_of_bytes).map_err(|_| {
                fmt_err!(
                    "invalid number_of_bytes {} for variable {variableName}",
                    storage_type.number_of_bytes
                )
            })?;
            let num_slots = num_bytes.div_ceil(U256::from(32));
            let num_slots = usize::try_from(num_slots).map_err(|_| {
                fmt_err!("number_of_bytes {} exceeds host usize", storage_type.number_of_bytes)
            })?;

            // Start from 1 since base slot is already added
            for i in 1..num_slots {
                slots.push(slot + U256::from(i));
            }
        }

        if storage_type.encoding == ENCODING_BYTES {
            // Try to check if it's a long bytes/string by reading the current storage
            // value
            if let Ok(value) =
                warm_storage_slot(ccx.evm.state_mut(), target, slot).map(|s| s.current())
            {
                let value_bytes = value.to_be_bytes::<32>();
                let length_byte = value_bytes[31];
                // Check if it's a long bytes/string (LSB is 1)
                if length_byte & 1 == 1 {
                    // Calculate data slots for long bytes/string
                    let length: U256 = value >> 1;
                    let length = usize::try_from(length)
                        .map_err(|_| fmt_err!("long bytes/string length exceeds host usize"))?;
                    let num_data_slots = length.div_ceil(32);
                    let data_start = Into::<U256>::into(keccak256(B256::from(slot).0));

                    for i in 0..num_data_slots {
                        slots.push(data_start + U256::from(i));
                    }
                }
            }
        }

        Ok(slots.abi_encode())
    }
}

impl Cheatcode for getStorageAccessesCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let mut storage_accesses = Vec::new();

        for account_access in ccx.state.recorded_account_diffs() {
            storage_accesses.extend(account_access.storageAccesses.clone());
        }

        Ok(storage_accesses.abi_encode())
    }
}

impl Cheatcode for broadcastRawTransactionCall {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let tx = TxEnvelopeFor::<FEN>::decode(&mut self.data.as_ref())
            .map_err(|err| fmt_err!("failed to decode RLP-encoded transaction: {err}"))?;

        let sender =
            tx.recover_signer().map_err(|err| fmt_err!("failed to recover signer: {err}"))?;
        let from = sender;
        let contract_address = if tx.kind().is_create() {
            let mut account = ccx.evm.state_mut().account(&sender)?;
            account.warm();
            Some(sender.create(account.nonce()))
        } else {
            None
        };

        let evm_tx = FEN::evm_tx(tx.clone(), sender, TxPayload::Signed)?;
        let overrides = ccx.state.env_overrides.get(ccx.active_fork_id());
        let chain_id = overrides.and_then(|o| o.chain_id);
        let custom_env = chain_id.is_some()
            || ccx.state.isolation_env.is_some()
            || ccx.spec() != ccx.evm.config_spec_id();
        let env = custom_env.then(|| {
            let mut env = ccx.evm_env();
            // Inside an isolated call, the transaction pays fees as in the transaction the call
            // runs in, with the requested spec's gas schedule and validation rules.
            if let Some(outer) = &ccx.state.isolation_env {
                env.version =
                    EvmFactoryFor::<FEN>::default().env_with_spec(*outer, env.spec).version;
                env.block.basefee = overrides
                    .and_then(|o| o.basefee_override())
                    .map_or(outer.block.basefee, U256::from);
            }
            // The transaction validates with a chain ID `vm.chainId` set in the running one.
            if let Some(chain_id) = chain_id {
                env.set_chain_id(chain_id);
            }
            env
        });
        let previous = enter_tx_warm_addresses(
            ccx.state,
            &env.unwrap_or_else(|| ccx.evm_env()),
            tx.access_list(),
        );
        let result = executor.transact_from_tx(
            ccx.state,
            ccx.backend,
            ccx.evm,
            ccx.depth as u16,
            &evm_tx,
            env,
        );
        ccx.state.tx_warm_addresses = previous;
        let _ = result?;
        ccx.state.state_epoch += 1;

        if ccx.state.broadcast.is_some() {
            ccx.state.broadcastable_transactions.push_back(BroadcastableTransaction {
                rpc: ccx.active_fork_url(),
                transaction: TransactionMaybeSigned::Signed { tx, from },
                contract_address,
            });
        }

        Ok(Default::default())
    }
}

impl Cheatcode for setBlockhashCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { blockNumber, blockHash } = *self;
        ensure!(blockNumber <= U256::from(u64::MAX), "blockNumber must be less than 2^64");
        ensure!(
            blockNumber <= ccx.block_number(),
            "block number must be less than or equal to the current block number"
        );

        let (evm, db) = ccx.evm_db_mut();
        db.set_blockhash(blockNumber, blockHash, evm);
        let current_block = ccx.block_number();
        if ccx.spec().into() >= SpecId::PRAGUE
            && blockNumber < current_block
            && current_block - blockNumber <= U256::from(HISTORY_SERVE_WINDOW)
        {
            set_eip2935_blockhash(ccx.evm.state_mut(), blockNumber, blockHash)?;
        }

        Ok(Default::default())
    }
}

impl Cheatcode for executeTransactionCall {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        // Block in script contexts.
        if let Some(ctx) = FORGE_CONTEXT.get()
            && *ctx == ForgeContext::ScriptGroup
        {
            return Err(fmt_err!("executeTransaction is not allowed in forge script"));
        }

        // Decode the RLP-encoded signed transaction.
        let tx = TxEnvelopeFor::<FEN>::decode(&mut self.rawTx.as_ref())
            .map_err(|err| fmt_err!("failed to decode RLP-encoded transaction: {err}"))?;

        ensure!(
            tx.ty() != 0x79,
            "EIP-8130 transactions are not supported by vm.executeTransaction"
        );

        let sender =
            tx.recover_signer().map_err(|err| fmt_err!("failed to recover signer: {err}"))?;
        let created_address = tx.kind().is_create().then(|| sender.create(tx.nonce()));
        let access_list = tx.access_list().cloned();
        let value = tx.value();
        let tx = FEN::evm_tx(tx, sender, TxPayload::Signed)?;

        // Execute in the running environment with realistic validation: no base fee or fee
        // payment, nonce checks, the spec's transaction gas limit cap, and an initcode limit
        // derived from the configured code size limit rather than the unlimited one used for
        // test contracts.
        let mut env = ccx.evm_env();
        env.block.basefee = U256::ZERO;
        env.set_feature(EvmFeatures::FEE_CHARGE, false);
        env.set_feature(EvmFeatures::NONCE_CHECK, true);
        // As on master, which zeroes the gas price, the sender only needs to afford the value.
        env.set_feature(EvmFeatures::BALANCE_CHECK, false);
        env.set_feature(EvmFeatures::BALANCE_TOP_UP, false);
        let balance = ccx
            .evm
            .state_mut()
            .account_info_untracked(&sender)?
            .map_or(U256::ZERO, |account| account.balance);
        if balance < value {
            let err = transaction_error(HandlerError::InsufficientFunds { fee: value, balance });
            return Err(fmt_err!("transaction execution failed: {err}"));
        }
        // The nested transaction validates with a chain ID `vm.chainId` set in the running one.
        if let Some(chain_id) =
            ccx.state.env_overrides.get(ccx.active_fork_id()).and_then(|o| o.chain_id)
        {
            env.set_chain_id(chain_id);
        }
        let default_version = EvmFactoryFor::<FEN>::default().version(env.spec, env.chain_id());
        env.version.tx_gas_limit_cap = default_version.tx_gas_limit_cap;
        env.version.max_initcode_size = match ccx.state.core.config.evm_opts.env.code_size_limit {
            Some(code_size_limit) => code_size_limit.saturating_mul(2),
            None => default_version.max_initcode_size,
        };

        if let Some(address) = created_address {
            let fork_id = ccx.active_fork_id();
            ccx.state.record_created_account(fork_id, address);
        }

        let previous = enter_tx_warm_addresses(ccx.state, &env, access_list.as_ref());
        let res = executor.nested_transaction(
            ccx.state,
            ccx.backend,
            ccx.evm,
            ccx.depth as u16,
            &tx,
            Some(env),
        );
        ccx.state.tx_warm_addresses = previous;
        let res = res.map_err(|e| fmt_err!("transaction execution failed: {e}"))?;

        // Return output bytes.
        if !res.status {
            if res.stop.is_revert() {
                return Err(fmt_err!(
                    "transaction reverted: {}",
                    hex::encode_prefixed(&res.output)
                ));
            }
            return Err(fmt_err!("transaction halted: {:?}", res.stop));
        }

        Ok(res.output.abi_encode())
    }
}

impl Cheatcode for startDebugTraceRecordingCall {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let Some(tracer) = executor.tracing_inspector() else {
            return Err(Error::from("no tracer initiated, consider adding -vvv flag"));
        };

        if ccx.state.record_debug_steps_info.is_some() {
            bail!("debug trace recording was already started");
        }

        let mut info = RecordDebugStepInfo {
            // will be updated later
            start_node_idx: 0,
            // keep the original config to revert back later
            original_tracer_config: *tracer.config(),
        };

        // turn on tracer debug configuration for recording
        *tracer.config_mut() =
            TraceRequirements::none().with_debug(true).into_config().expect("cannot be None");

        // track where the recording starts
        if let Some(last_node) = tracer.traces().nodes().last() {
            info.start_node_idx = last_node.idx;
        }

        ccx.state.record_debug_steps_info = Some(info);
        Ok(Default::default())
    }
}

impl Cheatcode for stopAndReturnDebugTraceRecordingCall {
    fn apply_full<FEN: FoundryEvmNetwork>(
        &self,
        ccx: &mut CheatsCtxt<'_, '_, FEN>,
        executor: &mut dyn CheatcodesExecutor<FEN>,
    ) -> Result {
        let Some(tracer) = executor.tracing_inspector() else {
            return Err(Error::from("no tracer initiated, consider adding -vvv flag"));
        };

        let Some(record_info) = ccx.state.record_debug_steps_info else {
            return Err(Error::from("nothing recorded"));
        };

        // Use the trace nodes to flatten the call trace
        let root = tracer.traces();
        let steps = flatten_call_trace(0, root, record_info.start_node_idx);

        let debug_steps: Vec<DebugStep> =
            steps.iter().map(|step| convert_call_trace_ctx_to_debug_step(step)).collect();
        // Free up memory by clearing the steps if they are not recorded outside of cheatcode usage.
        if !record_info.original_tracer_config.record_steps {
            tracer.traces_mut().nodes_mut().iter_mut().for_each(|node| {
                node.trace.steps = Vec::new();
                node.logs = Vec::new();
                node.ordering = Vec::new();
            });
        }

        // Revert the tracer config to the one before recording
        tracer.update_config(|_config| record_info.original_tracer_config);

        // Clean up the recording info
        ccx.state.record_debug_steps_info = None;

        Ok(debug_steps.abi_encode())
    }
}

impl Cheatcode for setEvmVersionCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { evm } = self;
        let spec_id = evm_spec_id_from_str(evm)
            .ok_or_else(|| Error::from(format!("invalid evm version {evm}")))?;
        ccx.state.execution_evm_version = Some(spec_id);
        Ok(Default::default())
    }
}

impl Cheatcode for getEvmVersionCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let spec = ccx.spec();
        Ok(spec.evm_version_name().to_lowercase().abi_encode())
    }
}

pub(super) fn get_nonce<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    address: &Address,
) -> Result {
    let mut account = ccx.evm.state_mut().account(address)?;
    account.warm();
    let nonce = account.nonce();
    Ok(nonce.abi_encode())
}

fn inner_snapshot_state<FEN: FoundryEvmNetwork>(ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
    let (evm, db) = ccx.evm_db_mut();
    let id = db.snapshot_state(evm);
    let fork_id = ccx.active_fork_id();
    ccx.state.env_overrides.save_snapshot(id);
    ccx.state.fork_block_number_override_snapshots.insert(id, ccx.state.fork_block_number_override);
    ccx.state.snapshot_created_accounts(id, fork_id);
    ccx.state.persistent_write_snapshots.insert(id, ccx.state.persistent_writes.len());
    Ok(id.abi_encode())
}

/// Zeroes the restored base fee again inside an isolated call, whose fee accounting runs without
/// one, and keeps the restored value visible to `BASEFEE`.
fn restore_isolation_fee_accounting<FEN: FoundryEvmNetwork>(ccx: &mut CheatsCtxt<'_, '_, FEN>) {
    if !ccx.state.in_isolation_context {
        return;
    }

    let basefee = ccx.basefee();
    if basefee != 0 {
        let fork_id = ccx.active_fork_id();
        ccx.state.env_overrides.update(fork_id, |o| {
            o.implicit_basefee.get_or_insert(basefee);
        });
        ccx.update_block(|block| block.basefee = U256::ZERO);
    }
}

fn inner_revert_to_state<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    snapshot_id: U256,
) -> Result {
    let (evm, db) = ccx.evm_db_mut();
    if let Some(replaced) = db.revert_state(
        snapshot_id,
        evm,
        RevertStateSnapshotAction::RevertKeep,
        ExecutionPhase::InFlight,
    ) {
        ccx.state.restore_persistent_writes(snapshot_id, replaced, false);
        // `RevertKeep` keeps the backend snapshot alive for further
        // reverts, so keep our matching env-overrides copy too.
        ccx.state.env_overrides.restore_snapshot(snapshot_id, false);
        if let Some(&fork_block_number) =
            ccx.state.fork_block_number_override_snapshots.get(&snapshot_id)
        {
            ccx.state.fork_block_number_override = fork_block_number;
        }
        ccx.state.revert_created_accounts(snapshot_id, false);
        restore_isolation_fee_accounting(ccx);
        Ok(true.abi_encode())
    } else {
        Ok(false.abi_encode())
    }
}

fn inner_revert_to_state_and_delete<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    snapshot_id: U256,
) -> Result {
    let (evm, db) = ccx.evm_db_mut();
    if let Some(replaced) = db.revert_state(
        snapshot_id,
        evm,
        RevertStateSnapshotAction::RevertRemove,
        ExecutionPhase::InFlight,
    ) {
        ccx.state.restore_persistent_writes(snapshot_id, replaced, true);
        ccx.state.env_overrides.restore_snapshot(snapshot_id, true);
        if let Some(fork_block_number) =
            ccx.state.fork_block_number_override_snapshots.remove(&snapshot_id)
        {
            ccx.state.fork_block_number_override = fork_block_number;
        }
        ccx.state.revert_created_accounts(snapshot_id, true);
        restore_isolation_fee_accounting(ccx);
        Ok(true.abi_encode())
    } else {
        Ok(false.abi_encode())
    }
}

fn inner_value_snapshot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    group: Option<String>,
    name: Option<String>,
    value: String,
) -> Result {
    let (group, name) = derive_snapshot_name(ccx, group, name);

    ccx.state.gas_snapshots.entry(group).or_default().insert(name, value);

    Ok(Default::default())
}

fn inner_last_gas_snapshot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    group: Option<String>,
    name: Option<String>,
    value: u64,
) -> Result {
    let (group, name) = derive_snapshot_name(ccx, group, name);

    ccx.state.gas_snapshots.entry(group).or_default().insert(name, value.to_string());

    Ok(value.abi_encode())
}

fn inner_start_gas_snapshot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    group: Option<String>,
    name: Option<String>,
) -> Result {
    // Revert if there is an active gas snapshot as we can only have one active snapshot at a time.
    if let Some((group, name)) = &ccx.state.gas_metering.active_gas_snapshot {
        bail!("gas snapshot was already started with group: {group} and name: {name}");
    }

    let (group, name) = derive_snapshot_name(ccx, group, name);

    ccx.state.gas_metering.gas_records.push(GasRecord {
        group: group.clone(),
        name: name.clone(),
        gas_used: 0,
        depth: ccx.depth(),
    });

    ccx.state.gas_metering.active_gas_snapshot = Some((group, name));

    ccx.state.gas_metering.start();

    Ok(Default::default())
}

fn inner_stop_gas_snapshot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    group: Option<String>,
    name: Option<String>,
) -> Result {
    // If group and name are not provided, use the last snapshot group and name.
    let (group, name) = group
        .zip(name)
        .or_else(|| ccx.state.gas_metering.active_gas_snapshot.clone())
        .ok_or_else(|| fmt_err!("no active gas snapshot; call `startGasSnapshot` first"))?;

    if let Some(record) = ccx
        .state
        .gas_metering
        .gas_records
        .iter_mut()
        .find(|record| record.group == group && record.name == name)
    {
        // Calculate the gas used since the snapshot was started.
        // We subtract 171 from the gas used to account for gas used by the snapshot itself.
        let value = record.gas_used.saturating_sub(171);

        ccx.state
            .gas_snapshots
            .entry(group.clone())
            .or_default()
            .insert(name.clone(), value.to_string());

        // Stop the gas metering.
        ccx.state.gas_metering.stop();

        // Remove the gas record.
        ccx.state
            .gas_metering
            .gas_records
            .retain(|record| record.group != group && record.name != name);

        // Clear last snapshot cache if we have an exact match.
        if let Some((snapshot_group, snapshot_name)) = &ccx.state.gas_metering.active_gas_snapshot
            && snapshot_group == &group
            && snapshot_name == &name
        {
            ccx.state.gas_metering.active_gas_snapshot = None;
        }

        Ok(value.abi_encode())
    } else {
        bail!("no gas snapshot was started with the name: {name} in group: {group}");
    }
}

// Derives the snapshot group and name from the provided group and name or the running contract.
fn derive_snapshot_name<FEN: FoundryEvmNetwork>(
    ccx: &CheatsCtxt<'_, '_, FEN>,
    group: Option<String>,
    name: Option<String>,
) -> (String, String) {
    let group = group.unwrap_or_else(|| {
        ccx.state.core.config.running_artifact.clone().expect("expected running contract").name
    });
    let name = name.unwrap_or_else(|| "default".to_string());
    (group, name)
}

/// Reads the current caller information and returns the current [CallerMode], `msg.sender` and
/// `tx.origin`.
///
/// Depending on the current caller mode, one of the following results will be returned:
/// - If there is an active prank:
///     - caller_mode will be equal to:
///         - [CallerMode::Prank] if the prank has been set with `vm.prank(..)`.
///         - [CallerMode::RecurrentPrank] if the prank has been set with `vm.startPrank(..)`.
///     - `msg.sender` will be equal to the address set for the prank.
///     - `tx.origin` will be equal to the default sender address unless an alternative one has been
///       set when configuring the prank.
///
/// - If there is an active broadcast:
///     - caller_mode will be equal to:
///         - [CallerMode::Broadcast] if the broadcast has been set with `vm.broadcast(..)`.
///         - [CallerMode::RecurrentBroadcast] if the broadcast has been set with
///           `vm.startBroadcast(..)`.
///     - `msg.sender` and `tx.origin` will be equal to the address provided when setting the
///       broadcast.
///
/// - If no caller modification is active:
///     - caller_mode will be equal to [CallerMode::None],
///     - `msg.sender` and `tx.origin` will be equal to the default sender address.
fn read_callers<FEN: FoundryEvmNetwork>(
    state: &Cheatcodes<FEN>,
    default_sender: &Address,
    call_depth: usize,
) -> Result {
    let mut mode = CallerMode::None;
    let mut new_caller = default_sender;
    let mut new_origin = default_sender;
    if let Some(prank) = state.get_prank(call_depth) {
        mode = if prank.single_call { CallerMode::Prank } else { CallerMode::RecurrentPrank };
        new_caller = &prank.new_caller;
        if let Some(new) = &prank.new_origin {
            new_origin = new;
        }
    } else if let Some(broadcast) = &state.broadcast {
        mode = if broadcast.single_call {
            CallerMode::Broadcast
        } else {
            CallerMode::RecurrentBroadcast
        };
        new_caller = &broadcast.new_origin;
        new_origin = &broadcast.new_origin;
    }

    Ok((mode, new_caller, new_origin).abi_encode_params())
}

/// Ensures the account is loaded, warm and touched.
pub(super) fn touch_account(state: &mut State<'_>, addr: Address) -> Result<()> {
    let mut account = state.account(&addr)?;
    account.touch();
    account.warm();
    Ok(())
}

/// Loads the storage slot and marks it warm, as an `SLOAD` or `SSTORE` of it would.
fn warm_storage_slot<'s, 'db>(
    state: &'s mut State<'db>,
    address: &'s Address,
    key: U256,
) -> DbResult<StorageSlotHandle<'s, 'db>> {
    let mut slot = state.storage_slot(address, key)?;
    slot.warm();
    Ok(slot)
}

/// Writes `block_hash` to the EIP-2935 history storage, if the history contract is deployed.
///
/// Writes do not warm the history account or slot.
fn set_eip2935_blockhash(
    state: &mut State<'_>,
    block_number: U256,
    block_hash: B256,
) -> Result<()> {
    if state.account(&HISTORY_STORAGE_ADDRESS)?.code_hash() != keccak256(&HISTORY_STORAGE_CODE) {
        return Ok(());
    }
    state
        .storage_slot(&HISTORY_STORAGE_ADDRESS, history_storage_slot(block_number))
        .map_err(|e| fmt_err!("failed to store EIP-2935 history slot: {e}"))?
        .set(history_storage_value(block_hash));
    Ok(())
}

/// Installs the addresses a nested transaction in `env` with `access_list` warms when it starts
/// and that `vm.cool` keeps warm, returning the running transaction's.
fn enter_tx_warm_addresses<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    env: &EvmEnvFor<FEN>,
    access_list: Option<&alloy_rpc_types::AccessList>,
) -> Vec<Address> {
    std::mem::replace(
        &mut state.tx_warm_addresses,
        foundry_evm_core::utils::tx_warm_addresses(env, access_list),
    )
}

/// Clears the EIP-2935 history ring buffer with journaled writes, so that reverting the enclosing
/// frame restores the previous history storage.
///
/// Warmth is preserved: slots that were cold stay cold. A database error reverts any partial
/// clearing.
fn clear_eip2935_history_storage(state: &mut State<'_>, features: EvmFeatures) -> Result<()> {
    let checkpoint = state.checkpoint();
    for slot in 0..HISTORY_SERVE_WINDOW {
        match state.storage_slot(&HISTORY_STORAGE_ADDRESS, U256::from(slot)) {
            Ok(mut slot) => slot.set(U256::ZERO),
            Err(e) => {
                state.rollback(checkpoint, features);
                return Err(fmt_err!("failed to clear EIP-2935 history slot: {e}"));
            }
        }
    }
    Ok(())
}

// Tempo TIP-1026 stores logoURI in TIP-20 storage slot 5, reusing the
// previously-unused domainSeparator slot. This mirrors Tempo's
// crates/precompiles/tests/storage_tests/solidity/testdata/tip20.layout.json
// fixture, where `logoUri` has slot "5".
const TIP20_LOGO_URI_SLOT_INDEX: u64 = 5;

fn set_tip20_logo_uri<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    token: &Address,
    new_logo_uri: &str,
) -> Result {
    validate_tip20_logo_uri(new_logo_uri).map_err(|err| match err {
        Tip20LogoUriValidationError::LogoURITooLong => fmt_err!("LogoURITooLong"),
        Tip20LogoUriValidationError::InvalidLogoURI => fmt_err!("InvalidLogoURI"),
    })?;
    ccx.ensure_not_precompile(token)?;
    let state = ccx.evm.state_mut();
    touch_account(state, *token)?;
    store_solidity_string(
        state,
        *token,
        U256::from(TIP20_LOGO_URI_SLOT_INDEX),
        new_logo_uri.as_bytes(),
    )
}

fn store_solidity_string(
    state: &mut State<'_>,
    target: Address,
    base_slot: U256,
    bytes: &[u8],
) -> Result {
    cleanup_long_string_tail(state, target, base_slot, bytes.len())?;

    if bytes.len() <= 31 {
        warm_storage_slot(state, &target, base_slot)
            .map_err(|e| fmt_err!("failed to store TIP-20 logo URI: {e}"))?
            .set(encode_short_string(bytes));
        return Ok(Default::default());
    }

    warm_storage_slot(state, &target, base_slot)
        .map_err(|e| fmt_err!("failed to store TIP-20 logo URI length: {e}"))?
        .set(U256::from(bytes.len() * 2 + 1));

    let slot_start = solidity_dynamic_data_slot(base_slot);
    for (index, chunk) in bytes.chunks(32).enumerate() {
        let mut chunk_bytes = [0u8; 32];
        chunk_bytes[..chunk.len()].copy_from_slice(chunk);
        warm_storage_slot(state, &target, slot_start + U256::from(index))
            .map_err(|e| fmt_err!("failed to store TIP-20 logo URI data: {e}"))?
            .set(U256::from_be_bytes(chunk_bytes));
    }

    Ok(Default::default())
}

fn cleanup_long_string_tail(
    state: &mut State<'_>,
    target: Address,
    base_slot: U256,
    new_len: usize,
) -> Result<()> {
    let previous = warm_storage_slot(state, &target, base_slot)
        .map_err(|e| fmt_err!("failed to load previous TIP-20 logo URI: {e}"))?
        .current();
    if !is_long_string(previous) {
        return Ok(());
    }

    let Some(previous_len) = long_string_length(previous) else {
        return Ok(());
    };
    let previous_chunks = string_chunks(previous_len);
    let new_chunks = if new_len > 31 { string_chunks(new_len) } else { 0 };
    if previous_chunks <= new_chunks {
        return Ok(());
    }

    let slot_start = solidity_dynamic_data_slot(base_slot);
    for index in new_chunks..previous_chunks {
        warm_storage_slot(state, &target, slot_start + U256::from(index))
            .map_err(|e| fmt_err!("failed to clear previous TIP-20 logo URI data: {e}"))?
            .set(U256::ZERO);
    }

    Ok(())
}

fn encode_short_string(bytes: &[u8]) -> U256 {
    let mut storage_bytes = [0u8; 32];
    storage_bytes[..bytes.len()].copy_from_slice(bytes);
    storage_bytes[31] =
        u8::try_from(bytes.len() * 2).expect("short Solidity string length tag fits in u8");
    U256::from_be_bytes(storage_bytes)
}

fn solidity_dynamic_data_slot(base_slot: U256) -> U256 {
    Into::<U256>::into(keccak256(base_slot.to_be_bytes::<32>()))
}

const fn is_long_string(slot_value: U256) -> bool {
    (slot_value.to_be_bytes::<32>()[31] & 1) != 0
}

fn long_string_length(slot_value: U256) -> Option<usize> {
    let length: U256 = (slot_value - U256::ONE) >> 1;
    usize::try_from(length).ok().filter(|length| *length <= TIP20_MAX_LOGO_URI_BYTES)
}

const fn string_chunks(byte_length: usize) -> usize {
    byte_length.div_ceil(32)
}

/// Consumes recorded account accesses and returns them as an abi encoded
/// array of [AccountAccess]. If there are no accounts were
/// recorded as accessed, an abi encoded empty array is returned.
///
/// In the case where `stopAndReturnStateDiff` is called at a lower
/// depth than `startStateDiffRecording`, multiple `Vec<RecordedAccountAccesses>`
/// will be flattened, preserving the order of the accesses.
fn get_state_diff<FEN: FoundryEvmNetwork>(state: &mut Cheatcodes<FEN>) -> Result {
    let mut res = state.take_recorded_account_diffs_prefix();
    res.extend(
        state
            .recorded_account_diffs_stack
            .replace(Default::default())
            .unwrap_or_default()
            .into_iter()
            .flatten(),
    );
    Ok(res.abi_encode())
}

/// Marks a reverted frame's account accesses, including their storage accesses, as reverted.
pub(crate) fn mark_account_accesses_reverted(accesses: &mut [AccountAccess]) {
    for access in accesses {
        access.reverted = true;
        for storage_access in &mut access.storageAccesses {
            storage_access.reverted = true;
        }
    }
}

/// Moves a finished frame's account accesses into the enclosing recorded frame, or back onto the
/// stack when no enclosing frame was recorded, preserving the order of the accesses.
pub(crate) fn merge_recorded_frame(
    stack: &mut Vec<Vec<AccountAccess>>,
    mut accesses: Vec<AccountAccess>,
) {
    if let Some(parent) = stack.last_mut() {
        parent.append(&mut accesses);
    } else {
        stack.push(accesses);
    }
}

/// Appends an AccountAccess that resumes the recording of the current context.
pub(crate) fn append_storage_access(
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
                    oldNonce: 0,
                    newNonce: 0,
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

/// Creates a `GenesisAccount` from the account at `address` and its known storage.
fn genesis_account(
    state: &mut State<'_>,
    address: &Address,
    slots: impl IntoIterator<Item = U256>,
) -> Result<GenesisAccount> {
    let (nonce, balance, code) = {
        let mut account = state.account(address)?;
        (account.nonce(), account.balance(), account.load_code()?)
    };
    let mut storage = BTreeMap::new();
    for slot in slots {
        storage.insert(B256::from(slot), B256::from(state.storage_slot_untracked(address, &slot)?));
    }
    Ok(GenesisAccount {
        nonce: Some(nonce),
        balance,
        code: Some(code.original_bytes()),
        storage: Some(storage),
        ..Default::default()
    })
}

/// Helper function to returns state diffs recorded for each changed account.
fn get_recorded_state_diffs<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
) -> BTreeMap<Address, AccountStateDiffs> {
    let mut state_diffs: BTreeMap<Address, AccountStateDiffs> = BTreeMap::default();

    // First, collect all unique addresses we need to look up
    let mut addresses_to_lookup = AddressSet::default();
    // Storage owner to the chain and bytecode address that executed each write. For a delegatecall,
    // these are the proxy and implementation respectively.
    let mut layout_sources = AddressMap::default();
    for account_access in ccx.state.recorded_account_diffs() {
        if !account_access.storageAccesses.is_empty()
            || account_access.oldBalance != account_access.newBalance
        {
            addresses_to_lookup.insert(account_access.account);
            for storage_access in &account_access.storageAccesses {
                if storage_access.isWrite && !storage_access.reverted {
                    addresses_to_lookup.insert(storage_access.account);
                    let source = (
                        account_access.chainInfo.chainId.to::<u64>(),
                        account_access.chainInfo.forkId,
                        account_access.account,
                    );
                    if let Some(existing) = layout_sources.get_mut(&storage_access.account) {
                        if *existing != Some(source) {
                            // The state-diff output has no chain dimension. If writes to one
                            // address came from different chains or implementations, decoding it
                            // with either layout would be misleading.
                            *existing = None;
                        }
                    } else {
                        layout_sources.insert(storage_access.account, Some(source));
                    }
                }
            }
        }
    }

    // Look up contract names and storage layouts for all addresses
    let mut contract_names = AddressMap::default();
    let mut storage_layouts = AddressMap::default();
    for address in addresses_to_lookup {
        if let Some((artifact_id, contract_data)) = get_contract_data(ccx, address, false) {
            contract_names.insert(address, artifact_id.identifier());

            // Also get storage layout if available
            if let Some(storage_layout) = &contract_data.storage_layout {
                storage_layouts.insert(address, storage_layout.clone());
            }
        }
    }

    // A delegatecall writes the caller's storage with the callee's layout. Prefer a local artifact
    // for the recorded bytecode address; otherwise fetch that exact implementation on the chain
    // where the write occurred. This remains correct for historical forks and upgraded proxies.
    if ccx.state.core.config.decode_external_storage {
        let current_chain_id = ccx.chain_id();
        let current_fork_id = ccx.active_fork_id().unwrap_or_default();
        let mut external_requests = BTreeMap::<u64, AddressSet>::new();
        let mut external_targets = Vec::new();
        for (storage_address, source) in layout_sources {
            let Some((chain_id, fork_id, code_address)) = source else {
                // Writes from multiple chains or bytecode addresses cannot share one trustworthy
                // layout in the address-keyed output.
                contract_names.remove(&storage_address);
                storage_layouts.remove(&storage_address);
                continue;
            };
            let recorded_context_is_current =
                chain_id == current_chain_id && fork_id == current_fork_id;
            if !recorded_context_is_current {
                contract_names.remove(&storage_address);
            }
            if recorded_context_is_current
                && code_address == storage_address
                && storage_layouts.contains_key(&storage_address)
            {
                continue;
            }

            // A local artifact for the storage owner may describe a proxy rather than the code
            // that executed the write. Replace it only with the recorded bytecode's layout.
            storage_layouts.remove(&storage_address);
            if recorded_context_is_current
                && let Some((artifact_id, contract_data)) =
                    get_contract_data(ccx, code_address, false)
                && let Some(layout) = &contract_data.storage_layout
            {
                contract_names.insert(storage_address, artifact_id.identifier());
                storage_layouts.insert(storage_address, layout.clone());
            } else {
                external_requests.entry(chain_id).or_default().insert(code_address);
                external_targets.push((storage_address, chain_id, code_address));
            }
        }

        for (chain_id, addresses) in external_requests {
            let external = crate::external_storage::storage_layouts(
                &ccx.state.core.config.external_sources,
                foundry_config::Chain::from(chain_id),
                addresses.into_iter().collect(),
            );
            for &(storage_address, target_chain_id, code_address) in &external_targets {
                if target_chain_id == chain_id
                    && let Some((name, layout)) = external.get(&code_address)
                {
                    contract_names.insert(storage_address, name.clone());
                    storage_layouts.insert(storage_address, layout.clone());
                }
            }
        }
    }

    // Now process the records
    ccx.state
        .recorded_account_diffs()
        .filter(|account_access| {
            !account_access.storageAccesses.is_empty()
                || account_access.oldBalance != account_access.newBalance
                || (!account_access.reverted && account_access.oldNonce != account_access.newNonce)
        })
        .for_each(|account_access| {
            // Record account balance diffs.
            if account_access.oldBalance != account_access.newBalance {
                let account_diff = state_diffs.entry(account_access.account).or_insert_with(|| {
                    AccountStateDiffs {
                        label: ccx.state.core.labels.get(&account_access.account).cloned(),
                        contract: contract_names.get(&account_access.account).cloned(),
                        ..Default::default()
                    }
                });
                // Update balance diff. Do not overwrite the initial balance if already set.
                if let Some(diff) = &mut account_diff.balance_diff {
                    diff.new_value = account_access.newBalance;
                } else {
                    account_diff.balance_diff = Some(BalanceDiff {
                        previous_value: account_access.oldBalance,
                        new_value: account_access.newBalance,
                    });
                }
            }

            // Record account nonce diffs.
            if account_access.oldNonce != account_access.newNonce && !account_access.reverted {
                let account_diff = state_diffs.entry(account_access.account).or_insert_with(|| {
                    AccountStateDiffs {
                        label: ccx.state.core.labels.get(&account_access.account).cloned(),
                        contract: contract_names.get(&account_access.account).cloned(),
                        ..Default::default()
                    }
                });
                // Update nonce diff. Do not overwrite the initial nonce if already set.
                if let Some(diff) = &mut account_diff.nonce_diff {
                    diff.new_value = account_access.newNonce;
                } else {
                    account_diff.nonce_diff = Some(NonceDiff {
                        previous_value: account_access.oldNonce,
                        new_value: account_access.newNonce,
                    });
                }
            }

            // Collect all storage accesses for this account
            let raw_changes_by_slot = account_access
                .storageAccesses
                .iter()
                .filter_map(|access| {
                    (access.isWrite && !access.reverted)
                        .then_some((access.slot, (access.previousValue, access.newValue)))
                })
                .collect::<BTreeMap<_, _>>();

            // Record account state diffs.
            for storage_access in &account_access.storageAccesses {
                if storage_access.isWrite && !storage_access.reverted {
                    let account_diff =
                        state_diffs.entry(storage_access.account).or_insert_with(|| {
                            AccountStateDiffs {
                                label: ccx.state.core.labels.get(&storage_access.account).cloned(),
                                contract: contract_names.get(&storage_access.account).cloned(),
                                ..Default::default()
                            }
                        });
                    let layout = storage_layouts.get(&storage_access.account);
                    // Update state diff. Do not overwrite the initial value if already set.
                    let entry = match account_diff.state_diff.entry(storage_access.slot) {
                        Entry::Vacant(slot_state_diff) => {
                            // Get storage layout info for this slot
                            // Include mapping slots if available for the account
                            let mapping_slots = ccx
                                .state
                                .core
                                .mapping_slots
                                .as_ref()
                                .and_then(|slots| slots.get(&storage_access.account));

                            let slot_info = layout.and_then(|layout| {
                                let decoder = SlotIdentifier::new(layout.clone());
                                decoder.identify(&storage_access.slot, mapping_slots).or_else(
                                    || {
                                        // Create a map of new values for bytes/string
                                        // identification. These values are used to determine
                                        // the length of the data which helps determine how many
                                        // slots to search
                                        let current_base_slot_values = raw_changes_by_slot
                                            .iter()
                                            .map(|(slot, (_, new_val))| (*slot, *new_val))
                                            .collect::<B256Map<_>>();
                                        decoder.identify_bytes_or_string(
                                            &storage_access.slot,
                                            &current_base_slot_values,
                                        )
                                    },
                                )
                            });

                            slot_state_diff.insert(SlotStateDiff {
                                previous_value: storage_access.previousValue,
                                new_value: storage_access.newValue,
                                slot_info,
                            })
                        }
                        Entry::Occupied(slot_state_diff) => {
                            let entry = slot_state_diff.into_mut();
                            entry.new_value = storage_access.newValue;
                            entry
                        }
                    };

                    // Update decoded values if we have slot info
                    if let Some(slot_info) = &mut entry.slot_info {
                        slot_info.decode_values(entry.previous_value, storage_access.newValue);
                        if slot_info.is_bytes_or_string() {
                            slot_info.decode_bytes_or_string_values(
                                &storage_access.slot,
                                &raw_changes_by_slot,
                            );
                        }
                    }
                }
            }
        });
    state_diffs
}

/// EIP-1967 implementation storage slot
const EIP1967_IMPL_SLOT: &str = "360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";

/// EIP-1822 UUPS implementation storage slot: keccak256("PROXIABLE")
const EIP1822_PROXIABLE_SLOT: &str =
    "c5f16f0fcc639fa48a6947836d9850f504798523bf8c9a3a87d5876cf622bcf7";

/// Helper function to get the contract data from the deployed code at an address.
fn get_contract_data<'a, FEN: FoundryEvmNetwork>(
    ccx: &'a mut CheatsCtxt<'_, '_, FEN>,
    address: Address,
    detect_proxy: bool,
) -> Option<(&'a foundry_compilers::ArtifactId, &'a foundry_common::contracts::ContractData)> {
    // Check if we have available artifacts to match against
    let artifacts = ccx.state.core.config.available_artifacts.as_ref()?;

    // Try to load the account and get its code
    let mut account = ccx.evm.state_mut().account(&address).ok()?;
    account.warm();
    let code = account.load_code().ok()?;

    // Skip if code is empty
    if code.is_empty() {
        return None;
    }

    // Try to find the artifact by deployed code
    let code_bytes = code.original_bytes();
    if let Some(result) = artifacts.find_by_deployed_code_exact(&code_bytes) {
        return Some(result);
    }

    // Fall back to known proxy artifacts when exact matching is impossible.
    let hex_str = hex::encode(&code_bytes);
    let find_by_suffix =
        |suffix: &str| artifacts.iter().find(|(a, _)| a.identifier().ends_with(suffix));
    if detect_proxy
        && hex_str.contains(EIP1967_IMPL_SLOT)
        && let Some(result) = find_by_suffix(":TransparentUpgradeableProxy")
    {
        return Some(result);
    } else if detect_proxy
        && hex_str.contains(EIP1822_PROXIABLE_SLOT)
        && let Some(result) = find_by_suffix(":UUPSUpgradeable")
    {
        return Some(result);
    }

    // Fallback to fuzzy matching if exact match fails
    artifacts.find_by_deployed_code(&code_bytes)
}

/// Helper function to set / unset cold storage slot of the target address.
fn set_cold_slot<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    target: Address,
    slot: U256,
    cold: bool,
) {
    ccx.evm.state_mut().set_storage_warm(&target, slot, !cold);
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::artifacts::StorageLayout;
    use std::sync::Arc;

    #[test]
    fn state_diff_preserves_raw_values_with_partial_decoding() {
        let layout = serde_json::from_value::<StorageLayout>(serde_json::json!({
            "storage": [
                { "astId": 1, "contract": "C", "label": "data", "offset": 0, "slot": "0", "type": "t_struct" }
            ],
            "types": {
                "t_struct": {
                    "encoding": "inplace", "label": "struct C.Data", "numberOfBytes": "32",
                    "members": [
                        { "astId": 2, "contract": "C", "label": "amount", "offset": 0, "slot": "0", "type": "t_amount" },
                        { "astId": 3, "contract": "C", "label": "paused", "offset": 16, "slot": "0", "type": "t_bool" }
                    ]
                },
                "t_amount": { "encoding": "inplace", "label": "Amount", "numberOfBytes": "16" },
                "t_bool": { "encoding": "inplace", "label": "bool", "numberOfBytes": "1" }
            }
        }))
        .unwrap();
        let identifier = SlotIdentifier::new(Arc::new(layout));
        let mut slot_info = identifier.identify(&B256::ZERO, None).unwrap();
        let new_value = B256::from(U256::from(1));
        slot_info.decode_values(B256::ZERO, new_value);
        let diffs = AccountStateDiffs {
            state_diff: BTreeMap::from([(
                B256::ZERO,
                SlotStateDiff { previous_value: B256::ZERO, new_value, slot_info: Some(slot_info) },
            )]),
            ..Default::default()
        };

        assert_eq!(
            diffs.to_string(),
            "- state diff:\n\
@ 0x0000000000000000000000000000000000000000000000000000000000000000 (data, (bool,)): 0x0000000000000000000000000000000000000000000000000000000000000000 → 0x0000000000000000000000000000000000000000000000000000000000000001\n    paused (bool): false → false\n"
        );
    }
}
