//! Implementations of [`Evm`](spec::Group::Evm) cheatcodes.

use crate::{
    BroadcastableTransaction, Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Error, Result,
    Vm::*,
    inspector::{Ecx, RecordDebugStepInfo},
};
use alloy_consensus::TxEnvelope;
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_network::eip2718::EIP4844_TX_TYPE_ID;
use alloy_primitives::{
    Address, B256, U256, hex, keccak256,
    map::{B256Map, HashMap},
};
use alloy_rlp::Decodable;
use alloy_sol_types::SolValue;
use foundry_common::{
    fs::{read_json_file, write_json_file},
    slot_identifier::{
        ENCODING_BYTES, ENCODING_DYN_ARRAY, ENCODING_INPLACE, ENCODING_MAPPING, SlotIdentifier,
        SlotInfo,
    },
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_evm_core::{
    ContextExt,
    backend::{DatabaseExt, RevertStateSnapshotAction},
    constants::{CALLER, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, TEST_CONTRACT_ADDRESS},
    utils::get_blob_base_fee_update_fraction_by_spec_id,
};
use foundry_evm_traces::TraceMode;
use itertools::Itertools;
use rand::Rng;
use revm::{
    bytecode::Bytecode,
    context::{Block, JournalTr},
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
    state::Account,
};
use std::{
    collections::{BTreeMap, HashSet, btree_map::Entry},
    fmt::Display,
    path::Path,
    str::FromStr,
};

mod record_debug_step;
use foundry_common::fmt::format_token_raw;
use foundry_config::evm_spec_id;
use record_debug_step::{convert_call_trace_ctx_to_debug_step, flatten_call_trace};
use serde::Serialize;

mod fork;
pub(crate) mod mapping;
pub(crate) mod mock;
pub(crate) mod prank;

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

/// Records `deal` cheatcodes
#[derive(Clone, Debug)]
pub struct DealRecord {
    /// Target of the deal.
    pub address: Address,
    /// The balance of the address before deal was applied
    pub old_balance: U256,
    /// Balance after deal was applied
    pub new_balance: U256,
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
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey } = self;
        let wallet = super::crypto::parse_wallet(privateKey)?;
        Ok(wallet.address().abi_encode())
    }
}

impl Cheatcode for getNonce_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        get_nonce(ccx, account)
    }
}

impl Cheatcode for getNonce_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { wallet } = self;
        get_nonce(ccx, &wallet.addr)
    }
}

impl Cheatcode for loadCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, slot } = *self;
        ccx.ensure_not_precompile(&target)?;

        let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
        journal.load_account(db, target)?;
        let mut val = journal
            .sload(db, target, slot.into(), false)
            .map_err(|e| fmt_err!("failed to load storage slot: {:?}", e))?;

        if val.is_cold && val.data.is_zero() {
            if ccx.state.has_arbitrary_storage(&target) {
                // If storage slot is untouched and load from a target with arbitrary storage,
                // then set random value for current slot.
                let rand_value = ccx.state.rng().random();
                ccx.state.arbitrary_storage.as_mut().unwrap().save(
                    ccx.ecx,
                    target,
                    slot.into(),
                    rand_value,
                );
                val.data = rand_value;
            } else if ccx.state.is_arbitrary_storage_copy(&target) {
                // If storage slot is untouched and load from a target that copies storage from
                // a source address with arbitrary storage, then copy existing arbitrary value.
                // If no arbitrary value generated yet, then the random one is saved and set.
                let rand_value = ccx.state.rng().random();
                val.data = ccx.state.arbitrary_storage.as_mut().unwrap().copy(
                    ccx.ecx,
                    target,
                    slot.into(),
                    rand_value,
                );
            }
        }

        Ok(val.abi_encode())
    }
}

impl Cheatcode for loadAllocsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
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
        let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
        db.load_allocs(&allocs, journal)
            .map(|()| Vec::default())
            .map_err(|e| fmt_err!("failed to load allocs: {e}"))
    }
}

impl Cheatcode for cloneAccountCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { source, target } = self;

        let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
        let account = journal.load_account(db, *source)?;
        let genesis = &genesis_account(account.data);
        db.clone_account(genesis, target, journal)?;
        // Cloned account should persist in forked envs.
        ccx.ecx.journaled_state.database.add_persistent_account(*target);
        Ok(Default::default())
    }
}

impl Cheatcode for dumpStateCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { pathToStateJson } = self;
        let path = Path::new(pathToStateJson);

        // Do not include system account or empty accounts in the dump.
        let skip = |key: &Address, val: &Account| {
            key == &CHEATCODE_ADDRESS
                || key == &CALLER
                || key == &HARDHAT_CONSOLE_ADDRESS
                || key == &TEST_CONTRACT_ADDRESS
                || key == &ccx.caller
                || key == &ccx.state.config.evm_opts.sender
                || val.is_empty()
        };

        let alloc = ccx
            .ecx
            .journaled_state
            .state()
            .iter_mut()
            .filter(|(key, val)| !skip(key, val))
            .map(|(key, val)| (key, genesis_account(val)))
            .collect::<BTreeMap<_, _>>();

        write_json_file(path, &alloc)?;
        Ok(Default::default())
    }
}

impl Cheatcode for recordCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.recording_accesses = true;
        state.accesses.clear();
        Ok(Default::default())
    }
}

impl Cheatcode for stopRecordCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        state.recording_accesses = false;
        Ok(Default::default())
    }
}

impl Cheatcode for accessesCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { target } = *self;
        let result = (
            state.accesses.reads.entry(target).or_default().as_slice(),
            state.accesses.writes.entry(target).or_default().as_slice(),
        );
        Ok(result.abi_encode_params())
    }
}

impl Cheatcode for recordLogsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.recorded_logs = Some(Default::default());
        Ok(Default::default())
    }
}

impl Cheatcode for getRecordedLogsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        Ok(state.recorded_logs.replace(Default::default()).unwrap_or_default().abi_encode())
    }
}

impl Cheatcode for pauseGasMeteringCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.gas_metering.paused = true;
        Ok(Default::default())
    }
}

impl Cheatcode for resumeGasMeteringCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.gas_metering.resume();
        Ok(Default::default())
    }
}

impl Cheatcode for resetGasMeteringCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.gas_metering.reset();
        Ok(Default::default())
    }
}

impl Cheatcode for lastCallGasCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        let Some(last_call_gas) = &state.gas_metering.last_call_gas else {
            bail!("no external call was made yet");
        };
        Ok(last_call_gas.abi_encode())
    }
}

impl Cheatcode for getChainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        Ok(U256::from(ccx.ecx.cfg.chain_id).abi_encode())
    }
}

impl Cheatcode for chainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newChainId } = self;
        ensure!(*newChainId <= U256::from(u64::MAX), "chain ID must be less than 2^64");
        ccx.ecx.cfg.chain_id = newChainId.to();
        Ok(Default::default())
    }
}

impl Cheatcode for coinbaseCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newCoinbase } = self;
        ccx.ecx.block.beneficiary = *newCoinbase;
        Ok(Default::default())
    }
}

impl Cheatcode for difficultyCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newDifficulty } = self;
        ensure!(
            ccx.ecx.cfg.spec < SpecId::MERGE,
            "`difficulty` is not supported after the Paris hard fork, use `prevrandao` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.block.difficulty = *newDifficulty;
        Ok(Default::default())
    }
}

impl Cheatcode for feeCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newBasefee } = self;
        ensure!(*newBasefee <= U256::from(u64::MAX), "base fee must be less than 2^64");
        ccx.ecx.block.basefee = newBasefee.saturating_to();
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.block.prevrandao = Some(*newPrevrandao);
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.block.prevrandao = Some((*newPrevrandao).into());
        Ok(Default::default())
    }
}

impl Cheatcode for blobhashesCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { hashes } = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::CANCUN,
            "`blobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        ccx.ecx.tx.blob_hashes.clone_from(hashes);
        // force this as 4844 txtype
        ccx.ecx.tx.tx_type = EIP4844_TX_TYPE_ID;
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobhashesCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::CANCUN,
            "`getBlobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        Ok(ccx.ecx.tx.blob_hashes.clone().abi_encode())
    }
}

impl Cheatcode for rollCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newHeight } = self;
        ccx.ecx.block.number = *newHeight;
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockNumberCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.block.number.abi_encode())
    }
}

impl Cheatcode for txGasPriceCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newGasPrice } = self;
        ensure!(*newGasPrice <= U256::from(u64::MAX), "gas price must be less than 2^64");
        ccx.ecx.tx.gas_price = newGasPrice.saturating_to();
        Ok(Default::default())
    }
}

impl Cheatcode for warpCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newTimestamp } = self;
        ccx.ecx.block.timestamp = *newTimestamp;
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockTimestampCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.block.timestamp.abi_encode())
    }
}

impl Cheatcode for blobBaseFeeCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newBlobBaseFee } = self;
        ensure!(
            ccx.ecx.cfg.spec >= SpecId::CANCUN,
            "`blobBaseFee` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );

        ccx.ecx.block.set_blob_excess_gas_and_price(
            (*newBlobBaseFee).to(),
            get_blob_base_fee_update_fraction_by_spec_id(ccx.ecx.cfg.spec),
        );
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobBaseFeeCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.block.blob_excess_gas().unwrap_or(0).abi_encode())
    }
}

impl Cheatcode for dealCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account: address, newBalance: new_balance } = *self;
        let account = journaled_account(ccx.ecx, address)?;
        let old_balance = std::mem::replace(&mut account.info.balance, new_balance);
        let record = DealRecord { address, old_balance, new_balance };
        ccx.state.eth_deals.push(record);
        Ok(Default::default())
    }
}

impl Cheatcode for etchCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, newRuntimeBytecode } = self;
        ccx.ensure_not_precompile(target)?;
        let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
        journal.load_account(db, *target)?;
        let bytecode = Bytecode::new_raw_checked(newRuntimeBytecode.clone())
            .map_err(|e| fmt_err!("failed to create bytecode: {e}"))?;
        journal.set_code(*target, bytecode);
        Ok(Default::default())
    }
}

impl Cheatcode for resetNonceCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account } = self;
        let account = journaled_account(ccx.ecx, *account)?;
        // Per EIP-161, EOA nonces start at 0, but contract nonces
        // start at 1. Comparing by code_hash instead of code
        // to avoid hitting the case where account's code is None.
        let empty = account.info.code_hash == KECCAK_EMPTY;
        let nonce = if empty { 0 } else { 1 };
        account.info.nonce = nonce;
        debug!(target: "cheatcodes", nonce, "reset");
        Ok(Default::default())
    }
}

impl Cheatcode for setNonceCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account, newNonce } = *self;
        let account = journaled_account(ccx.ecx, account)?;
        // nonce must increment only
        let current = account.info.nonce;
        ensure!(
            newNonce >= current,
            "new nonce ({newNonce}) must be strictly equal to or higher than the \
             account's current nonce ({current})"
        );
        account.info.nonce = newNonce;
        Ok(Default::default())
    }
}

impl Cheatcode for setNonceUnsafeCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { account, newNonce } = *self;
        let account = journaled_account(ccx.ecx, account)?;
        account.info.nonce = newNonce;
        Ok(Default::default())
    }
}

impl Cheatcode for storeCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, slot, value } = *self;
        ccx.ensure_not_precompile(&target)?;
        ensure_loaded_account(ccx.ecx, target)?;
        let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
        journal
            .sstore(db, target, slot.into(), value.into(), false)
            .map_err(|e| fmt_err!("failed to store storage slot: {:?}", e))?;
        Ok(Default::default())
    }
}

impl Cheatcode for coolCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target } = self;
        if let Some(account) = ccx.ecx.journaled_state.state.get_mut(target) {
            account.unmark_touch();
            account.storage.values_mut().for_each(|slot| slot.mark_cold());
        }
        Ok(Default::default())
    }
}

impl Cheatcode for accessListCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
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
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        // Set to empty option in order to override previous applied access list.
        if state.access_list.is_some() {
            state.access_list = Some(alloy_rpc_types::AccessList::default());
        }
        Ok(Default::default())
    }
}

impl Cheatcode for warmSlotCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, slot } = *self;
        set_cold_slot(ccx, target, slot.into(), false);
        Ok(Default::default())
    }
}

impl Cheatcode for coolSlotCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, slot } = *self;
        set_cold_slot(ccx, target, slot.into(), true);
        Ok(Default::default())
    }
}

impl Cheatcode for readCallersCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        read_callers(ccx.state, &ccx.ecx.tx.caller, ccx.ecx.journaled_state.depth())
    }
}

impl Cheatcode for snapshotValue_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { name, value } = self;
        inner_value_snapshot(ccx, None, Some(name.clone()), value.to_string())
    }
}

impl Cheatcode for snapshotValue_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { group, name, value } = self;
        inner_value_snapshot(ccx, Some(group.clone()), Some(name.clone()), value.to_string())
    }
}

impl Cheatcode for snapshotGasLastCall_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { name } = self;
        let Some(last_call_gas) = &ccx.state.gas_metering.last_call_gas else {
            bail!("no external call was made yet");
        };
        inner_last_gas_snapshot(ccx, None, Some(name.clone()), last_call_gas.gasTotalUsed)
    }
}

impl Cheatcode for snapshotGasLastCall_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { name, group } = self;
        let Some(last_call_gas) = &ccx.state.gas_metering.last_call_gas else {
            bail!("no external call was made yet");
        };
        inner_last_gas_snapshot(
            ccx,
            Some(group.clone()),
            Some(name.clone()),
            last_call_gas.gasTotalUsed,
        )
    }
}

impl Cheatcode for startSnapshotGas_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { name } = self;
        inner_start_gas_snapshot(ccx, None, Some(name.clone()))
    }
}

impl Cheatcode for startSnapshotGas_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { group, name } = self;
        inner_start_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()))
    }
}

impl Cheatcode for stopSnapshotGas_0Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        inner_stop_gas_snapshot(ccx, None, None)
    }
}

impl Cheatcode for stopSnapshotGas_1Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { name } = self;
        inner_stop_gas_snapshot(ccx, None, Some(name.clone()))
    }
}

impl Cheatcode for stopSnapshotGas_2Call {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { group, name } = self;
        inner_stop_gas_snapshot(ccx, Some(group.clone()), Some(name.clone()))
    }
}

// Deprecated in favor of `snapshotStateCall`
impl Cheatcode for snapshotCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

impl Cheatcode for snapshotStateCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

// Deprecated in favor of `revertToStateCall`
impl Cheatcode for revertToCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

// Deprecated in favor of `revertToStateAndDeleteCall`
impl Cheatcode for revertToAndDeleteCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateAndDeleteCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

// Deprecated in favor of `deleteStateSnapshotCall`
impl Cheatcode for deleteSnapshotCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_delete_state_snapshot(ccx, *snapshotId)
    }
}

impl Cheatcode for deleteStateSnapshotCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { snapshotId } = self;
        inner_delete_state_snapshot(ccx, *snapshotId)
    }
}

// Deprecated in favor of `deleteStateSnapshotsCall`
impl Cheatcode for deleteSnapshotsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        inner_delete_state_snapshots(ccx)
    }
}

impl Cheatcode for deleteStateSnapshotsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self {} = self;
        inner_delete_state_snapshots(ccx)
    }
}

impl Cheatcode for startStateDiffRecordingCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.recorded_account_diffs_stack = Some(Default::default());
        // Enable mapping recording to track mapping slot accesses
        state.mapping_slots.get_or_insert_default();
        Ok(Default::default())
    }
}

impl Cheatcode for stopAndReturnStateDiffCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        get_state_diff(state)
    }
}

impl Cheatcode for getStateDiffCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
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
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let state_diffs = get_recorded_state_diffs(ccx);
        Ok(serde_json::to_string(&state_diffs)?.abi_encode())
    }
}

impl Cheatcode for getStorageSlotsCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { target, variableName } = self;

        let storage_layout = get_contract_data(ccx, *target)
            .and_then(|(_, data)| data.storage_layout.as_ref().map(|layout| layout.clone()))
            .ok_or_else(|| fmt_err!("Storage layout not available for contract at {target}. Try compiling contracts with `--extra-output storageLayout`"))?;

        trace!(storage = ?storage_layout.storage, "fetched storage");

        let storage = storage_layout
            .storage
            .iter()
            .find(|s| s.label.to_lowercase() == *variableName.to_lowercase())
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

            // Start from 1 since base slot is already added
            for i in 1..num_slots.to::<usize>() {
                slots.push(slot + U256::from(i));
            }
        }

        if storage_type.encoding == ENCODING_BYTES {
            // Try to check if it's a long bytes/string by reading the current storage
            // value
            let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
            if let Ok(value) = journal.sload(db, *target, slot, false) {
                let value_bytes = value.data.to_be_bytes::<32>();
                let length_byte = value_bytes[31];
                // Check if it's a long bytes/string (LSB is 1)
                if length_byte & 1 == 1 {
                    // Calculate data slots for long bytes/string
                    let length: U256 = value.data >> 1;
                    let num_data_slots = length.to::<usize>().div_ceil(32);
                    let data_start = U256::from_be_bytes(keccak256(B256::from(slot).0).0);

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
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let mut storage_accesses = Vec::new();

        if let Some(recorded_diffs) = &state.recorded_account_diffs_stack {
            for account_accesses in recorded_diffs.iter().flatten() {
                storage_accesses.extend(account_accesses.storageAccesses.clone());
            }
        }

        Ok(storage_accesses.abi_encode())
    }
}

impl Cheatcode for broadcastRawTransactionCall {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let tx = TxEnvelope::decode(&mut self.data.as_ref())
            .map_err(|err| fmt_err!("failed to decode RLP-encoded transaction: {err}"))?;

        let (db, journal, env) = ccx.ecx.as_db_env_and_journal();
        db.transact_from_tx(
            &tx.clone().into(),
            env.to_owned(),
            journal,
            &mut *executor.get_inspector(ccx.state),
        )?;

        if ccx.state.broadcast.is_some() {
            ccx.state.broadcastable_transactions.push_back(BroadcastableTransaction {
                rpc: ccx.ecx.journaled_state.database.active_fork_url(),
                transaction: tx.try_into()?,
            });
        }

        Ok(Default::default())
    }
}

impl Cheatcode for setBlockhashCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { blockNumber, blockHash } = *self;
        ensure!(blockNumber <= U256::from(u64::MAX), "blockNumber must be less than 2^64");
        ensure!(
            blockNumber <= U256::from(ccx.ecx.block.number),
            "block number must be less than or equal to the current block number"
        );

        ccx.ecx.journaled_state.database.set_blockhash(blockNumber, blockHash);

        Ok(Default::default())
    }
}

impl Cheatcode for startDebugTraceRecordingCall {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let Some(tracer) = executor.tracing_inspector() else {
            return Err(Error::from("no tracer initiated, consider adding -vvv flag"));
        };

        let mut info = RecordDebugStepInfo {
            // will be updated later
            start_node_idx: 0,
            // keep the original config to revert back later
            original_tracer_config: *tracer.config(),
        };

        // turn on tracer debug configuration for recording
        *tracer.config_mut() = TraceMode::Debug.into_config().expect("cannot be None");

        // track where the recording starts
        if let Some(last_node) = tracer.traces().nodes().last() {
            info.start_node_idx = last_node.idx;
        }

        ccx.state.record_debug_steps_info = Some(info);
        Ok(Default::default())
    }
}

impl Cheatcode for stopAndReturnDebugTraceRecordingCall {
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
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
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { evm } = self;
        let spec_id = evm_spec_id(
            EvmVersion::from_str(evm)
                .map_err(|_| Error::from(format!("invalid evm version {evm}")))?,
        );
        ccx.state.execution_evm_version = Some(spec_id);
        Ok(Default::default())
    }
}

impl Cheatcode for getEvmVersionCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        Ok(ccx.ecx.cfg.spec.to_string().to_lowercase().abi_encode())
    }
}

pub(super) fn get_nonce(ccx: &mut CheatsCtxt, address: &Address) -> Result {
    let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
    let account = journal.load_account(db, *address)?;
    Ok(account.info.nonce.abi_encode())
}

fn inner_snapshot_state(ccx: &mut CheatsCtxt) -> Result {
    let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
    Ok(db.snapshot_state(journal, &mut env).abi_encode())
}

fn inner_revert_to_state(ccx: &mut CheatsCtxt, snapshot_id: U256) -> Result {
    let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();
    let result = if let Some(journaled_state) =
        db.revert_state(snapshot_id, &*journal, &mut env, RevertStateSnapshotAction::RevertKeep)
    {
        // we reset the evm's journaled_state to the state of the snapshot previous state
        ccx.ecx.journaled_state.inner = journaled_state;
        true
    } else {
        false
    };
    Ok(result.abi_encode())
}

fn inner_revert_to_state_and_delete(ccx: &mut CheatsCtxt, snapshot_id: U256) -> Result {
    let (db, journal, mut env) = ccx.ecx.as_db_env_and_journal();

    let result = if let Some(journaled_state) =
        db.revert_state(snapshot_id, &*journal, &mut env, RevertStateSnapshotAction::RevertRemove)
    {
        // we reset the evm's journaled_state to the state of the snapshot previous state
        ccx.ecx.journaled_state.inner = journaled_state;
        true
    } else {
        false
    };
    Ok(result.abi_encode())
}

fn inner_delete_state_snapshot(ccx: &mut CheatsCtxt, snapshot_id: U256) -> Result {
    let result = ccx.ecx.journaled_state.database.delete_state_snapshot(snapshot_id);
    Ok(result.abi_encode())
}

fn inner_delete_state_snapshots(ccx: &mut CheatsCtxt) -> Result {
    ccx.ecx.journaled_state.database.delete_state_snapshots();
    Ok(Default::default())
}

fn inner_value_snapshot(
    ccx: &mut CheatsCtxt,
    group: Option<String>,
    name: Option<String>,
    value: String,
) -> Result {
    let (group, name) = derive_snapshot_name(ccx, group, name);

    ccx.state.gas_snapshots.entry(group).or_default().insert(name, value);

    Ok(Default::default())
}

fn inner_last_gas_snapshot(
    ccx: &mut CheatsCtxt,
    group: Option<String>,
    name: Option<String>,
    value: u64,
) -> Result {
    let (group, name) = derive_snapshot_name(ccx, group, name);

    ccx.state.gas_snapshots.entry(group).or_default().insert(name, value.to_string());

    Ok(value.abi_encode())
}

fn inner_start_gas_snapshot(
    ccx: &mut CheatsCtxt,
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
        depth: ccx.ecx.journaled_state.depth(),
    });

    ccx.state.gas_metering.active_gas_snapshot = Some((group, name));

    ccx.state.gas_metering.start();

    Ok(Default::default())
}

fn inner_stop_gas_snapshot(
    ccx: &mut CheatsCtxt,
    group: Option<String>,
    name: Option<String>,
) -> Result {
    // If group and name are not provided, use the last snapshot group and name.
    let (group, name) = group
        .zip(name)
        .unwrap_or_else(|| ccx.state.gas_metering.active_gas_snapshot.clone().unwrap());

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
fn derive_snapshot_name(
    ccx: &CheatsCtxt,
    group: Option<String>,
    name: Option<String>,
) -> (String, String) {
    let group = group.unwrap_or_else(|| {
        ccx.state.config.running_artifact.clone().expect("expected running contract").name
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
fn read_callers(state: &Cheatcodes, default_sender: &Address, call_depth: usize) -> Result {
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

/// Ensures the `Account` is loaded and touched.
pub(super) fn journaled_account<'a>(
    ecx: Ecx<'a, '_, '_>,
    addr: Address,
) -> Result<&'a mut Account> {
    ensure_loaded_account(ecx, addr)?;
    Ok(ecx.journaled_state.state.get_mut(&addr).expect("account is loaded"))
}

pub(super) fn ensure_loaded_account(ecx: Ecx, addr: Address) -> Result<()> {
    let (db, journal, _) = ecx.as_db_env_and_journal();
    journal.load_account(db, addr)?;
    journal.touch(addr);
    Ok(())
}

/// Consumes recorded account accesses and returns them as an abi encoded
/// array of [AccountAccess]. If there are no accounts were
/// recorded as accessed, an abi encoded empty array is returned.
///
/// In the case where `stopAndReturnStateDiff` is called at a lower
/// depth than `startStateDiffRecording`, multiple `Vec<RecordedAccountAccesses>`
/// will be flattened, preserving the order of the accesses.
fn get_state_diff(state: &mut Cheatcodes) -> Result {
    let res = state
        .recorded_account_diffs_stack
        .replace(Default::default())
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(res.abi_encode())
}

/// Helper function that creates a `GenesisAccount` from a regular `Account`.
fn genesis_account(account: &Account) -> GenesisAccount {
    GenesisAccount {
        nonce: Some(account.info.nonce),
        balance: account.info.balance,
        code: account.info.code.as_ref().map(|o| o.original_bytes()),
        storage: Some(
            account
                .storage
                .iter()
                .map(|(k, v)| (B256::from(*k), B256::from(v.present_value())))
                .collect(),
        ),
        private_key: None,
    }
}

/// Helper function to returns state diffs recorded for each changed account.
fn get_recorded_state_diffs(ccx: &mut CheatsCtxt) -> BTreeMap<Address, AccountStateDiffs> {
    let mut state_diffs: BTreeMap<Address, AccountStateDiffs> = BTreeMap::default();

    // First, collect all unique addresses we need to look up
    let mut addresses_to_lookup = HashSet::new();
    if let Some(records) = &ccx.state.recorded_account_diffs_stack {
        for account_access in records.iter().flatten() {
            if !account_access.storageAccesses.is_empty()
                || account_access.oldBalance != account_access.newBalance
            {
                addresses_to_lookup.insert(account_access.account);
                for storage_access in &account_access.storageAccesses {
                    if storage_access.isWrite && !storage_access.reverted {
                        addresses_to_lookup.insert(storage_access.account);
                    }
                }
            }
        }
    }

    // Look up contract names and storage layouts for all addresses
    let mut contract_names = HashMap::new();
    let mut storage_layouts = HashMap::new();
    for address in addresses_to_lookup {
        if let Some((artifact_id, contract_data)) = get_contract_data(ccx, address) {
            contract_names.insert(address, artifact_id.identifier());

            // Also get storage layout if available
            if let Some(storage_layout) = &contract_data.storage_layout {
                storage_layouts.insert(address, storage_layout.clone());
            }
        }
    }

    // Now process the records
    if let Some(records) = &ccx.state.recorded_account_diffs_stack {
        records
            .iter()
            .flatten()
            .filter(|account_access| {
                !account_access.storageAccesses.is_empty()
                    || account_access.oldBalance != account_access.newBalance
                    || account_access.oldNonce != account_access.newNonce
            })
            .for_each(|account_access| {
                // Record account balance diffs.
                if account_access.oldBalance != account_access.newBalance {
                    let account_diff =
                        state_diffs.entry(account_access.account).or_insert_with(|| {
                            AccountStateDiffs {
                                label: ccx.state.labels.get(&account_access.account).cloned(),
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
                if account_access.oldNonce != account_access.newNonce {
                    let account_diff =
                        state_diffs.entry(account_access.account).or_insert_with(|| {
                            AccountStateDiffs {
                                label: ccx.state.labels.get(&account_access.account).cloned(),
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
                        let account_diff = state_diffs
                            .entry(storage_access.account)
                            .or_insert_with(|| AccountStateDiffs {
                                label: ccx.state.labels.get(&storage_access.account).cloned(),
                                contract: contract_names.get(&storage_access.account).cloned(),
                                ..Default::default()
                            });
                        let layout = storage_layouts.get(&storage_access.account);
                        // Update state diff. Do not overwrite the initial value if already set.
                        let entry = match account_diff.state_diff.entry(storage_access.slot) {
                            Entry::Vacant(slot_state_diff) => {
                                // Get storage layout info for this slot
                                // Include mapping slots if available for the account
                                let mapping_slots = ccx
                                    .state
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
    }
    state_diffs
}

/// EIP-1967 implementation storage slot
const EIP1967_IMPL_SLOT: &str = "360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";

/// EIP-1822 UUPS implementation storage slot: keccak256("PROXIABLE")
const EIP1822_PROXIABLE_SLOT: &str =
    "c5f16f0fcc639fa48a6947836d9850f504798523bf8c9a3a87d5876cf622bcf7";

/// Helper function to get the contract data from the deployed code at an address.
fn get_contract_data<'a>(
    ccx: &'a mut CheatsCtxt,
    address: Address,
) -> Option<(&'a foundry_compilers::ArtifactId, &'a foundry_common::contracts::ContractData)> {
    // Check if we have available artifacts to match against
    let artifacts = ccx.state.config.available_artifacts.as_ref()?;

    // Try to load the account and get its code
    let (db, journal, _) = ccx.ecx.as_db_env_and_journal();
    let account = journal.load_account(db, address).ok()?;
    let code = account.info.code.as_ref()?;

    // Skip if code is empty
    if code.is_empty() {
        return None;
    }

    // Try to find the artifact by deployed code
    let code_bytes = code.original_bytes();
    // First check for proxy patterns
    let hex_str = hex::encode(&code_bytes);
    let find_by_suffix =
        |suffix: &str| artifacts.iter().find(|(a, _)| a.identifier().ends_with(suffix));
    // Simple proxy detection based on storage slot patterns
    if hex_str.contains(EIP1967_IMPL_SLOT)
        && let Some(result) = find_by_suffix(":TransparentUpgradeableProxy")
    {
        return Some(result);
    } else if hex_str.contains(EIP1822_PROXIABLE_SLOT)
        && let Some(result) = find_by_suffix(":UUPSUpgradeable")
    {
        return Some(result);
    }

    // Try exact match
    if let Some(result) = artifacts.find_by_deployed_code_exact(&code_bytes) {
        return Some(result);
    }

    // Fallback to fuzzy matching if exact match fails
    artifacts.find_by_deployed_code(&code_bytes)
}

/// Helper function to set / unset cold storage slot of the target address.
fn set_cold_slot(ccx: &mut CheatsCtxt, target: Address, slot: U256, cold: bool) {
    if let Some(account) = ccx.ecx.journaled_state.state.get_mut(&target)
        && let Some(storage_slot) = account.storage.get_mut(&slot)
    {
        storage_slot.is_cold = cold;
    }
}
