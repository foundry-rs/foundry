//! Base block preparation and protocol state for the in-memory backend.

use super::Backend;
use crate::eth::{
    backend::{
        db::{DatabaseRef, Db, EmptyDb},
        env::{CfgEnv, EvmEnv},
        evm::{EthEvmNetwork, EvmOptions, PreparedTx, TxContext, build_evm, transact},
        executor::{BlockExecutionKind, ExecutedPoolTransactions},
        time::{PendingBlockTimestamp, TimeManager},
    },
    error::BlockchainError,
    pool::transactions::PoolTransaction,
};
use alloy_consensus::{Signed, TxLegacy};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_network::Network;
use alloy_primitives::{
    Address, B256, Bytes, KECCAK256_EMPTY, Signature, TxKind, U256, address, b256, hex, uint,
};
use anvil_core::eth::transaction::PendingTransaction;
use base_common_consensus::Predeploys;
use base_common_genesis::RollupConfig;
use base_consensus_upgrades::Jovian;
use base_evm2::{BaseSpecId, BaseUpgrade, L1BlockInfo};
use base_protocol::{BaseTimeUpdateTx, L1BlockInfoJovian, L1BlockInfoTx};
use evm2::bytecode::Bytecode;
use foundry_evm_core::backend::DatabaseError;
use foundry_evm_hardforks::FoundryHardfork;
use foundry_primitives::FoundryTxEnvelope;
use std::sync::Arc;

/// The `BaseTime` predeploy's implementation code address.
const BASE_TIME_IMPLEMENTATION_ADDRESS: Address =
    address!("0xc0D3C0d3C0d3C0D3c0d3C0d3c0D3C0d3c0d30030");

/// The EIP-1967 proxy implementation slot.
const PROXY_IMPLEMENTATION_SLOT: U256 =
    uint!(0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc_U256);

/// The EIP-1967 proxy admin slot.
const PROXY_ADMIN_SLOT: U256 =
    uint!(0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103_U256);

/// The slot of `BaseTime` holding the packed millisecond component.
const BASE_TIME_MILLIS_PART_SLOT: U256 = U256::ZERO;

/// Byte offset of the packed `uint16` millisecond component within its slot.
const BASE_TIME_MILLIS_PART_OFFSET: usize = 30;

/// The Canyon create2 deployer address.
const CREATE_2_DEPLOYER_ADDRESS: Address = address!("0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2");

/// The code hash of the Canyon create2 deployer.
const CREATE_2_DEPLOYER_CODE_HASH: B256 =
    b256!("0xb0550b5b431e30d38000efb7107aaa0ade03d48a7198a140edda9d27134468b2");

/// The EIP-8130 nonce manager system account, which Zenith makes non-empty with a code stub.
const EIP8130_NONCE_MANAGER_ADDRESS: Address =
    address!("0x813000000000000000000000000000000000aa01");

/// The one-byte EIP-3541 reserved code stub planted on code-less EIP-8130 system accounts.
const EIP8130_SYSTEM_ACCOUNT_STUB: [u8; 1] = [0xEF];

/// Canonical `BaseTime` runtime bytecode from `base/contracts` commit `4848ec70`.
const BASE_TIME_IMPLEMENTATION_BYTECODE: &str = include_str!("bytecode/base-time.hex");

/// Canonical predeploy proxy runtime bytecode from `base/contracts` commit `4848ec70`.
const PROXY_BYTECODE: &str = include_str!("bytecode/proxy.hex");

/// Canonical Canyon create2 deployer runtime bytecode.
const CREATE_2_DEPLOYER_BYTECODE: &str = include_str!("bytecode/create2-deployer.hex");

fn decode_artifact(artifact: &str) -> Bytes {
    hex::decode(artifact.trim()).expect("bundled Base bytecode artifacts are valid hex").into()
}

fn db_err(err: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::AnyRequest(Arc::new(eyre::eyre!("{err}")))
}

impl<N: Network> Backend<N> {
    /// Rejects a fork whose protocol contracts would reject every Denim block's system deposits.
    ///
    /// The deposits run through the block executor against the fork state, so startup and resets
    /// fail with a clear error instead of producing a node that cannot mine.
    pub(super) fn ensure_fork_accepts_system_transactions(
        &self,
        db: &dyn Db,
        parent_env: &EvmEnv,
        parent_hash: B256,
        hardfork: FoundryHardfork,
    ) -> Result<(), DatabaseError> {
        let mut evm_env = parent_env.clone();
        evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::from(1));
        let transactions = system_transactions(
            db,
            evm_env.block_env.number.saturating_to(),
            evm_env.block_env.timestamp.saturating_to(),
            parent_hash,
            true,
        )?;
        self.execute_with_block_executor(
            db,
            &evm_env,
            parent_hash,
            hardfork,
            Some(B256::ZERO),
            BlockExecutionKind::Complete,
            &transactions,
            &self.pool_tx_gas_config(&evm_env),
            &self.inspector_tx_config(),
            &|_, _| Ok(()),
        )
        .and_then(|executed| validate_system_transactions(&transactions, &executed.pool))
        .map_err(|err| {
            db_err(format!(
                "Denim system deposits fail on this fork; its L1Block likely predates Jovian (fork \
                 at or after the Jovian upgrade, or use an earlier --hardfork): {err}"
            ))
        })
    }
}

/// Advances the persisted BaseTime phase, independently of the absolute block number.
fn next_millis_part(db: &dyn DatabaseRef) -> Result<u16, DatabaseError> {
    let slot =
        db.storage_ref(Predeploys::BASE_TIME, BASE_TIME_MILLIS_PART_SLOT)?.to_be_bytes::<32>();
    let parent = u16::from_be_bytes([
        slot[BASE_TIME_MILLIS_PART_OFFSET],
        slot[BASE_TIME_MILLIS_PART_OFFSET + 1],
    ]);
    if !BaseTimeUpdateTx::is_valid_timestamp_millis_part(parent) {
        return Err(db_err(format!("invalid parent BaseTime millisecond component: {parent}")));
    }
    Ok(parent + BaseTimeUpdateTx::BLOCK_INTERVAL_MILLIS)
}

/// Prepares the whole-second carry and explicit time controls for a Denim child block.
pub(super) fn prepare_block_timestamp(
    db: &dyn DatabaseRef,
    time: &TimeManager,
    parent_timestamp: u64,
) -> Result<PendingBlockTimestamp, DatabaseError> {
    let carry = u64::from(next_millis_part(db)? >= 1_000);
    Ok(time.prepare_next_timestamp_with_increment(carry, parent_timestamp.saturating_add(carry)))
}

/// Builds the L1-info and BaseTime deposits from the candidate's parent database.
///
/// Fork continuation retains the L1 origin and fee configuration, advancing only its sequence.
/// Standalone blocks use a deterministic synthetic origin because Anvil does not derive from L1.
pub(super) fn system_transactions(
    db: &dyn DatabaseRef,
    block_number: u64,
    timestamp: u64,
    parent_hash: B256,
    fork: bool,
) -> Result<Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>, DatabaseError> {
    let fees = L1BlockInfo::try_fetch_with(
        |slot| db.storage_ref(Predeploys::L1_BLOCK_INFO, slot),
        U256::from(block_number),
        BaseSpecId::new(BaseUpgrade::Denim),
    )?;
    let origin = db.storage_ref(Predeploys::L1_BLOCK_INFO, U256::ZERO)?.to_be_bytes::<32>();
    let sequence_slot =
        db.storage_ref(Predeploys::L1_BLOCK_INFO, U256::from(3))?.to_be_bytes::<32>();
    let sequence = u64::from_be_bytes(sequence_slot[24..32].try_into().unwrap());
    let batcher = db.storage_ref(Predeploys::L1_BLOCK_INFO, U256::from(4))?;
    let (number, origin_timestamp, origin_hash, sequence) = if fork {
        (
            u64::from_be_bytes(origin[24..32].try_into().unwrap()),
            u64::from_be_bytes(origin[16..24].try_into().unwrap()),
            B256::from(db.storage_ref(Predeploys::L1_BLOCK_INFO, U256::from(2))?),
            sequence.checked_add(1).ok_or_else(|| db_err("L1-info sequence number overflow"))?,
        )
    } else {
        (block_number, timestamp, parent_hash, block_number)
    };
    let l1_info = L1BlockInfoTx::Jovian(L1BlockInfoJovian::new(
        number,
        origin_timestamp,
        fees.l1_base_fee.saturating_to(),
        origin_hash,
        sequence,
        Address::from_word(B256::from(batcher)),
        fees.l1_blob_base_fee.unwrap_or_default().saturating_to(),
        fees.l1_blob_base_fee_scalar.unwrap_or_default().saturating_to(),
        fees.l1_base_fee_scalar.saturating_to(),
        fees.operator_fee_scalar.unwrap_or_default().saturating_to(),
        fees.operator_fee_constant.unwrap_or_default().saturating_to(),
        fees.da_footprint_gas_scalar.unwrap_or_default(),
    ));
    let mut upgrades = RollupConfig::default();
    upgrades.set_upgrade_activation_timestamp(BaseUpgrade::Regolith, 0);
    let l1_info = l1_info.into_deposit_tx(&upgrades, timestamp);
    let base_time = BaseTimeUpdateTx::new(next_millis_part(db)? % 1_000)
        .expect("the parent BaseTime phase was validated")
        .into_deposit_tx(block_number);
    Ok([l1_info, base_time]
        .into_iter()
        .map(|deposit| {
            let envelope = FoundryTxEnvelope::decode_2718(&mut deposit.encoded_2718().as_slice())
                .expect("Base and OP deposits share the canonical encoding");
            let pending = PendingTransaction::new(envelope).expect("deposit sender is explicit");
            Arc::new(PoolTransaction::new(pending))
        })
        .collect())
}

/// Installs the canonical L1Block runtime when a local chain has no deployment.
///
/// Executes the pinned upgrade's constructor to obtain its runtime instead of carrying a second
/// bytecode artifact. Existing fork deployments and their storage are left intact.
pub(super) fn ensure_l1_block_predeploy(db: &mut dyn Db) -> Result<(), DatabaseError> {
    if db
        .basic_ref(Predeploys::L1_BLOCK_INFO)?
        .is_some_and(|info| info.code_hash != KECCAK256_EMPTY)
    {
        return Ok(());
    }
    let create = TxLegacy {
        gas_limit: 1_000_000,
        to: TxKind::Create,
        input: Jovian::l1_block_deployment_bytecode(),
        ..Default::default()
    };
    let tx = PreparedTx::simulated(
        FoundryTxEnvelope::Legacy(Signed::new_unhashed(create, Signature::test_signature())),
        Address::ZERO,
    );
    let env = EvmEnv {
        cfg_env: CfgEnv { disable_base_fee: true, ..Default::default() },
        ..Default::default()
    };
    let mut evm =
        build_evm::<EthEvmNetwork>(&EmptyDb, &env, &EvmOptions::default()).map_err(db_err)?;
    let executed =
        transact::<EthEvmNetwork>(&mut evm, &tx, &TxContext::default(), None).map_err(db_err)?;
    let runtime = executed
        .result
        .created_address
        .filter(|_| executed.result.status)
        .and_then(|address| executed.state.account_info(&address))
        .and_then(|info| info.code.clone())
        .ok_or_else(|| db_err("failed to initialize the L1Block runtime"))?;
    db.set_code(Predeploys::L1_BLOCK_INFO, runtime.original_bytes())
}

/// Installs the BaseTime proxy and links its Denim implementation.
///
/// Standalone genesis owns the proxy admin and always resets it. A fork must not rewrite an
/// inherited deployment, so the proxy code and admin are only seeded when the account has no code.
/// An existing implementation link is preserved so governance upgrades are not undone.
pub(super) fn ensure_base_time_predeploy(
    db: &mut dyn Db,
    reset_admin: bool,
) -> Result<(), DatabaseError> {
    ensure_l1_block_predeploy(db)?;
    let mut proxy = db.basic_ref(Predeploys::BASE_TIME)?.unwrap_or_default();
    let missing_code = proxy.code_hash == KECCAK256_EMPTY;
    if missing_code {
        proxy.set_code(Bytecode::new_raw(decode_artifact(PROXY_BYTECODE)));
        db.insert_account(Predeploys::BASE_TIME, proxy);
    }
    let proxy_admin = U256::from_be_slice(Predeploys::PROXY_ADMIN.as_slice());
    if reset_admin || missing_code {
        db.set_storage_at(Predeploys::BASE_TIME, PROXY_ADMIN_SLOT.into(), proxy_admin.into())?;
    }

    if !db.storage_ref(Predeploys::BASE_TIME, PROXY_IMPLEMENTATION_SLOT)?.is_zero() {
        return Ok(());
    }
    let admin = db.storage_ref(Predeploys::BASE_TIME, PROXY_ADMIN_SLOT)?;
    if admin != proxy_admin {
        return Err(db_err(format!(
            "BaseTime activation requires the canonical proxy admin, found {admin:#x}"
        )));
    }
    let implementation = decode_artifact(BASE_TIME_IMPLEMENTATION_BYTECODE);
    let mut info = db.basic_ref(BASE_TIME_IMPLEMENTATION_ADDRESS)?.unwrap_or_default();
    info.set_code(Bytecode::new_raw(implementation));
    db.insert_account(BASE_TIME_IMPLEMENTATION_ADDRESS, info);
    db.set_storage_at(
        Predeploys::BASE_TIME,
        PROXY_IMPLEMENTATION_SLOT.into(),
        U256::from_be_slice(BASE_TIME_IMPLEMENTATION_ADDRESS.as_slice()).into(),
    )
}

/// Force-deploys the Canyon create2 deployer, preserving the account's balance and nonce.
pub(super) fn ensure_create2_deployer(db: &mut dyn Db) -> Result<(), DatabaseError> {
    let mut info = db.basic_ref(CREATE_2_DEPLOYER_ADDRESS)?.unwrap_or_default();
    info.code_hash = CREATE_2_DEPLOYER_CODE_HASH;
    info.code = Some(Bytecode::new_raw(decode_artifact(CREATE_2_DEPLOYER_BYTECODE)));
    db.insert_account(CREATE_2_DEPLOYER_ADDRESS, info);
    Ok(())
}

/// Plants Zenith's code stub on code-less EIP-8130 system accounts so state clearing keeps them.
pub(super) fn ensure_eip8130_system_accounts(db: &mut dyn Db) -> Result<(), DatabaseError> {
    let mut info = db.basic_ref(EIP8130_NONCE_MANAGER_ADDRESS)?.unwrap_or_default();
    if info.code_hash != KECCAK256_EMPTY {
        return Ok(());
    }
    info.set_code(Bytecode::new_legacy(Bytes::from_static(&EIP8130_SYSTEM_ACCOUNT_STUB)));
    db.insert_account(EIP8130_NONCE_MANAGER_ADDRESS, info);
    Ok(())
}

/// Rejects a candidate whose mandatory prefix was skipped or reverted during execution.
pub(super) fn validate_system_transactions(
    expected: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
    executed: &ExecutedPoolTransactions<FoundryTxEnvelope>,
) -> Result<(), BlockchainError> {
    for (index, transaction) in expected.iter().enumerate() {
        if executed.included.get(index).is_none_or(|included| included.hash() != transaction.hash())
            || executed.tx_info.get(index).is_none_or(|info| !info.exit.is_success())
        {
            return Err(BlockchainError::Internal(format!(
                "required Base system deposit at transaction index {index} failed"
            )));
        }
    }
    Ok(())
}

/// Upper bound on a Base EIP-8130 payer's debit: `(gas_limit + payer_auth_cost) * max_fee`.
///
/// Payer authentication is metered on top of the sender-signed gas limit, so the payer reimburses
/// it as well. Matches Base's `FeeCheck::max_fee_charge`.
pub(crate) fn eip8130_max_fee_charge(
    gas_limit: u64,
    payer_auth_cost: u64,
    max_fee_per_gas: u128,
) -> U256 {
    U256::from(gas_limit.saturating_add(payer_auth_cost))
        .saturating_mul(U256::from(max_fee_per_gas))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;

    #[test]
    fn bundled_artifacts_match_their_code_hashes() {
        assert_eq!(
            keccak256(decode_artifact(CREATE_2_DEPLOYER_BYTECODE)),
            CREATE_2_DEPLOYER_CODE_HASH
        );
        assert_eq!(
            keccak256(decode_artifact(BASE_TIME_IMPLEMENTATION_BYTECODE)),
            b256!("0x9c4c8a497a69d0b8f2ba67be0bee7a1373186055978c3be6ec3068e0ec27f32a")
        );
        assert_eq!(
            keccak256(decode_artifact(PROXY_BYTECODE)),
            b256!("0xfa8c9db6c6cab7108dea276f4cd09d575674eb0852c0fa3187e59e98ef977998")
        );
    }

    #[test]
    fn l1_block_runtime_is_installed() {
        let mut db = crate::eth::backend::mem::in_memory_db::MemDb::default();
        ensure_l1_block_predeploy(&mut db).unwrap();
        let info = db.basic_ref(Predeploys::L1_BLOCK_INFO).unwrap().unwrap();
        assert_ne!(info.code_hash, KECCAK256_EMPTY);
    }
}
