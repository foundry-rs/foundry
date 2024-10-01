//! Implementations of [`Evm`](spec::Group::Evm) cheatcodes.

use crate::{
    BroadcastableTransaction, Cheatcode, Cheatcodes, CheatcodesExecutor, CheatsCtxt, Result, Vm::*,
};
use alloy_consensus::TxEnvelope;
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rlp::Decodable;
use alloy_sol_types::SolValue;
use foundry_common::fs::{read_json_file, write_json_file};
use foundry_evm_core::{
    backend::{DatabaseExt, RevertStateSnapshotAction},
    constants::{CALLER, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, TEST_CONTRACT_ADDRESS},
};
use rand::Rng;
use revm::{
    primitives::{Account, Bytecode, SpecId, KECCAK_EMPTY},
    InnerEvmContext,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

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

impl Cheatcode for addrCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { privateKey } = self;
        let wallet = super::crypto::parse_wallet(privateKey)?;
        Ok(wallet.address().abi_encode())
    }
}

impl Cheatcode for getNonce_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account } = self;
        get_nonce(ccx, account)
    }
}

impl Cheatcode for getNonce_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { wallet } = self;
        get_nonce(ccx, &wallet.addr)
    }
}

impl Cheatcode for loadCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { target, slot } = *self;
        ensure_not_precompile!(&target, ccx);
        ccx.ecx.load_account(target)?;
        let mut val = ccx.ecx.sload(target, slot.into())?;

        if val.is_cold && val.data.is_zero() {
            if ccx.state.has_arbitrary_storage(&target) {
                // If storage slot is untouched and load from a target with arbitrary storage,
                // then set random value for current slot.
                let rand_value = ccx.state.rng().gen();
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
                let rand_value = ccx.state.rng().gen();
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
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
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
        ccx.ecx
            .db
            .load_allocs(&allocs, &mut ccx.ecx.journaled_state)
            .map(|()| Vec::default())
            .map_err(|e| fmt_err!("failed to load allocs: {e}"))
    }
}

impl Cheatcode for dumpStateCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { pathToStateJson } = self;
        let path = Path::new(pathToStateJson);

        // Do not include system account or empty accounts in the dump.
        let skip = |key: &Address, val: &Account| {
            key == &CHEATCODE_ADDRESS ||
                key == &CALLER ||
                key == &HARDHAT_CONSOLE_ADDRESS ||
                key == &TEST_CONTRACT_ADDRESS ||
                key == &ccx.caller ||
                key == &ccx.state.config.evm_opts.sender ||
                val.is_empty()
        };

        let alloc = ccx
            .ecx
            .journaled_state
            .state()
            .iter_mut()
            .filter(|(key, val)| !skip(key, val))
            .map(|(key, val)| {
                (
                    key,
                    GenesisAccount {
                        nonce: Some(val.info.nonce),
                        balance: val.info.balance,
                        code: val.info.code.as_ref().map(|o| o.original_bytes()),
                        storage: Some(
                            val.storage
                                .iter()
                                .map(|(k, v)| (B256::from(*k), B256::from(v.present_value())))
                                .collect(),
                        ),
                        private_key: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        write_json_file(path, &alloc)?;
        Ok(Default::default())
    }
}

impl Cheatcode for recordCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.accesses = Some(Default::default());
        Ok(Default::default())
    }
}

impl Cheatcode for accessesCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { target } = *self;
        let result = state
            .accesses
            .as_mut()
            .map(|accesses| {
                (
                    &accesses.reads.entry(target).or_default()[..],
                    &accesses.writes.entry(target).or_default()[..],
                )
            })
            .unwrap_or_default();
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
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newChainId } = self;
        ensure!(*newChainId <= U256::from(u64::MAX), "chain ID must be less than 2^64 - 1");
        ccx.ecx.env.cfg.chain_id = newChainId.to();
        Ok(Default::default())
    }
}

impl Cheatcode for coinbaseCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newCoinbase } = self;
        ccx.ecx.env.block.coinbase = *newCoinbase;
        Ok(Default::default())
    }
}

impl Cheatcode for difficultyCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newDifficulty } = self;
        ensure!(
            ccx.ecx.spec_id() < SpecId::MERGE,
            "`difficulty` is not supported after the Paris hard fork, use `prevrandao` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.env.block.difficulty = *newDifficulty;
        Ok(Default::default())
    }
}

impl Cheatcode for feeCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newBasefee } = self;
        ccx.ecx.env.block.basefee = *newBasefee;
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.ecx.spec_id() >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.env.block.prevrandao = Some(*newPrevrandao);
        Ok(Default::default())
    }
}

impl Cheatcode for prevrandao_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newPrevrandao } = self;
        ensure!(
            ccx.ecx.spec_id() >= SpecId::MERGE,
            "`prevrandao` is not supported before the Paris hard fork, use `difficulty` instead; \
             see EIP-4399: https://eips.ethereum.org/EIPS/eip-4399"
        );
        ccx.ecx.env.block.prevrandao = Some((*newPrevrandao).into());
        Ok(Default::default())
    }
}

impl Cheatcode for blobhashesCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { hashes } = self;
        ensure!(
            ccx.ecx.spec_id() >= SpecId::CANCUN,
            "`blobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        ccx.ecx.env.tx.blob_hashes.clone_from(hashes);
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobhashesCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        ensure!(
            ccx.ecx.spec_id() >= SpecId::CANCUN,
            "`getBlobhashes` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        Ok(ccx.ecx.env.tx.blob_hashes.clone().abi_encode())
    }
}

impl Cheatcode for rollCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newHeight } = self;
        ccx.ecx.env.block.number = *newHeight;
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockNumberCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.env.block.number.abi_encode())
    }
}

impl Cheatcode for txGasPriceCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newGasPrice } = self;
        ccx.ecx.env.tx.gas_price = *newGasPrice;
        Ok(Default::default())
    }
}

impl Cheatcode for warpCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newTimestamp } = self;
        ccx.ecx.env.block.timestamp = *newTimestamp;
        Ok(Default::default())
    }
}

impl Cheatcode for getBlockTimestampCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.env.block.timestamp.abi_encode())
    }
}

impl Cheatcode for blobBaseFeeCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { newBlobBaseFee } = self;
        ensure!(
            ccx.ecx.spec_id() >= SpecId::CANCUN,
            "`blobBaseFee` is not supported before the Cancun hard fork; \
             see EIP-4844: https://eips.ethereum.org/EIPS/eip-4844"
        );
        ccx.ecx.env.block.set_blob_excess_gas_and_price((*newBlobBaseFee).to());
        Ok(Default::default())
    }
}

impl Cheatcode for getBlobBaseFeeCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        Ok(ccx.ecx.env.block.get_blob_excess_gas().unwrap_or(0).abi_encode())
    }
}

impl Cheatcode for dealCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account: address, newBalance: new_balance } = *self;
        let account = journaled_account(ccx.ecx, address)?;
        let old_balance = std::mem::replace(&mut account.info.balance, new_balance);
        let record = DealRecord { address, old_balance, new_balance };
        ccx.state.eth_deals.push(record);
        Ok(Default::default())
    }
}

impl Cheatcode for etchCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { target, newRuntimeBytecode } = self;
        ensure_not_precompile!(target, ccx);
        ccx.ecx.load_account(*target)?;
        let bytecode = Bytecode::new_raw(Bytes::copy_from_slice(newRuntimeBytecode));
        ccx.ecx.journaled_state.set_code(*target, bytecode);
        Ok(Default::default())
    }
}

impl Cheatcode for resetNonceCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
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
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
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
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { account, newNonce } = *self;
        let account = journaled_account(ccx.ecx, account)?;
        account.info.nonce = newNonce;
        Ok(Default::default())
    }
}

impl Cheatcode for storeCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { target, slot, value } = *self;
        ensure_not_precompile!(&target, ccx);
        // ensure the account is touched
        let _ = journaled_account(ccx.ecx, target)?;
        ccx.ecx.sstore(target, slot.into(), value.into())?;
        Ok(Default::default())
    }
}

impl Cheatcode for coolCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { target } = self;
        if let Some(account) = ccx.ecx.journaled_state.state.get_mut(target) {
            account.unmark_touch();
            account.storage.clear();
        }
        Ok(Default::default())
    }
}

impl Cheatcode for readCallersCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        read_callers(ccx.state, &ccx.ecx.env.tx.caller)
    }
}

// Deprecated in favor of `snapshotStateCall`
impl Cheatcode for snapshotCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

impl Cheatcode for snapshotStateCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        inner_snapshot_state(ccx)
    }
}

// Deprecated in favor of `revertToStateCall`
impl Cheatcode for revertToCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state(ccx, *snapshotId)
    }
}

// Deprecated in favor of `revertToStateAndDeleteCall`
impl Cheatcode for revertToAndDeleteCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

impl Cheatcode for revertToStateAndDeleteCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_revert_to_state_and_delete(ccx, *snapshotId)
    }
}

// Deprecated in favor of `deleteStateSnapshotCall`
impl Cheatcode for deleteSnapshotCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_delete_state_snapshot(ccx, *snapshotId)
    }
}

impl Cheatcode for deleteStateSnapshotCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { snapshotId } = self;
        inner_delete_state_snapshot(ccx, *snapshotId)
    }
}

// Deprecated in favor of `deleteStateSnapshotsCall`
impl Cheatcode for deleteSnapshotsCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        inner_delete_state_snapshots(ccx)
    }
}

impl Cheatcode for deleteStateSnapshotsCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        inner_delete_state_snapshots(ccx)
    }
}

impl Cheatcode for startStateDiffRecordingCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.recorded_account_diffs_stack = Some(Default::default());
        Ok(Default::default())
    }
}

impl Cheatcode for stopAndReturnStateDiffCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        get_state_diff(state)
    }
}

impl Cheatcode for broadcastRawTransactionCall {
    fn apply_full<DB: DatabaseExt, E: CheatcodesExecutor>(
        &self,
        ccx: &mut CheatsCtxt<DB>,
        executor: &mut E,
    ) -> Result {
        let mut data = self.data.as_ref();
        let tx = TxEnvelope::decode(&mut data)
            .map_err(|err| fmt_err!("failed to decode RLP-encoded transaction: {err}"))?;

        ccx.ecx.db.transact_from_tx(
            tx.clone().into(),
            &ccx.ecx.env,
            &mut ccx.ecx.journaled_state,
            &mut executor.get_inspector(ccx.state),
        )?;

        if ccx.state.broadcast.is_some() {
            ccx.state.broadcastable_transactions.push_back(BroadcastableTransaction {
                rpc: ccx.db.active_fork_url(),
                transaction: tx.try_into()?,
            });
        }

        Ok(Default::default())
    }
}

impl Cheatcode for setBlockhashCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { blockNumber, blockHash } = *self;
        ensure!(
            blockNumber <= ccx.ecx.env.block.number,
            "block number must be less than or equal to the current block number"
        );

        ccx.ecx.db.set_blockhash(blockNumber, blockHash);

        Ok(Default::default())
    }
}

pub(super) fn get_nonce<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>, address: &Address) -> Result {
    let account = ccx.ecx.journaled_state.load_account(*address, &mut ccx.ecx.db)?;
    Ok(account.info.nonce.abi_encode())
}

fn inner_snapshot_state<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>) -> Result {
    Ok(ccx.ecx.db.snapshot_state(&ccx.ecx.journaled_state, &ccx.ecx.env).abi_encode())
}

fn inner_revert_to_state<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>, snapshot_id: U256) -> Result {
    let result = if let Some(journaled_state) = ccx.ecx.db.revert_state(
        snapshot_id,
        &ccx.ecx.journaled_state,
        &mut ccx.ecx.env,
        RevertStateSnapshotAction::RevertKeep,
    ) {
        // we reset the evm's journaled_state to the state of the snapshot previous state
        ccx.ecx.journaled_state = journaled_state;
        true
    } else {
        false
    };
    Ok(result.abi_encode())
}

fn inner_revert_to_state_and_delete<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    snapshot_id: U256,
) -> Result {
    let result = if let Some(journaled_state) = ccx.ecx.db.revert_state(
        snapshot_id,
        &ccx.ecx.journaled_state,
        &mut ccx.ecx.env,
        RevertStateSnapshotAction::RevertRemove,
    ) {
        // we reset the evm's journaled_state to the state of the snapshot previous state
        ccx.ecx.journaled_state = journaled_state;
        true
    } else {
        false
    };
    Ok(result.abi_encode())
}

fn inner_delete_state_snapshot<DB: DatabaseExt>(
    ccx: &mut CheatsCtxt<DB>,
    snapshot_id: U256,
) -> Result {
    let result = ccx.ecx.db.delete_state_snapshot(snapshot_id);
    Ok(result.abi_encode())
}

fn inner_delete_state_snapshots<DB: DatabaseExt>(ccx: &mut CheatsCtxt<DB>) -> Result {
    ccx.ecx.db.delete_state_snapshots();
    Ok(Default::default())
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
fn read_callers(state: &Cheatcodes, default_sender: &Address) -> Result {
    let Cheatcodes { prank, broadcast, .. } = state;

    let mut mode = CallerMode::None;
    let mut new_caller = default_sender;
    let mut new_origin = default_sender;
    if let Some(prank) = prank {
        mode = if prank.single_call { CallerMode::Prank } else { CallerMode::RecurrentPrank };
        new_caller = &prank.new_caller;
        if let Some(new) = &prank.new_origin {
            new_origin = new;
        }
    } else if let Some(broadcast) = broadcast {
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
pub(super) fn journaled_account<DB: DatabaseExt>(
    ecx: &mut InnerEvmContext<DB>,
    addr: Address,
) -> Result<&mut Account> {
    ecx.load_account(addr)?;
    ecx.journaled_state.touch(&addr);
    Ok(ecx.journaled_state.state.get_mut(&addr).expect("account is loaded"))
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
