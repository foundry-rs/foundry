//! Implementations of [`Evm`](spec::Group::Evm) cheatcodes.

use crate::{
    BroadcastableTransaction, Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Error, Result,
    Vm::*,
    inspector::{Ecx, RecordDebugStepInfo},
};
use alloy_consensus::TxEnvelope;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_network::eip2718::EIP4844_TX_TYPE_ID;
use alloy_primitives::{Address, B256, U256, hex, map::HashMap};
use alloy_rlp::Decodable;
use alloy_sol_types::SolValue;
use foundry_common::fs::{read_json_file, write_json_file};
use foundry_compilers::artifacts::{Storage, StorageLayout, StorageType};
use foundry_evm_core::{
    ContextExt,
    backend::{DatabaseExt, RevertStateSnapshotAction},
    constants::{CALLER, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, TEST_CONTRACT_ADDRESS},
    utils::get_blob_base_fee_update_fraction_by_spec_id,
};
use foundry_evm_traces::StackSnapshotType;
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
use record_debug_step::{convert_call_trace_to_debug_step, flatten_call_trace};
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
    /// Decoded values according to the Solidity type (e.g., uint256, address).
    /// Only present when storage layout is available and decoding succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded: Option<DecodedSlotValues>,

    /// Storage layout metadata (variable name, type, offset).
    /// Only present when contract has storage layout output.
    #[serde(skip_serializing_if = "Option::is_none", flatten)]
    slot_info: Option<SlotInfo>,
}

/// Custom serializer for mapping keys that creates "key" or "keys" field based on count
fn serialize_mapping_keys<S>(keys: &Option<Vec<String>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;

    match keys {
        None => serializer.serialize_none(),
        Some(keys) => {
            let mut map = serializer.serialize_map(Some(1))?;

            if keys.len() == 1 {
                // Single key: serialize as "key": "value"
                map.serialize_entry("key", &keys[0])?;
            } else {
                // Multiple keys: serialize as "keys": ["value1", "value2", ...]
                map.serialize_entry("keys", keys)?;
            }

            map.end()
        }
    }
}

#[derive(Serialize, Debug)]
struct SlotInfo {
    /// The variable name from the storage layout.
    ///
    /// For top-level variables: just the variable name (e.g., "myVariable")
    /// For struct members: dotted path (e.g., "myStruct.memberName")
    /// For array elements: name with indices (e.g., "myArray\[0\]", "matrix\[1\]\[2\]")
    /// For nested structures: full path (e.g., "outer.inner.field")
    /// For mappings: base name only (e.g., "balances"), keys are in mapping_info
    label: String,
    #[serde(rename = "type", serialize_with = "serialize_slot_type")]
    slot_type: StorageTypeInfo,
    offset: i64,
    slot: String,
    /// For struct members, contains nested SlotInfo for each member
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<Vec<SlotInfo>>,
    /// Decoded values (if available) - used for struct members
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded: Option<DecodedSlotValues>,
    /// Decoded mapping keys (serialized as "key" for single, "keys" for multiple)
    #[serde(
        skip_serializing_if = "Option::is_none",
        flatten,
        serialize_with = "serialize_mapping_keys"
    )]
    keys: Option<Vec<String>>,
}

/// Wrapper type that holds both the original type label and the parsed DynSolType.
///
/// We need both because:
/// - `label`: Contains the exact Solidity type string from the storage layout (e.g., "struct
///   Contract.StructName", "uint256", "address\[2\]\[3\]")
/// - `dyn_sol_type`: The parsed type used for actual value decoding
#[derive(Debug)]
struct StorageTypeInfo {
    /// This label is used during serialization to ensure the output matches
    /// what users expect to see in the state diff JSON.
    label: String,
    dyn_sol_type: DynSolType,
}

fn serialize_slot_type<S>(slot_type: &StorageTypeInfo, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // For CustomStruct, format as "struct Name", otherwise use the label
    let type_str = match &slot_type.dyn_sol_type {
        DynSolType::CustomStruct { name, .. } => {
            // If the label already has "struct " prefix, use it as-is
            if slot_type.label.starts_with("struct ") {
                slot_type.label.clone()
            } else {
                format!("struct {name}")
            }
        }
        _ => slot_type.label.clone(),
    };
    serializer.serialize_str(&type_str)
}

#[derive(Debug)]
struct DecodedSlotValues {
    /// Initial decoded storage value
    previous_value: DynSolValue,
    /// Current decoded storage value
    new_value: DynSolValue,
}

impl Serialize for DecodedSlotValues {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("DecodedSlotValues", 2)?;
        state.serialize_field("previousValue", &format_dyn_sol_value_raw(&self.previous_value))?;
        state.serialize_field("newValue", &format_dyn_sol_value_raw(&self.new_value))?;
        state.end()
    }
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
                match (&slot_changes.slot_info, &slot_changes.decoded) {
                    (Some(slot_info), Some(decoded)) => {
                        // Have both slot info and decoded values - only show decoded values
                        writeln!(
                            f,
                            "@ {slot} ({}, {}): {} → {}",
                            slot_info.label,
                            slot_info.slot_type.dyn_sol_type,
                            format_dyn_sol_value_raw(&decoded.previous_value),
                            format_dyn_sol_value_raw(&decoded.new_value)
                        )?;
                    }
                    (Some(slot_info), None) => {
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
                    _ => {
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
        ccx.ecx.journaled_state.load_account(target)?;
        let mut val = ccx.ecx.journaled_state.sload(target, slot.into())?;

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

impl Cheatcode for chainIdCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { newChainId } = self;
        ensure!(*newChainId <= U256::from(u64::MAX), "chain ID must be less than 2^64 - 1");
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
        ensure!(*newBasefee <= U256::from(u64::MAX), "base fee must be less than 2^64 - 1");
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
        ensure!(*newGasPrice <= U256::from(u64::MAX), "gas price must be less than 2^64 - 1");
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
        ccx.ecx.journaled_state.load_account(*target)?;
        let bytecode = Bytecode::new_raw_checked(newRuntimeBytecode.clone())
            .map_err(|e| fmt_err!("failed to create bytecode: {e}"))?;
        ccx.ecx.journaled_state.set_code(*target, bytecode);
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
        ccx.ecx.journaled_state.sstore(target, slot.into(), value.into())?;
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
        if state.mapping_slots.is_none() {
            state.mapping_slots = Some(Default::default());
        }
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
        ensure!(blockNumber <= U256::from(u64::MAX), "blockNumber must be less than 2^64 - 1");
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

        // turn on tracer configuration for recording
        tracer.update_config(|config| {
            config
                .set_steps(true)
                .set_memory_snapshots(true)
                .set_stack_snapshots(StackSnapshotType::Full)
        });

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
            steps.iter().map(|&step| convert_call_trace_to_debug_step(step)).collect();
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

pub(super) fn get_nonce(ccx: &mut CheatsCtxt, address: &Address) -> Result {
    let account = ccx.ecx.journaled_state.load_account(*address)?;
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
    if ccx.state.gas_metering.active_gas_snapshot.is_some() {
        let (group, name) = ccx.state.gas_metering.active_gas_snapshot.as_ref().unwrap().clone();
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
    let (group, name) = group.zip(name).unwrap_or_else(|| {
        let (group, name) = ccx.state.gas_metering.active_gas_snapshot.as_ref().unwrap().clone();
        (group, name)
    });

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
    ecx.journaled_state.load_account(addr)?;
    ecx.journaled_state.touch(addr);
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
        if let Some(name) = get_contract_name(ccx, address) {
            contract_names.insert(address, name);
        }

        // Also get storage layout if available
        if let Some((_artifact_id, contract_data)) = get_contract_data(ccx, address)
            && let Some(storage_layout) = &contract_data.storage_layout
        {
            storage_layouts.insert(address, storage_layout.clone());
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
                        match account_diff.state_diff.entry(storage_access.slot) {
                            Entry::Vacant(slot_state_diff) => {
                                // Get storage layout info for this slot
                                // Include mapping slots if available for the account
                                let mapping_slots = ccx
                                    .state
                                    .mapping_slots
                                    .as_ref()
                                    .and_then(|slots| slots.get(&storage_access.account));

                                let slot_info = layout.and_then(|layout| {
                                    get_slot_info(layout, &storage_access.slot, mapping_slots)
                                });

                                // Try to decode values if we have slot info
                                let (decoded, slot_info_with_decoded) = if let Some(mut info) =
                                    slot_info
                                {
                                    // Check if this is a struct with members
                                    if let Some(ref mut members) = info.members {
                                        // Decode each member individually
                                        for member in members.iter_mut() {
                                            let offset = member.offset as usize;
                                            let size = match &member.slot_type.dyn_sol_type {
                                                DynSolType::Uint(bits) | DynSolType::Int(bits) => {
                                                    bits / 8
                                                }
                                                DynSolType::Address => 20,
                                                DynSolType::Bool => 1,
                                                DynSolType::FixedBytes(size) => *size,
                                                _ => 32, // Default to full word
                                            };

                                            // Extract and decode member values
                                            let mut prev_bytes = [0u8; 32];
                                            let mut new_bytes = [0u8; 32];

                                            if offset + size <= 32 {
                                                // In Solidity storage, values are right-aligned
                                                // For offset 0, we want the rightmost bytes
                                                // For offset 16 (for a uint128), we want bytes
                                                // 0-16
                                                // For packed storage: offset 0 is at the rightmost
                                                // position
                                                // offset 0, size 16 -> read bytes 16-32 (rightmost)
                                                // offset 16, size 16 -> read bytes 0-16 (leftmost)
                                                let byte_start = 32 - offset - size;
                                                prev_bytes[32 - size..].copy_from_slice(
                                                    &storage_access.previousValue.0
                                                        [byte_start..byte_start + size],
                                                );
                                                new_bytes[32 - size..].copy_from_slice(
                                                    &storage_access.newValue.0
                                                        [byte_start..byte_start + size],
                                                );
                                            }

                                            // Decode the member values
                                            if let (Ok(prev_val), Ok(new_val)) = (
                                                member
                                                    .slot_type
                                                    .dyn_sol_type
                                                    .abi_decode(&prev_bytes),
                                                member
                                                    .slot_type
                                                    .dyn_sol_type
                                                    .abi_decode(&new_bytes),
                                            ) {
                                                member.decoded = Some(DecodedSlotValues {
                                                    previous_value: prev_val,
                                                    new_value: new_val,
                                                });
                                            }
                                        }
                                        // For structs with members, we don't need a top-level
                                        // decoded value
                                        (None, Some(info))
                                    } else {
                                        // Not a struct, decode as a single value
                                        let storage_layout =
                                            storage_layouts.get(&storage_access.account);
                                        let storage_type = storage_layout.and_then(|layout| {
                                            layout
                                                .storage
                                                .iter()
                                                .find(|s| s.slot == info.slot)
                                                .and_then(|s| layout.types.get(&s.storage_type))
                                        });

                                        let decoded = if let (Some(prev), Some(new)) = (
                                            decode_storage_value(
                                                storage_access.previousValue,
                                                &info.slot_type.dyn_sol_type,
                                                storage_type,
                                                storage_layout.as_ref().map(|arc| arc.as_ref()),
                                            ),
                                            decode_storage_value(
                                                storage_access.newValue,
                                                &info.slot_type.dyn_sol_type,
                                                storage_type,
                                                storage_layout.as_ref().map(|arc| arc.as_ref()),
                                            ),
                                        ) {
                                            Some(DecodedSlotValues {
                                                previous_value: prev,
                                                new_value: new,
                                            })
                                        } else {
                                            None
                                        };
                                        (decoded, Some(info))
                                    }
                                } else {
                                    (None, None)
                                };

                                slot_state_diff.insert(SlotStateDiff {
                                    previous_value: storage_access.previousValue,
                                    new_value: storage_access.newValue,
                                    decoded,
                                    slot_info: slot_info_with_decoded,
                                });
                            }
                            Entry::Occupied(mut slot_state_diff) => {
                                let entry = slot_state_diff.get_mut();
                                entry.new_value = storage_access.newValue;

                                if let Some(slot_info) = &entry.slot_info
                                    && let Some(ref mut decoded) = entry.decoded
                                {
                                    // Get storage type info
                                    let storage_type = layout.and_then(|layout| {
                                        // Find the storage item that matches this slot
                                        layout
                                            .storage
                                            .iter()
                                            .find(|s| s.slot == slot_info.slot)
                                            .and_then(|s| layout.types.get(&s.storage_type))
                                    });

                                    if let Some(new_value) = decode_storage_value(
                                        storage_access.newValue,
                                        &slot_info.slot_type.dyn_sol_type,
                                        storage_type,
                                        layout.as_ref().map(|arc| arc.as_ref()),
                                    ) {
                                        decoded.new_value = new_value;
                                    }
                                }
                            }
                        }
                    }
                }
            });
    }
    state_diffs
}

/// Helper function to get the contract name from the deployed code.
fn get_contract_name(ccx: &mut CheatsCtxt, address: Address) -> Option<String> {
    // Check if we have available artifacts to match against
    let artifacts = ccx.state.config.available_artifacts.as_ref()?;

    // Try to load the account and get its code
    let account = ccx.ecx.journaled_state.load_account(address).ok()?;
    let code = account.info.code.as_ref()?;

    // Skip if code is empty
    if code.is_empty() {
        return None;
    }

    // Try to find the artifact by deployed code
    let code_bytes = code.original_bytes();
    if let Some((artifact_id, _)) = artifacts.find_by_deployed_code_exact(&code_bytes) {
        return Some(artifact_id.identifier());
    }

    // Fallback to fuzzy matching if exact match fails
    if let Some((artifact_id, _)) = artifacts.find_by_deployed_code(&code_bytes) {
        return Some(artifact_id.identifier());
    }

    None
}

/// Helper function to get the contract data from the deployed code at an address.
fn get_contract_data<'a>(
    ccx: &'a mut CheatsCtxt,
    address: Address,
) -> Option<(&'a foundry_compilers::ArtifactId, &'a foundry_common::contracts::ContractData)> {
    // Check if we have available artifacts to match against
    let artifacts = ccx.state.config.available_artifacts.as_ref()?;

    // Try to load the account and get its code
    let account = ccx.ecx.journaled_state.load_account(address).ok()?;
    let code = account.info.code.as_ref()?;

    // Skip if code is empty
    if code.is_empty() {
        return None;
    }

    // Try to find the artifact by deployed code
    let code_bytes = code.original_bytes();
    if let Some(result) = artifacts.find_by_deployed_code_exact(&code_bytes) {
        return Some(result);
    }

    // Fallback to fuzzy matching if exact match fails
    artifacts.find_by_deployed_code(&code_bytes)
}

/// Gets storage layout info for a specific slot
fn get_slot_info(
    storage_layout: &StorageLayout,
    slot: &B256,
    mapping_slots: Option<&mapping::MappingSlots>,
) -> Option<SlotInfo> {
    let slot_u256 = U256::from_be_bytes(slot.0);
    let slot_str = slot_u256.to_string();

    for storage in &storage_layout.storage {
        let storage_type = storage_layout.types.get(&storage.storage_type)?;
        let dyn_type = DynSolType::parse(&storage_type.label).ok();

        // Check if we're able to match on a slot from the layout
        if storage.slot == slot_str
            && let Some(parsed_type) = dyn_type
        {
            // Successfully parsed - handle arrays or simple types
            let label = if let DynSolType::FixedArray(_, _) = &parsed_type {
                format!("{}{}", storage.label, get_array_base_indices(&parsed_type))
            } else {
                storage.label.clone()
            };

            return Some(SlotInfo {
                label,
                slot_type: StorageTypeInfo {
                    label: storage_type.label.clone(),
                    dyn_sol_type: parsed_type,
                },
                offset: storage.offset,
                slot: storage.slot.clone(),
                members: None,
                decoded: None,
                keys: None,
            });
        }

        // Handle the case where the accessed `slot` if maybe different from the base slot.
        let array_start_slot = U256::from_str(&storage.slot).ok()?;

        if let Some(parsed_type) = dyn_type
            && let DynSolType::FixedArray(_, _) = parsed_type
            && let Some(slot_info) =
                handle_array_slot(storage, storage_type, slot_u256, array_start_slot, &slot_str)
        {
            return Some(slot_info);
        }

        // If type parsing fails and the label is a struct
        if storage_type.label.starts_with("struct ")
            && let Some(slot_info) =
                handle_struct(storage, storage_type, storage_layout, slot_u256, &slot_str)
        {
            return Some(slot_info);
        }

        // Check if this is a mapping slot
        if let Some(mapping_slots) = mapping_slots
            && let Some(slot_info) = handle_mapping(
                storage,
                storage_type,
                slot,
                &slot_str,
                mapping_slots,
                storage_layout,
            )
        {
            return Some(slot_info);
        }
    }

    None
}

/// Recursively resolves a mapping type reference, handling nested mappings.
///
/// For a mapping type like `mapping(address => mapping(uint256 => bool))`, this function
/// traverses the type hierarchy to extract all key types and the final value type.
///
/// # Returns
/// Returns `Some((key_types, final_value_type, full_type_label))` where:
/// * `key_types` - A vector of all key types from outermost to innermost (e.g., ["address",
///   "uint256"])
/// * `final_value_type` - The ultimate value type at the end of the chain (e.g., "bool")
/// * `full_type_label` - The complete mapping type label (e.g., "mapping(address => mapping(uint256
///   => bool))")
///
/// Returns `None` if the type reference cannot be resolved.
fn resolve_mapping_type(
    type_ref: &str,
    storage_layout: &StorageLayout,
) -> Option<(Vec<String>, String, String)> {
    let storage_type = storage_layout.types.get(type_ref)?;

    if storage_type.encoding != "mapping" {
        // Not a mapping, return the type as-is
        return Some((vec![], storage_type.label.clone(), storage_type.label.clone()));
    }

    // Get key and value type references
    let key_type_ref = storage_type.key.as_ref()?;
    let value_type_ref = storage_type.value.as_ref()?;

    // Resolve the key type
    let key_type = storage_layout.types.get(key_type_ref)?;
    let mut key_types = vec![key_type.label.clone()];

    // Check if the value is another mapping (nested case)
    if let Some(value_storage_type) = storage_layout.types.get(value_type_ref) {
        if value_storage_type.encoding == "mapping" {
            // Recursively resolve the nested mapping
            let (nested_keys, final_value, _) =
                resolve_mapping_type(value_type_ref, storage_layout)?;
            key_types.extend(nested_keys);
            return Some((key_types, final_value, storage_type.label.clone()));
        } else {
            // Value is not a mapping, we're done
            return Some((key_types, value_storage_type.label.clone(), storage_type.label.clone()));
        }
    }

    None
}

/// Handles mapping slot access by checking if the given slot is a known mapping entry
/// and decoding the key to create a readable label.
fn handle_mapping(
    storage: &Storage,
    storage_type: &StorageType,
    slot: &B256,
    slot_str: &str,
    mapping_slots: &mapping::MappingSlots,
    storage_layout: &StorageLayout,
) -> Option<SlotInfo> {
    trace!(
        "handle_mapping: storage.slot={}, slot={:?}, has_keys={}, has_parents={}",
        storage.slot,
        slot,
        mapping_slots.keys.contains_key(slot),
        mapping_slots.parent_slots.contains_key(slot)
    );

    // Verify it's actually a mapping type
    if storage_type.encoding != "mapping" {
        return None;
    }

    // Check if this slot is a known mapping entry
    if !mapping_slots.keys.contains_key(slot) {
        return None;
    }

    // Convert storage.slot to B256 for comparison
    let storage_slot_b256 = B256::from(U256::from_str(&storage.slot).ok()?);

    // Walk up the parent chain to collect keys and validate the base slot
    // This single traversal both validates and collects keys
    let mut current_slot = *slot;
    let mut keys_to_decode = Vec::new();
    let mut found_base = false;

    while let Some((key, parent)) =
        mapping_slots.keys.get(&current_slot).zip(mapping_slots.parent_slots.get(&current_slot))
    {
        keys_to_decode.push(*key);

        // Check if the parent is our base storage slot
        if *parent == storage_slot_b256 {
            found_base = true;
            break;
        }

        // Move up to the parent for the next iteration
        current_slot = *parent;
    }

    if !found_base {
        trace!("Mapping slot {} does not match any parent in chain", storage.slot);
        return None;
    }

    // Resolve the mapping type to get all key types and the final value type
    let (key_types, value_type_label, full_type_label) =
        resolve_mapping_type(&storage.storage_type, storage_layout)?;

    // Reverse keys to process from outermost to innermost
    keys_to_decode.reverse();

    // Build the label with decoded keys and collect decoded key values
    let mut label = storage.label.clone();
    let mut decoded_keys = Vec::new();

    // Decode each key using the corresponding type
    for (i, key) in keys_to_decode.iter().enumerate() {
        if let Some(key_type_label) = key_types.get(i)
            && let Ok(sol_type) = DynSolType::parse(key_type_label)
            && let Ok(decoded) = sol_type.abi_decode(&key.0)
        {
            let decoded_key_str = format_dyn_sol_value_raw(&decoded);
            decoded_keys.push(decoded_key_str.clone());
            label = format!("{label}[{decoded_key_str}]");
        } else {
            let hex_key = hex::encode_prefixed(key.0);
            decoded_keys.push(hex_key.clone());
            label = format!("{label}[{hex_key}]");
        }
    }

    // Parse the final value type for decoding
    let dyn_sol_type = DynSolType::parse(&value_type_label).unwrap_or(DynSolType::Bytes);

    Some(SlotInfo {
        label,
        slot_type: StorageTypeInfo { label: full_type_label, dyn_sol_type },
        offset: storage.offset,
        slot: slot_str.to_string(),
        members: None,
        decoded: None,
        keys: Some(decoded_keys),
    })
}

/// Handles array slot access.
fn handle_array_slot(
    storage: &Storage,
    storage_type: &StorageType,
    slot: U256,
    array_start_slot: U256, // The slot where this array begins
    slot_str: &str,
) -> Option<SlotInfo> {
    // Check if slot is within array bounds
    let total_bytes = storage_type.number_of_bytes.parse::<u64>().ok()?;
    let total_slots = total_bytes.div_ceil(32);

    if slot >= array_start_slot && slot < array_start_slot + U256::from(total_slots) {
        let parsed_type = DynSolType::parse(&storage_type.label).ok()?;
        let index = (slot - array_start_slot).to::<u64>();
        // Format the array element label based on array dimensions
        let label = match &parsed_type {
            DynSolType::FixedArray(inner, _) => {
                if let DynSolType::FixedArray(_, inner_size) = inner.as_ref() {
                    // 2D array: calculate row and column
                    let row = index / (*inner_size as u64);
                    let col = index % (*inner_size as u64);
                    format!("{}[{row}][{col}]", storage.label)
                } else {
                    // 1D array
                    format!("{}[{index}]", storage.label)
                }
            }
            _ => storage.label.clone(),
        };

        return Some(SlotInfo {
            label,
            slot_type: StorageTypeInfo {
                label: storage_type.label.clone(),
                dyn_sol_type: parsed_type,
            },
            offset: 0,
            slot: slot_str.to_string(),
            members: None,
            decoded: None,
            keys: None,
        });
    }

    None
}

/// Returns the base index [\0\] or [\0\][\0\] for a fixed array type depending on the dimensions.
fn get_array_base_indices(dyn_type: &DynSolType) -> String {
    match dyn_type {
        DynSolType::FixedArray(inner, _) => {
            if let DynSolType::FixedArray(_, _) = inner.as_ref() {
                // Nested array (2D or higher)
                format!("[0]{}", get_array_base_indices(inner))
            } else {
                // Simple 1D array
                "[0]".to_string()
            }
        }
        _ => String::new(),
    }
}

/// Context for recursive struct processing
struct StructContext<'a> {
    storage_layout: &'a StorageLayout,
    target_slot: U256, // The slot we're trying to decode
    slot_str: String,  // String representation of target_slot
}

/// Recursively processes a struct and finds the slot info for the requested slot.
/// This handles both accessing the struct itself and accessing its members at any depth.
fn handle_struct_recursive(
    ctx: &StructContext,
    base_label: &str,
    storage_type: &StorageType,
    struct_start_slot: U256, // The slot where this struct begins
    offset: i64,
    depth: usize,
) -> Option<SlotInfo> {
    // Limit recursion depth to prevent stack overflow
    const MAX_DEPTH: usize = 10;
    if depth > MAX_DEPTH {
        return None;
    }

    let members = storage_type
        .other
        .get("members")
        .and_then(|v| serde_json::from_value::<Vec<Storage>>(v.clone()).ok())?;

    // If this is the exact slot we're looking for (struct's base slot)
    if struct_start_slot == ctx.target_slot {
        // For structs, we need to determine what to return:
        // - If all members are in the same slot (single-slot struct), return the struct with member
        //   info
        // - If members span multiple slots, return the first member at this slot

        // Find the member at slot offset 0 (the member that starts at this slot)
        if let Some(first_member) = members.iter().find(|m| m.slot == "0") {
            let member_type_info = ctx.storage_layout.types.get(&first_member.storage_type)?;

            // Check if we have a single-slot struct (all members have slot "0")
            let is_single_slot = members.iter().all(|m| m.slot == "0");

            if is_single_slot {
                // Build member info for single-slot struct
                let mut member_infos = Vec::new();
                for member in &members {
                    if let Some(member_type_info) =
                        ctx.storage_layout.types.get(&member.storage_type)
                        && let Some(member_type) = DynSolType::parse(&member_type_info.label).ok()
                    {
                        member_infos.push(SlotInfo {
                            label: member.label.clone(),
                            slot_type: StorageTypeInfo {
                                label: member_type_info.label.clone(),
                                dyn_sol_type: member_type,
                            },
                            offset: member.offset,
                            slot: ctx.slot_str.clone(),
                            members: None,
                            decoded: None,
                            keys: None,
                        });
                    }
                }

                // Build the CustomStruct type
                let struct_name =
                    storage_type.label.strip_prefix("struct ").unwrap_or(&storage_type.label);
                let prop_names: Vec<String> = members.iter().map(|m| m.label.clone()).collect();
                let member_types: Vec<DynSolType> =
                    member_infos.iter().map(|info| info.slot_type.dyn_sol_type.clone()).collect();

                let parsed_type = DynSolType::CustomStruct {
                    name: struct_name.to_string(),
                    prop_names,
                    tuple: member_types,
                };

                return Some(SlotInfo {
                    label: base_label.to_string(),
                    slot_type: StorageTypeInfo {
                        label: storage_type.label.clone(),
                        dyn_sol_type: parsed_type,
                    },
                    offset,
                    slot: ctx.slot_str.clone(),
                    decoded: None,
                    members: if member_infos.is_empty() { None } else { Some(member_infos) },
                    keys: None,
                });
            } else {
                // Multi-slot struct - return the first member.
                let member_label = format!("{}.{}", base_label, first_member.label);

                // If the first member is itself a struct, recurse
                if member_type_info.label.starts_with("struct ") {
                    return handle_struct_recursive(
                        ctx,
                        &member_label,
                        member_type_info,
                        struct_start_slot, // First member is at the same slot
                        first_member.offset,
                        depth + 1,
                    );
                }

                // Return the first member as a primitive
                return Some(SlotInfo {
                    label: member_label,
                    slot_type: StorageTypeInfo {
                        label: member_type_info.label.clone(),
                        dyn_sol_type: DynSolType::parse(&member_type_info.label).ok()?,
                    },
                    offset: first_member.offset,
                    slot: ctx.slot_str.clone(),
                    decoded: None,
                    members: None,
                    keys: None,
                });
            }
        }
    }

    // Not the base slot - search through members
    for member in &members {
        let member_slot_offset = U256::from_str(&member.slot).ok()?;
        let member_slot = struct_start_slot + member_slot_offset;
        let member_type_info = ctx.storage_layout.types.get(&member.storage_type)?;
        let member_label = format!("{}.{}", base_label, member.label);

        if member_slot == ctx.target_slot {
            // Found the exact member slot

            // If this member is a struct, recurse into it
            if member_type_info.label.starts_with("struct ") {
                return handle_struct_recursive(
                    ctx,
                    &member_label,
                    member_type_info,
                    member_slot,
                    member.offset,
                    depth + 1,
                );
            }

            // Regular member
            let member_type = DynSolType::parse(&member_type_info.label).ok()?;
            return Some(SlotInfo {
                label: member_label,
                slot_type: StorageTypeInfo {
                    label: member_type_info.label.clone(),
                    dyn_sol_type: member_type,
                },
                offset: member.offset,
                slot: ctx.slot_str.clone(),
                members: None,
                decoded: None,
                keys: None,
            });
        }

        // If this member is a struct and the requested slot might be inside it, recurse
        if member_type_info.label.starts_with("struct ")
            && let Some(slot_info) = handle_struct_recursive(
                ctx,
                &member_label,
                member_type_info,
                member_slot,
                member.offset,
                depth + 1,
            )
        {
            return Some(slot_info);
        }
    }

    None
}

/// Handles struct slot decoding for both direct and member slot access.
fn handle_struct(
    storage: &Storage,
    storage_type: &StorageType,
    storage_layout: &StorageLayout,
    target_slot: U256,
    slot_str: &str,
) -> Option<SlotInfo> {
    let struct_start_slot = U256::from_str(&storage.slot).ok()?;

    let ctx = StructContext { storage_layout, target_slot, slot_str: slot_str.to_string() };

    handle_struct_recursive(
        &ctx,
        &storage.label,
        storage_type,
        struct_start_slot,
        storage.offset,
        0,
    )
}

/// Helper function to decode a single storage value using its DynSolType
fn decode_storage_value(
    value: B256,
    dyn_type: &DynSolType,
    storage_type: Option<&StorageType>,
    storage_layout: Option<&StorageLayout>,
) -> Option<DynSolValue> {
    // Storage values are always 32 bytes, stored as a single word
    // For arrays, we need to unwrap to the base element type
    let mut actual_type = dyn_type;
    // Unwrap nested arrays to get to the base element type.
    while let DynSolType::FixedArray(elem_type, _) = actual_type {
        actual_type = elem_type.as_ref();
    }

    // For tuples (structs), we need to decode each member based on its offset
    if let DynSolType::Tuple(member_types) = actual_type {
        // If we have storage type info with members, decode each member from the value
        if let Some(st) = storage_type
            && let Some(members_value) = st.other.get("members")
            && let Ok(members) = serde_json::from_value::<Vec<Storage>>(members_value.clone())
            && members.len() == member_types.len()
        {
            let mut decoded_members = Vec::new();

            for (i, member) in members.iter().enumerate() {
                // Get the member type
                let member_type = &member_types[i];

                // Calculate byte range for this member
                let offset = member.offset as usize;
                let member_storage_type =
                    storage_layout.and_then(|sl| sl.types.get(&member.storage_type));
                let size = member_storage_type
                    .and_then(|t| t.number_of_bytes.parse::<usize>().ok())
                    .unwrap_or(32);

                // Extract bytes for this member from the full value
                let mut member_bytes = [0u8; 32];
                if offset + size <= 32 {
                    // For packed storage: offset 0 is at the rightmost position
                    // offset 0, size 16 -> read bytes 16-32 (rightmost)
                    // offset 16, size 16 -> read bytes 0-16 (leftmost)
                    let byte_start = 32 - offset - size;
                    member_bytes[32 - size..]
                        .copy_from_slice(&value.0[byte_start..byte_start + size]);
                }

                // Decode the member value
                if let Ok(decoded) = member_type.abi_decode(&member_bytes) {
                    decoded_members.push(decoded);
                } else {
                    return None;
                }
            }

            return Some(DynSolValue::Tuple(decoded_members));
        }
    }

    // Use abi_decode to decode the value for non-struct types
    actual_type.abi_decode(&value.0).ok()
}

/// Helper function to format DynSolValue as raw string without type information
fn format_dyn_sol_value_raw(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Int(i, _) => i.to_string(),
        DynSolValue::Uint(u, _) => u.to_string(),
        DynSolValue::FixedBytes(bytes, size) => hex::encode_prefixed(&bytes.0[..*size]),
        DynSolValue::Address(addr) => addr.to_string(),
        DynSolValue::Function(func) => func.as_address_and_selector().1.to_string(),
        DynSolValue::Bytes(bytes) => hex::encode_prefixed(bytes),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Array(values) | DynSolValue::FixedArray(values) => {
            let formatted: Vec<String> = values.iter().map(format_dyn_sol_value_raw).collect();
            format!("[{}]", formatted.join(", "))
        }
        DynSolValue::Tuple(values) => {
            let formatted: Vec<String> = values.iter().map(format_dyn_sol_value_raw).collect();
            format!("({})", formatted.join(", "))
        }
        DynSolValue::CustomStruct { name: _, prop_names: _, tuple } => {
            format_dyn_sol_value_raw(&DynSolValue::Tuple(tuple.clone()))
        }
    }
}

/// Helper function to set / unset cold storage slot of the target address.
fn set_cold_slot(ccx: &mut CheatsCtxt, target: Address, slot: U256, cold: bool) {
    if let Some(account) = ccx.ecx.journaled_state.state.get_mut(&target)
        && let Some(storage_slot) = account.storage.get_mut(&slot)
    {
        storage_slot.is_cold = cold;
    }
}
