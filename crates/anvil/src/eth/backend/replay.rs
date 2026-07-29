//! Transaction-hash fork replay preparation and execution.

use crate::{
    config::ForkTransactionReplay,
    eth::backend::executor::AnvilBlockExecutor,
    mem::inspector::{AnvilInspector, InspectorTxConfig},
};
use alloy_consensus::{
    Transaction,
    transaction::{Recovered, SignerRecoverable, TxHashRef},
};
use alloy_evm::{
    Evm, FromRecoveredTx, FromTxWithEncoded, RecoveredTx,
    block::{BlockExecutionResult, BlockExecutor, StateDB, TxResult},
};
use alloy_network::{BlockResponse, TransactionResponse};
use anvil_core::eth::transaction::{MaybeImpersonatedTransaction, TransactionInfo};
use eyre::{Context, Result};
use foundry_evm::core::evm::IntoInstructionResult;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope};
use revm::{
    Database,
    context_interface::result::{ExecutionResult, Output},
    interpreter::InstructionResult,
    state::EvmState,
};

/// A source transaction prepared for direct historical execution.
#[derive(Clone, Debug)]
pub(crate) struct HistoricalReplayTransaction {
    pub(crate) transaction: Recovered<FoundryTxEnvelope>,
    pub(crate) source_index: usize,
}

/// The complete result of executing a historical prefix against an overlay.
pub(crate) struct ExecutedHistoricalReplay {
    pub(crate) block_result: BlockExecutionResult<FoundryReceiptEnvelope>,
    pub(crate) transactions: Vec<MaybeImpersonatedTransaction<FoundryTxEnvelope>>,
    pub(crate) transaction_infos: Vec<TransactionInfo>,
    pub(crate) state_changes: Vec<EvmState>,
}

/// Converts and validates every source-prefix transaction before database execution.
pub(crate) fn prepare_fork_transaction_replay(
    replay: ForkTransactionReplay,
) -> Result<Vec<HistoricalReplayTransaction>> {
    let source_hash = replay.source_block.header().hash;
    let source_number = replay.source_block.header().number;
    let source_transactions = replay
        .source_block
        .transactions()
        .as_transactions()
        .expect("full source block validated during resolution");

    source_transactions
        .iter()
        .take(replay.target_index.saturating_add(1))
        .enumerate()
        .map(|(source_index, source_transaction)| {
            let source_transaction_hash = source_transaction.tx_hash();
            let transaction = FoundryTxEnvelope::try_from(source_transaction.clone())
                .wrap_err_with(|| {
                    format!(
                        "failed to convert source transaction {source_transaction_hash} at index \
                         {source_index} in block {source_hash} ({source_number})"
                    )
                })?;
            eyre::ensure!(
                transaction.tx_hash() == &source_transaction_hash,
                "converted source transaction at index {source_index} in block {source_hash} \
                 ({source_number}) changed hash from {source_transaction_hash} to {}",
                transaction.tx_hash()
            );
            let sender = transaction.recover_signer().wrap_err_with(|| {
                format!(
                    "failed to recover sender for source transaction {source_transaction_hash} at \
                     index {source_index} in block {source_hash} ({source_number})"
                )
            })?;
            Ok(HistoricalReplayTransaction {
                transaction: Recovered::new_unchecked(transaction, sender),
                source_index,
            })
        })
        .collect()
}

/// Executes a prepared prefix strictly and captures changesets for deferred publication.
pub(crate) fn execute_historical_replay<E>(
    executor: &mut AnvilBlockExecutor<E>,
    transactions: &[HistoricalReplayTransaction],
    inspector_config: &InspectorTxConfig,
) -> Result<(Vec<MaybeImpersonatedTransaction<FoundryTxEnvelope>>, Vec<TransactionInfo>)>
where
    E: Evm<
            DB: StateDB,
            Inspector = AnvilInspector,
            Tx: FromRecoveredTx<FoundryTxEnvelope> + FromTxWithEncoded<FoundryTxEnvelope>,
        >,
    E::HaltReason: Clone + IntoInstructionResult,
{
    let mut stored_transactions = Vec::with_capacity(transactions.len());
    let mut transaction_infos = Vec::with_capacity(transactions.len());

    for replay in transactions {
        let transaction = replay.transaction.tx();
        let transaction_hash = *transaction.tx_hash();
        let sender = replay.transaction.signer();
        let nonce = executor
            .evm_mut()
            .db_mut()
            .basic(sender)
            .wrap_err_with(|| {
                format!(
                    "database error preparing source transaction {transaction_hash} at index {}",
                    replay.source_index
                )
            })?
            .unwrap_or_default()
            .nonce;

        let result = executor
            .execute_transaction_without_commit(replay.transaction.clone().into_encoded())
            .map_err(|err| {
                eyre::eyre!(
                    "failed to execute source transaction {transaction_hash} at index {}: {err}",
                    replay.source_index
                )
            })?;
        let execution_result = result.result().result.clone();
        let gas_used = execution_result.tx_gas_used();
        executor.commit_transaction(result);

        let traces = executor.evm_mut().inspector_mut().finish_transaction(inspector_config);
        let (exit_reason, out) = match execution_result {
            ExecutionResult::Success { reason, output, .. } => (reason.into(), Some(output)),
            ExecutionResult::Revert { output, .. } => {
                (InstructionResult::Revert, Some(Output::Call(output)))
            }
            ExecutionResult::Halt { reason, .. } => (reason.into_instruction_result(), None),
        };
        let contract_address = transaction.to().is_none().then(|| sender.create(nonce));
        transaction_infos.push(TransactionInfo {
            transaction_hash,
            transaction_index: replay.source_index as u64,
            from: sender,
            to: transaction.to(),
            contract_address,
            traces,
            exit: exit_reason,
            out: out.map(Output::into_data),
            nonce,
            gas_used,
        });
        stored_transactions.push(MaybeImpersonatedTransaction::new(transaction.clone()));
    }

    Ok((stored_transactions, transaction_infos))
}
