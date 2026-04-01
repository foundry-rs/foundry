use crate::{
    eth::{
        backend::cheats::CheatsManager, error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    mem::{
        IntoInstructionResult,
        inspector::{AnvilInspector, InspectorTxConfig},
    },
};
use alloy_consensus::{
    Eip658Value, Transaction, TransactionEnvelope, TxReceipt,
    transaction::{Either, Recovered},
};
use alloy_eips::{
    Encodable2718, eip2935, eip4788,
    eip7702::{RecoveredAuthority, RecoveredAuthorization},
};
use alloy_evm::{
    Evm, FromRecoveredTx, FromTxWithEncoded, RecoveredTx,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockValidationError,
        ExecutableTx, OnStateHook, StateChangePreBlockSource, StateChangeSource, StateDB, TxResult,
    },
    eth::{
        EthTxResult,
        receipt_builder::{ReceiptBuilder, ReceiptBuilderCtx},
    },
};
use alloy_primitives::{Address, B256, Bytes};
use anvil_core::eth::transaction::{
    MaybeImpersonatedTransaction, PendingTransaction, TransactionInfo,
};
use foundry_evm::core::env::FoundryTransaction;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxType};
use revm::{
    Database, DatabaseCommit,
    context::Block as RevmBlock,
    context_interface::result::{ExecutionResult, Output, ResultAndState},
    interpreter::InstructionResult,
    primitives::hardfork::SpecId,
    state::AccountInfo,
};
use std::{fmt, fmt::Debug, mem::take, sync::Arc};

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
                    StateChangeSource::PreBlock(StateChangePreBlockSource::BlockHashesContract),
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

/// Result of executing pool transactions against a block executor.
pub struct ExecutedPoolTransactions<T> {
    /// Successfully included transactions.
    pub included: Vec<Arc<PoolTransaction<T>>>,
    /// Transactions that failed validation.
    pub invalid: Vec<Arc<PoolTransaction<T>>>,
    /// Per-transaction execution info.
    pub tx_info: Vec<TransactionInfo>,
    /// The raw pending transactions that were included (in order).
    pub txs: Vec<MaybeImpersonatedTransaction<T>>,
}

/// Gas-related configuration for pool transaction execution.
///
/// Bundles parameters that cannot be derived from the generic `Evm` trait
/// (which doesn't expose `cfg()`), so callers construct this from `EvmEnv`
/// before calling [`execute_pool_transactions`].
pub struct PoolTxGasConfig {
    pub disable_block_gas_limit: bool,
    pub tx_gas_limit_cap: Option<u64>,
    pub tx_gas_limit_cap_resolved: u64,
    pub max_blob_gas_per_block: u64,
    pub is_cancun: bool,
}

/// Executes pool transactions against a block executor, handling validation,
/// execution, commit, inspector drain, and result collection.
///
/// This is the shared core of `do_mine_block` and `with_pending_block`.
#[allow(clippy::type_complexity)]
pub fn execute_pool_transactions<B>(
    executor: &mut B,
    pool_transactions: &[Arc<PoolTransaction<B::Transaction>>],
    gas_config: &PoolTxGasConfig,
    inspector_config: &InspectorTxConfig,
    cheats: &CheatsManager,
    validator: &dyn Fn(
        &PendingTransaction<B::Transaction>,
        &AccountInfo,
    ) -> Result<(), InvalidTransactionError>,
) -> ExecutedPoolTransactions<B::Transaction>
where
    B: BlockExecutor<Evm: Evm<DB: Database + Debug, Inspector = AnvilInspector>>,
    B::Transaction: Transaction + Encodable2718 + Clone,
    B::Receipt: TxReceipt,
    <B::Result as TxResult>::HaltReason: Clone + IntoInstructionResult,
    <B::Evm as Evm>::Tx: FromTxWithEncoded<B::Transaction> + FoundryTransaction,
{
    let gas_limit = executor.evm().block().gas_limit();

    let mut included = Vec::new();
    let mut invalid = Vec::new();
    let mut tx_info: Vec<TransactionInfo> = Vec::new();
    let mut transactions = Vec::new();
    let mut blob_gas_used = 0u64;

    for pool_tx in pool_transactions {
        let pending = &pool_tx.pending_transaction;
        let sender = *pending.sender();

        let account = match executor.evm_mut().db_mut().basic(sender).map(|a| a.unwrap_or_default())
        {
            Ok(acc) => acc,
            Err(err) => {
                trace!(target: "backend", ?err, "db error for tx {:?}, skipping", pool_tx.hash());
                continue;
            }
        };

        let tx_env =
            build_tx_env_for_pending::<B::Transaction, <B::Evm as Evm>::Tx>(pending, cheats);

        // Gas limit checks
        let cumulative_gas =
            executor.receipts().last().map(|r| r.cumulative_gas_used()).unwrap_or(0);
        let max_block_gas = cumulative_gas.saturating_add(pending.transaction.gas_limit());
        if !gas_config.disable_block_gas_limit && max_block_gas > gas_limit {
            trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), ?pool_tx, "block gas limit exhausting, skipping transaction");
            continue;
        }

        // Osaka EIP-7825 tx gas limit cap check
        if gas_config.tx_gas_limit_cap.is_none()
            && pending.transaction.gas_limit() > gas_config.tx_gas_limit_cap_resolved
        {
            trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), ?pool_tx, "transaction gas limit exhausting, skipping transaction");
            continue;
        }

        // Blob gas check
        let tx_blob_gas = pending.transaction.blob_gas_used().unwrap_or(0);
        if blob_gas_used.saturating_add(tx_blob_gas) > gas_config.max_blob_gas_per_block {
            trace!(target: "backend", blob_gas = %tx_blob_gas, ?pool_tx, "block blob gas limit exhausting, skipping transaction");
            continue;
        }

        // Validate
        if let Err(err) = validator(pending, &account) {
            warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", pool_tx.hash(), err);
            invalid.push(pool_tx.clone());
            continue;
        }

        let nonce = account.nonce;

        let recovered = Recovered::new_unchecked(pending.transaction.as_ref().clone(), sender);
        trace!(target: "backend", "[{:?}] executing", pool_tx.hash());
        match executor.execute_transaction_without_commit((tx_env, recovered)) {
            Ok(result) => {
                let exec_result = result.result().result.clone();
                let gas_used = result.result().result.gas_used();

                executor.commit_transaction(result).expect("commit failed");

                let traces =
                    executor.evm_mut().inspector_mut().finish_transaction(inspector_config);

                if gas_config.is_cancun {
                    blob_gas_used = blob_gas_used.saturating_add(tx_blob_gas);
                }

                let (exit_reason, out, _logs) = match exec_result {
                    ExecutionResult::Success { reason, logs, output, .. } => {
                        (reason.into(), Some(output), logs)
                    }
                    ExecutionResult::Revert { output, .. } => {
                        (InstructionResult::Revert, Some(Output::Call(output)), Vec::new())
                    }
                    ExecutionResult::Halt { reason, .. } => {
                        (reason.into_instruction_result(), None, Vec::new())
                    }
                };

                if exit_reason == InstructionResult::OutOfGas {
                    warn!(target: "backend", "[{:?}] executed with out of gas", pool_tx.hash());
                }

                trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", pool_tx.hash(), out);
                trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", pool_tx.hash(), exit_reason, gas_used);

                let contract_address = if pending.transaction.to().is_none() {
                    let addr = sender.create(nonce);
                    trace!(target: "backend", "Contract creation tx: computed address {:?}", addr);
                    Some(addr)
                } else {
                    None
                };

                // TODO: replace `TransactionInfo` with alloy receipt/transaction types
                let transaction_index = tx_info.len() as u64;
                let info = TransactionInfo {
                    transaction_hash: pool_tx.hash(),
                    transaction_index,
                    from: sender,
                    to: pending.transaction.to(),
                    contract_address,
                    traces,
                    exit: exit_reason,
                    out: out.map(Output::into_data),
                    nonce,
                    gas_used,
                };

                included.push(pool_tx.clone());
                tx_info.push(info);
                transactions.push(pending.transaction.clone());
            }
            Err(err) => {
                trace!(target: "backend", ?err, "tx execution error, skipping {:?}", pool_tx.hash());
            }
        }
    }

    ExecutedPoolTransactions { included, invalid, tx_info, txs: transactions }
}

/// Builds the EVM transaction env from a pending pool transaction.
pub fn build_tx_env_for_pending<Tx, T>(tx: &PendingTransaction<Tx>, cheats: &CheatsManager) -> T
where
    Tx: Transaction + Encodable2718,
    T: FromTxWithEncoded<Tx> + FoundryTransaction,
{
    let encoded = tx.transaction.encoded_2718().into();
    let mut tx_env: T =
        FromTxWithEncoded::from_encoded_tx(tx.transaction.as_ref(), *tx.sender(), encoded);

    if let Some(signed_auths) = tx.transaction.authorization_list()
        && cheats.has_recover_overrides()
    {
        let auth_list = tx_env.authorization_list_mut();
        let cheated_auths = signed_auths
            .iter()
            .zip(take(auth_list))
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
        *tx_env.authorization_list_mut() = cheated_auths;
    }

    tx_env
}
