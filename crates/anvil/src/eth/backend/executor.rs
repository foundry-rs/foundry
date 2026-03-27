use crate::eth::backend::cheats::CheatsManager;
use alloy_consensus::{Eip658Value, Transaction, TransactionEnvelope, transaction::Either};
use alloy_eips::{
    Encodable2718, eip2935, eip4788,
    eip7702::{RecoveredAuthority, RecoveredAuthorization},
};
use alloy_evm::{
    EthEvmFactory, Evm, EvmEnv, EvmFactory, FromRecoveredTx, FromTxWithEncoded, RecoveredTx,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockValidationError,
        ExecutableTx, OnStateHook, StateChangeSource, StateDB, TxResult,
    },
    eth::{
        EthEvmContext, EthTxResult,
        receipt_builder::{ReceiptBuilder, ReceiptBuilderCtx},
    },
    precompiles::PrecompilesMap,
};
use alloy_op_evm::OpEvmFactory;
use alloy_primitives::{Address, B256, Bytes};
use anvil_core::eth::transaction::PendingTransaction;
use foundry_evm::{backend::DatabaseError, core::either_evm::EitherEvm};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxType};
use op_revm::{OpContext, OpTransaction};
use revm::{
    Database, DatabaseCommit, Inspector,
    context::{Block as RevmBlock, TxEnv},
    context_interface::result::ResultAndState,
    primitives::hardfork::SpecId,
};
use std::{fmt, fmt::Debug};

/// Receipt builder for Foundry/Anvil that handles all transaction types
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct FoundryReceiptBuilder;

impl ReceiptBuilder for FoundryReceiptBuilder {
    type Transaction = FoundryTxEnvelope;
    type Receipt = FoundryReceiptEnvelope;

    fn build_receipt<E: Evm>(
        &self,
        ctx: ReceiptBuilderCtx<'_, FoundryTxType, E>,
    ) -> FoundryReceiptEnvelope {
        let receipt = alloy_consensus::Receipt {
            status: Eip658Value::Eip658(ctx.result.is_success()),
            cumulative_gas_used: ctx.cumulative_gas_used,
            logs: ctx.result.into_logs(),
        }
        .with_bloom();

        match ctx.tx_type {
            FoundryTxType::Legacy => FoundryReceiptEnvelope::Legacy(receipt),
            FoundryTxType::Eip2930 => FoundryReceiptEnvelope::Eip2930(receipt),
            FoundryTxType::Eip1559 => FoundryReceiptEnvelope::Eip1559(receipt),
            FoundryTxType::Eip4844 => FoundryReceiptEnvelope::Eip4844(receipt),
            FoundryTxType::Eip7702 => FoundryReceiptEnvelope::Eip7702(receipt),
            FoundryTxType::Deposit => {
                unreachable!("deposit receipts are built in commit_transaction")
            }
            FoundryTxType::Tempo => FoundryReceiptEnvelope::Tempo(receipt),
        }
    }
}

/// Result of executing a transaction in [`AnvilBlockExecutor`].
///
/// Wraps [`EthTxResult`] with the sender address, needed for deposit nonce resolution.
#[derive(Debug)]
pub struct AnvilTxResult<H> {
    pub inner: EthTxResult<H, FoundryTxType>,
    pub sender: Address,
}

impl<H> TxResult for AnvilTxResult<H> {
    type HaltReason = H;

    fn result(&self) -> &ResultAndState<Self::HaltReason> {
        self.inner.result()
    }
}

/// Block executor for Anvil that implements [`BlockExecutor`].
///
/// Wraps an EVM instance and produces [`FoundryReceiptEnvelope`] receipts.
/// Validation (gas limits, blob gas, transaction validity) is handled by the
/// caller before transactions are fed to this executor.
pub struct AnvilBlockExecutor<E> {
    /// The EVM instance used for execution.
    evm: E,
    /// Parent block hash — needed for EIP-2935 system call.
    parent_hash: B256,
    /// The active spec id, used to gate hardfork-specific behavior.
    spec_id: SpecId,
    /// Receipt builder.
    receipt_builder: FoundryReceiptBuilder,
    /// Receipts of executed transactions.
    receipts: Vec<FoundryReceiptEnvelope>,
    /// Total gas used by transactions in this block.
    gas_used: u64,
    /// Blob gas used by the block.
    blob_gas_used: u64,
    /// Optional state change hook.
    state_hook: Option<Box<dyn OnStateHook>>,
}

impl<E: fmt::Debug> fmt::Debug for AnvilBlockExecutor<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilBlockExecutor")
            .field("evm", &self.evm)
            .field("parent_hash", &self.parent_hash)
            .field("spec_id", &self.spec_id)
            .field("gas_used", &self.gas_used)
            .field("blob_gas_used", &self.blob_gas_used)
            .field("receipts", &self.receipts.len())
            .finish_non_exhaustive()
    }
}

impl<E> AnvilBlockExecutor<E> {
    /// Creates a new [`AnvilBlockExecutor`].
    pub fn new(evm: E, parent_hash: B256, spec_id: SpecId) -> Self {
        Self {
            evm,
            parent_hash,
            spec_id,
            receipt_builder: FoundryReceiptBuilder,
            receipts: Vec::new(),
            gas_used: 0,
            blob_gas_used: 0,
            state_hook: None,
        }
    }
}

impl<E> BlockExecutor for AnvilBlockExecutor<E>
where
    E: Evm<
            DB: StateDB,
            Tx: FromRecoveredTx<FoundryTxEnvelope> + FromTxWithEncoded<FoundryTxEnvelope>,
        >,
{
    type Transaction = FoundryTxEnvelope;
    type Receipt = FoundryReceiptEnvelope;
    type Evm = E;
    type Result = AnvilTxResult<E::HaltReason>;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        // EIP-2935: store parent block hash in history storage contract.
        if self.spec_id >= SpecId::PRAGUE {
            let result = self
                .evm
                .transact_system_call(
                    eip4788::SYSTEM_ADDRESS,
                    eip2935::HISTORY_STORAGE_ADDRESS,
                    Bytes::copy_from_slice(self.parent_hash.as_slice()),
                )
                .map_err(BlockExecutionError::other)?;

            if let Some(hook) = &mut self.state_hook {
                hook.on_state(
                    StateChangeSource::PreBlock(
                        alloy_evm::block::StateChangePreBlockSource::BlockHashesContract,
                    ),
                    &result.state,
                );
            }
            self.evm.db_mut().commit(result.state);
        }
        Ok(())
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        let (tx_env, tx) = tx.into_parts();

        let block_available_gas = self.evm.block().gas_limit() - self.gas_used;
        if tx.tx().gas_limit() > block_available_gas {
            return Err(BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
                transaction_gas_limit: tx.tx().gas_limit(),
                block_available_gas,
            }
            .into());
        }

        let sender = *tx.signer();

        let result = self.evm.transact(tx_env).map_err(|err| {
            let hash = tx.tx().trie_hash();
            BlockExecutionError::evm(err, hash)
        })?;

        Ok(AnvilTxResult {
            inner: EthTxResult {
                result,
                blob_gas_used: tx.tx().blob_gas_used().unwrap_or_default(),
                tx_type: tx.tx().tx_type(),
            },
            sender,
        })
    }

    fn commit_transaction(&mut self, output: Self::Result) -> Result<u64, BlockExecutionError> {
        let AnvilTxResult {
            inner: EthTxResult { result: ResultAndState { result, state }, blob_gas_used, tx_type },
            sender,
        } = output;

        if let Some(hook) = &mut self.state_hook {
            hook.on_state(StateChangeSource::Transaction(self.receipts.len()), &state);
        }

        let gas_used = result.gas_used();
        self.gas_used += gas_used;

        if self.spec_id >= SpecId::CANCUN {
            self.blob_gas_used = self.blob_gas_used.saturating_add(blob_gas_used);
        }

        let receipt = if tx_type == FoundryTxType::Deposit {
            let deposit_nonce = state.get(&sender).map(|acc| acc.info.nonce);
            let receipt = alloy_consensus::Receipt {
                status: Eip658Value::Eip658(result.is_success()),
                cumulative_gas_used: self.gas_used,
                logs: result.into_logs(),
            }
            .with_bloom();
            FoundryReceiptEnvelope::Deposit(op_alloy_consensus::OpDepositReceiptWithBloom {
                receipt: op_alloy_consensus::OpDepositReceipt {
                    inner: receipt.receipt,
                    deposit_nonce,
                    deposit_receipt_version: deposit_nonce.map(|_| 1),
                },
                logs_bloom: receipt.logs_bloom,
            })
        } else {
            self.receipt_builder.build_receipt(ReceiptBuilderCtx {
                tx_type,
                evm: &self.evm,
                result,
                state: &state,
                cumulative_gas_used: self.gas_used,
            })
        };

        self.receipts.push(receipt);
        self.evm.db_mut().commit(state);

        Ok(gas_used)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<FoundryReceiptEnvelope>), BlockExecutionError>
    {
        Ok((
            self.evm,
            BlockExecutionResult {
                receipts: self.receipts,
                requests: Default::default(),
                gas_used: self.gas_used,
                blob_gas_used: self.blob_gas_used,
            },
        ))
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.state_hook = hook;
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        &mut self.evm
    }

    fn evm(&self) -> &Self::Evm {
        &self.evm
    }

    fn receipts(&self) -> &[FoundryReceiptEnvelope] {
        &self.receipts
    }
}

/// Builds the per-tx `OpTransaction<TxEnv>` from a pending transaction, replicating the logic
/// from `TransactionExecutor::env_for`.
pub fn build_tx_env_for_pending(
    tx: &PendingTransaction<FoundryTxEnvelope>,
    cheats: &CheatsManager,
    is_optimism: bool,
) -> OpTransaction<TxEnv> {
    let mut tx_env: OpTransaction<TxEnv> =
        FromRecoveredTx::from_recovered_tx(tx.transaction.as_ref(), *tx.sender());

    if let FoundryTxEnvelope::Eip7702(tx_7702) = tx.transaction.as_ref()
        && cheats.has_recover_overrides()
    {
        let cheated_auths = tx_7702
            .tx()
            .authorization_list
            .iter()
            .zip(tx_env.base.authorization_list)
            .map(|(signed_auth, either_auth)| {
                either_auth.right_and_then(|recovered_auth| {
                    if recovered_auth.authority().is_none()
                        && let Ok(signature) = signed_auth.signature()
                        && let Some(override_addr) =
                            cheats.get_recover_override(&signature.as_bytes().into())
                    {
                        Either::Right(RecoveredAuthorization::new_unchecked(
                            recovered_auth.into_parts().0,
                            RecoveredAuthority::Valid(override_addr),
                        ))
                    } else {
                        Either::Right(recovered_auth)
                    }
                })
            })
            .collect();
        tx_env.base.authorization_list = cheated_auths;
    }

    if is_optimism {
        tx_env.enveloped_tx = Some(tx.transaction.encoded_2718().into());
    }

    tx_env
}

/// Creates a database with given database and inspector.
pub fn new_eth_evm_with_inspector<DB, I>(
    db: DB,
    evm_env: &EvmEnv,
    inspector: I,
    is_optimism: bool,
) -> EitherEvm<DB, I, PrecompilesMap>
where
    DB: Database<Error = DatabaseError> + Debug,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    if is_optimism {
        let evm_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(op_revm::OpSpecId::ISTHMUS),
            evm_env.block_env.clone(),
        );
        EitherEvm::Op(OpEvmFactory::default().create_evm_with_inspector(db, evm_env, inspector))
    } else {
        EitherEvm::Eth(EthEvmFactory::default().create_evm_with_inspector(
            db,
            evm_env.clone(),
            inspector,
        ))
    }
}
