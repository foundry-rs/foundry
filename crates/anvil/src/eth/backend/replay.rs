//! Transaction-hash fork replay preparation and execution.

use crate::{
    config::ForkTransactionReplay,
    eth::backend::executor::AnvilBlockExecutor,
    mem::inspector::{AnvilInspector, InspectorTxConfig},
};
use alloy_consensus::{
    BlockHeader, Transaction, Typed2718,
    transaction::{Recovered, SignerRecoverable, TxHashRef},
};
use alloy_evm::{
    Evm, FromRecoveredTx, FromTxWithEncoded, RecoveredTx,
    block::{BlockExecutionError, BlockExecutionResult, BlockExecutor, StateDB, TxResult},
};
use alloy_network::{BlockResponse, TransactionResponse};
use alloy_primitives::B256;
use anvil_core::eth::transaction::{MaybeImpersonatedTransaction, TransactionInfo};
use eyre::{Context, Result};
use foundry_common::sh_warn;
use foundry_evm::core::evm::IntoInstructionResult;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxType};
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
    /// Where this transaction sat in the source block, for diagnostics only.
    ///
    /// The replayed block holds just the transactions anvil executed, so this is not its index
    /// there; storage keys receipts and traces by position in that block.
    pub(crate) source_index: usize,
}

/// A validated transaction prefix together with its source block execution inputs.
pub(crate) struct PreparedForkTransactionReplay {
    pub(crate) transactions: Vec<HistoricalReplayTransaction>,
    pub(crate) timestamp: u64,
    pub(crate) parent_beacon_block_root: Option<B256>,
}

impl PreparedForkTransactionReplay {
    /// Resolves the execution chain ID encoded by the source prefix.
    ///
    /// Unprotected legacy prefixes inherit the execution identity exposed by the endpoint.
    pub(crate) fn execution_chain_id(&self, fallback: u64) -> Result<u64> {
        let mut resolved = None;
        for replay in &self.transactions {
            let Some(chain_id) = replay.transaction.tx().chain_id() else { continue };
            if let Some(expected) = resolved {
                eyre::ensure!(
                    chain_id == expected,
                    "source transaction at index {} uses chain ID {chain_id}, expected {expected}",
                    replay.source_index
                );
            } else {
                resolved = Some(chain_id);
            }
        }
        Ok(resolved.unwrap_or(fallback))
    }
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
    #[cfg_attr(not(feature = "monad"), allow(unused_variables))] trust_monad_protocol_sender: bool,
) -> Result<PreparedForkTransactionReplay> {
    let source_hash = replay.source_block.header().hash;
    let source_number = replay.source_block.header().number;
    let timestamp = replay.source_block.header().timestamp();
    let parent_beacon_block_root = replay.source_block.header().parent_beacon_block_root();
    let source_transactions = replay
        .source_block
        .transactions()
        .as_transactions()
        .expect("full source block validated during resolution");

    let target_index = replay.target_index;
    let transactions = source_transactions
        .iter()
        .take(target_index.saturating_add(1))
        .enumerate()
        .map(|(source_index, source_transaction)| {
            let source_transaction_hash = source_transaction.tx_hash();
            // Chains anvil can fork but not execute, such as Arbitrum and its Orbit rollups, mint
            // their own transaction types; Arbitrum opens every block with an `ArbitrumInternalTx`.
            // Those carry no EVM semantics anvil could apply, so the prefix skips them instead of
            // failing the whole replay. The requested transaction itself must still be executable,
            // otherwise the resulting fork would not be the state the caller asked for.
            if FoundryTxType::try_from(source_transaction.ty()).is_err() {
                eyre::ensure!(
                    source_index != target_index,
                    "fork transaction {source_transaction_hash} in block {source_hash} \
                     ({source_number}) has type 0x{:x}, which anvil cannot execute",
                    source_transaction.ty(),
                );
                sh_warn!(
                    "skipping source transaction {source_transaction_hash} at index \
                     {source_index} with unsupported type 0x{:x}; replayed state will not \
                     include its effects",
                    source_transaction.ty(),
                )?;
                return Ok(None);
            }
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
            #[cfg(feature = "monad")]
            let sender = if trust_monad_protocol_sender
                && source_transaction.from() == monad_revm::staking::constants::SYSTEM_ADDRESS
            {
                source_transaction.from()
            } else {
                transaction.recover_signer().wrap_err_with(|| {
                    format!(
                        "failed to recover sender for source transaction \
                         {source_transaction_hash} at index {source_index} in block {source_hash} \
                         ({source_number})"
                    )
                })?
            };
            #[cfg(not(feature = "monad"))]
            let sender = transaction.recover_signer().wrap_err_with(|| {
                format!(
                    "failed to recover sender for source transaction {source_transaction_hash} at \
                     index {source_index} in block {source_hash} ({source_number})"
                )
            })?;
            Ok(Some(HistoricalReplayTransaction {
                transaction: Recovered::new_unchecked(transaction, sender),
                source_index,
            }))
        })
        .filter_map(Result::transpose)
        .collect::<Result<_>>()?;

    Ok(PreparedForkTransactionReplay { transactions, timestamp, parent_beacon_block_root })
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
    execute_historical_replay_with(
        executor,
        transactions,
        inspector_config,
        |evm, tx_env, transaction_hash| {
            evm.transact(tx_env).map_err(|err| BlockExecutionError::evm(err, transaction_hash))
        },
    )
}

/// Executes a prepared prefix using a caller-selected network transaction entry point.
pub(crate) fn execute_historical_replay_with<E, F>(
    executor: &mut AnvilBlockExecutor<E>,
    transactions: &[HistoricalReplayTransaction],
    inspector_config: &InspectorTxConfig,
    mut transact: F,
) -> Result<(Vec<MaybeImpersonatedTransaction<FoundryTxEnvelope>>, Vec<TransactionInfo>)>
where
    E: Evm<
            DB: StateDB,
            Inspector = AnvilInspector,
            Tx: FromRecoveredTx<FoundryTxEnvelope> + FromTxWithEncoded<FoundryTxEnvelope>,
        >,
    E::HaltReason: Clone + IntoInstructionResult,
    F: FnMut(
        &mut E,
        E::Tx,
        B256,
    ) -> Result<
        revm::context_interface::result::ResultAndState<E::HaltReason>,
        BlockExecutionError,
    >,
{
    let mut stored_transactions = Vec::with_capacity(transactions.len());
    let mut transaction_infos = Vec::with_capacity(transactions.len());

    for (execution_index, replay) in transactions.iter().enumerate() {
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
            .execute_transaction_without_commit_with(
                replay.transaction.clone().into_encoded(),
                &mut transact,
            )
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
            transaction_index: execution_index as u64,
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::AnyRpcBlock;

    /// A real signed legacy transaction, so signer recovery and the hash check pass.
    const LEGACY_TX: &str = r#"{
        "type": "0x0",
        "chainId": "0x1",
        "nonce": "0x0",
        "gas": "0x5208",
        "gasPrice": "0x1",
        "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
        "value": "0x1",
        "input": "0x",
        "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
        "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
        "v": "0x1b",
        "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515",
        "from": "0xa1e4380a3b1f749673e270229993ee55f35663b4",
        "blockHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "blockNumber": "0x1",
        "transactionIndex": "0x1"
    }"#;

    /// An `ArbitrumInternalTx`, the type Arbitrum opens each block with.
    const ARBITRUM_INTERNAL_TX: &str = r#"{
        "type": "0x6a",
        "chainId": "0xa4b1",
        "nonce": "0x0",
        "gas": "0x0",
        "gasPrice": "0x0",
        "to": "0x00000000000000000000000000000000000a4b05",
        "value": "0x0",
        "input": "0x6bf6a42d",
        "hash": "0x2222222222222222222222222222222222222222222222222222222222222222",
        "from": "0x00000000000000000000000000000000000a4b05",
        "blockHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "blockNumber": "0x1",
        "transactionIndex": "0x0"
    }"#;

    fn source_block() -> AnyRpcBlock {
        let json = format!(
            r#"{{
                "hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
                "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
                "miner": "0x0000000000000000000000000000000000000000",
                "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "transactionsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "logsBloom": "0x{bloom}",
                "difficulty": "0x0",
                "number": "0x1",
                "gasLimit": "0x1c9c380",
                "gasUsed": "0x5208",
                "timestamp": "0x64",
                "extraData": "0x",
                "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "nonce": "0x0000000000000000",
                "uncles": [],
                "transactions": [{ARBITRUM_INTERNAL_TX}, {LEGACY_TX}]
            }}"#,
            bloom = "0".repeat(512),
        );
        serde_json::from_str(&json).unwrap()
    }

    fn replay_for(target_index: usize) -> ForkTransactionReplay {
        ForkTransactionReplay { source_block: source_block(), target_index }
    }

    #[test]
    fn skips_unsupported_prefix_transactions() {
        let prepared = prepare_fork_transaction_replay(replay_for(1), false).unwrap();

        // The Arbitrum-typed transaction at index 0 is dropped, and the standard one keeps its
        // position in the source block.
        assert_eq!(prepared.transactions.len(), 1);
        assert_eq!(prepared.transactions[0].source_index, 1);
    }

    #[test]
    fn rejects_unsupported_target_transaction() {
        let Err(err) = prepare_fork_transaction_replay(replay_for(0), false) else {
            panic!("expected the unsupported target transaction to be rejected");
        };
        assert!(err.to_string().contains("0x6a"), "unexpected error: {err}");
    }
}
