//! Base block preparation and transaction execution for the in-memory backend.

use super::Backend;
use crate::eth::{
    backend::{
        db::{AnvilCacheDB, Db},
        executor::{BlockExecutionKind, ExecutedPoolTransactions},
        time::{PendingBlockTimestamp, TimeManager},
    },
    error::BlockchainError,
    pool::transactions::PoolTransaction,
};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_evm::{Database, EthEvmFactory, Evm, EvmEnv, EvmFactory};
use alloy_network::Network;
use alloy_primitives::{Address, B256, TxKind, U256};
use anvil_core::eth::transaction::PendingTransaction;
use base_common_chains::ChainConfig;
use base_common_consensus::Predeploys;
use base_common_evm::{
    BaseContext, BaseEvmFactory, BaseHaltReason, BaseSpecId, BaseTime, BaseTransaction,
    BaseUpgrade, L1BlockInfo,
};
use base_common_genesis::RollupConfig;
use base_common_rpc_types::EIP8130_PRE_ZENITH_RPC_ERROR;
use base_consensus_upgrades::Jovian;
use base_protocol::{BaseTimeUpdateTx, L1BlockInfoJovian, L1BlockInfoTx};
use foundry_evm::{backend::DatabaseError, hardfork::FoundryHardfork};
use foundry_primitives::FoundryTxEnvelope;
use revm::{
    DatabaseRef, Inspector,
    context::{
        TxEnv,
        result::{ExecutionResult, HaltReason, Output, ResultAndState},
    },
    database::EmptyDB,
    database_interface::WrapDatabaseRef,
};
use std::sync::Arc;

impl<N: Network> Backend<N> {
    /// Base path of [`Backend::transact_call_with_inspector_ref`].
    pub(super) fn transact_base_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx: BaseTransaction<TxEnv>,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<BaseContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let upgrade = self.base_upgrade_at_timestamp(evm_env.block_env.timestamp.saturating_to());
        if tx.eip8130.is_some() && upgrade < BaseUpgrade::Zenith {
            return Err(BlockchainError::InvalidTransactionRequest(
                EIP8130_PRE_ZENITH_RPC_ERROR.to_string(),
            ));
        }
        let base_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(BaseSpecId::new(upgrade)),
            evm_env.block_env.clone(),
        );
        let activation_admin = self.base_activation_admin().or_else(|| {
            ChainConfig::activation_admin_address_for_upgrade_by_chain_id(
                base_env.cfg_env.chain_id,
                upgrade,
            )
        });
        let factory = BaseEvmFactory::new(activation_admin);
        let mut evm = factory.create_evm_with_inspector(WrapDatabaseRef(db), base_env, inspector);
        evm.ctx_mut().cfg.tx_chain_id_check = true;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);
        let result = Evm::transact_raw(&mut evm, tx)?;
        Ok(ResultAndState {
            result: result.result.map_haltreason(|halt| match halt {
                BaseHaltReason::Base(eth) => eth,
                BaseHaltReason::FailedDeposit => HaltReason::PrecompileError,
            }),
            state: result.state,
        })
    }

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
        let mut candidate_db = AnvilCacheDB::new(db, *evm_env.spec_id());
        self.execute_with_block_executor(
            &mut candidate_db,
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
        .and_then(|(executed, _)| validate_system_transactions(&transactions, &executed))
        .map_err(|err| {
            DatabaseError::AnyRequest(Arc::new(eyre::eyre!(
                "Denim system deposits fail on this fork; its L1Block likely predates Jovian (fork \
                 at or after the Jovian upgrade, or use an earlier --hardfork): {err}"
            )))
        })
    }
}

/// Advances the persisted BaseTime phase, independently of the absolute block number.
fn next_millis_part(db: &dyn DatabaseRef<Error = DatabaseError>) -> Result<u16, DatabaseError> {
    let parent = BaseTime::fetch_timestamp_millis_part(&mut WrapDatabaseRef(db))?;
    if !BaseTimeUpdateTx::is_valid_timestamp_millis_part(parent) {
        return Err(DatabaseError::AnyRequest(Arc::new(eyre::eyre!(
            "invalid parent BaseTime millisecond component: {parent}"
        ))));
    }
    Ok(parent + BaseTimeUpdateTx::BLOCK_INTERVAL_MILLIS)
}

/// Prepares the whole-second carry and explicit time controls for a Denim child block.
pub(super) fn prepare_block_timestamp(
    db: &dyn DatabaseRef<Error = DatabaseError>,
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
    db: &dyn DatabaseRef<Error = DatabaseError>,
    block_number: u64,
    timestamp: u64,
    parent_hash: B256,
    fork: bool,
) -> Result<Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>, DatabaseError> {
    let mut state = WrapDatabaseRef(db);
    let fees = L1BlockInfo::try_fetch(
        &mut state,
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
            sequence.checked_add(1).ok_or_else(|| {
                DatabaseError::AnyRequest(Arc::new(eyre::eyre!("L1-info sequence number overflow")))
            })?,
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
/// Execute the pinned upgrade's constructor to obtain its runtime instead of carrying a second
/// bytecode artifact. Existing fork deployments and their storage are left intact.
pub(super) fn ensure_l1_block_predeploy(db: &mut dyn Db) -> Result<(), DatabaseError> {
    if db.basic(Predeploys::L1_BLOCK_INFO)?.is_some_and(|info| !info.is_empty_code_hash()) {
        return Ok(());
    }
    let mut evm = EthEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());
    let result = evm
        .transact_raw(TxEnv {
            kind: TxKind::Create,
            gas_limit: 1_000_000,
            data: Jovian::l1_block_deployment_bytecode(),
            ..Default::default()
        })
        .map_err(|err| DatabaseError::AnyRequest(Arc::new(eyre::eyre!(err))))?;
    let ExecutionResult::Success { output: Output::Create(code, _), .. } = result.result else {
        return Err(DatabaseError::AnyRequest(Arc::new(eyre::eyre!(
            "failed to initialize the L1Block runtime"
        ))));
    };
    db.set_code(Predeploys::L1_BLOCK_INFO, code)
}

/// Rejects a candidate whose mandatory prefix was skipped or reverted during execution.
pub(super) fn validate_system_transactions(
    expected: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
    executed: &ExecutedPoolTransactions<FoundryTxEnvelope>,
) -> Result<(), BlockchainError> {
    for (index, transaction) in expected.iter().enumerate() {
        if executed.included.get(index).is_none_or(|included| included.hash() != transaction.hash())
            || executed.tx_info.get(index).is_none_or(|info| !info.exit.is_ok())
        {
            return Err(BlockchainError::Internal(format!(
                "required Base system deposit at transaction index {index} failed"
            )));
        }
    }
    Ok(())
}
