use crate::{
    eth::{
        backend::{db::Db, validate::TransactionValidator},
        error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    inject_precompiles,
    mem::inspector::Inspector,
    PrecompileFactory,
};
use alloy_consensus::{Header, Receipt, ReceiptWithBloom};
use alloy_primitives::{Bloom, BloomInput, Log, B256};
use anvil_core::eth::{
    block::{Block, BlockInfo, PartialHeader},
    transaction::{
        DepositReceipt, PendingTransaction, TransactionInfo, TypedReceipt, TypedTransaction,
    },
    trie,
};
use foundry_evm::{
    backend::DatabaseError,
    inspectors::{TracingInspector, TracingInspectorConfig},
    revm::{
        interpreter::InstructionResult,
        primitives::{
            BlockEnv, CfgEnvWithHandlerCfg, EVMError, EnvWithHandlerCfg, ExecutionResult, Output,
            SpecId,
        },
    },
    traces::CallTraceNode,
};
use std::sync::Arc;

/// Represents an executed transaction (transacted on the DB)
#[derive(Debug)]
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit_reason: InstructionResult,
    out: Option<Output>,
    gas_used: u128,
    logs: Vec<Log>,
    traces: Vec<CallTraceNode>,
    nonce: u64,
}

// == impl ExecutedTransaction ==

impl ExecutedTransaction {
    /// Creates the receipt for the transaction
    fn create_receipt(&self, cumulative_gas_used: &mut u128) -> TypedReceipt {
        let logs = self.logs.clone();
        *cumulative_gas_used = cumulative_gas_used.saturating_add(self.gas_used);

        // successful return see [Return]
        let status_code = u8::from(self.exit_reason as u8 <= InstructionResult::SelfDestruct as u8);
        let receipt_with_bloom: ReceiptWithBloom =
            Receipt { status: status_code == 1, cumulative_gas_used: *cumulative_gas_used, logs }
                .into();

        match &self.transaction.pending_transaction.transaction.transaction {
            TypedTransaction::Legacy(_) => TypedReceipt::Legacy(receipt_with_bloom),
            TypedTransaction::EIP2930(_) => TypedReceipt::EIP2930(receipt_with_bloom),
            TypedTransaction::EIP1559(_) => TypedReceipt::EIP1559(receipt_with_bloom),
            TypedTransaction::EIP4844(_) => TypedReceipt::EIP4844(receipt_with_bloom),
            TypedTransaction::Deposit(tx) => TypedReceipt::Deposit(DepositReceipt {
                inner: receipt_with_bloom,
                deposit_nonce: Some(tx.nonce),
                deposit_nonce_version: Some(1),
            }),
        }
    }
}

/// Represents the outcome of mining a new block
#[derive(Clone, Debug)]
pub struct ExecutedTransactions {
    /// The block created after executing the `included` transactions
    pub block: BlockInfo,
    /// All transactions included in the
    pub included: Vec<Arc<PoolTransaction>>,
    /// All transactions that were invalid at the point of their execution and were not included in
    /// the block
    pub invalid: Vec<Arc<PoolTransaction>>,
}

/// An executor for a series of transactions
pub struct TransactionExecutor<'a, Db: ?Sized, Validator: TransactionValidator> {
    /// where to insert the transactions
    pub db: &'a mut Db,
    /// type used to validate before inclusion
    pub validator: Validator,
    /// all pending transactions
    pub pending: std::vec::IntoIter<Arc<PoolTransaction>>,
    pub block_env: BlockEnv,
    /// The configuration environment and spec id
    pub cfg_env: CfgEnvWithHandlerCfg,
    pub parent_hash: B256,
    /// Cumulative gas used by all executed transactions
    pub gas_used: u128,
    pub enable_steps_tracing: bool,
    /// Precompiles to inject to the EVM.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
}

impl<'a, DB: Db + ?Sized, Validator: TransactionValidator> TransactionExecutor<'a, DB, Validator> {
    /// Executes all transactions and puts them in a new block with the provided `timestamp`
    pub fn execute(mut self) -> ExecutedTransactions {
        let mut transactions = Vec::new();
        let mut transaction_infos = Vec::new();
        let mut receipts = Vec::new();
        let mut bloom = Bloom::default();
        let mut cumulative_gas_used: u128 = 0;
        let mut invalid = Vec::new();
        let mut included = Vec::new();
        let gas_limit = self.block_env.gas_limit.to::<u128>();
        let parent_hash = self.parent_hash;
        let block_number = self.block_env.number.to::<u64>();
        let difficulty = self.block_env.difficulty;
        let beneficiary = self.block_env.coinbase;
        let timestamp = self.block_env.timestamp.to::<u64>();
        let base_fee = if self.cfg_env.handler_cfg.spec_id >= SpecId::LONDON {
            Some(self.block_env.basefee.to::<u128>())
        } else {
            None
        };

        for tx in self.into_iter() {
            let tx = match tx {
                TransactionExecutionOutcome::Executed(tx) => {
                    included.push(tx.transaction.clone());
                    tx
                }
                TransactionExecutionOutcome::Exhausted(tx) => {
                    trace!(target: "backend",  tx_gas_limit = %tx.pending_transaction.transaction.gas_limit(), ?tx,  "block gas limit exhausting, skipping transaction");
                    continue
                }
                TransactionExecutionOutcome::Invalid(tx, _) => {
                    trace!(target: "backend", ?tx,  "skipping invalid transaction");
                    invalid.push(tx);
                    continue
                }
                TransactionExecutionOutcome::DatabaseError(_, err) => {
                    // Note: this is only possible in forking mode, if for example a rpc request
                    // failed
                    trace!(target: "backend", ?err,  "Failed to execute transaction due to database error");
                    continue
                }
            };
            let receipt = tx.create_receipt(&mut cumulative_gas_used);
            let ExecutedTransaction { transaction, logs, out, traces, exit_reason: exit, .. } = tx;
            build_logs_bloom(logs.clone(), &mut bloom);

            let contract_address = out.as_ref().and_then(|out| {
                if let Output::Create(_, contract_address) = out {
                    trace!(target: "backend", "New contract deployed: at {:?}", contract_address);
                    *contract_address
                } else {
                    None
                }
            });

            let transaction_index = transaction_infos.len() as u64;
            let info = TransactionInfo {
                transaction_hash: transaction.hash(),
                transaction_index,
                from: *transaction.pending_transaction.sender(),
                to: transaction.pending_transaction.transaction.to(),
                contract_address,
                traces,
                exit,
                out: match out {
                    Some(Output::Call(b)) => Some(alloy_primitives::Bytes(b.0)),
                    Some(Output::Create(b, _)) => Some(alloy_primitives::Bytes(b.0)),
                    _ => None,
                },
                nonce: tx.nonce,
                gas_used: tx.gas_used,
            };

            transaction_infos.push(info);
            receipts.push(receipt);
            transactions.push(transaction.pending_transaction.transaction.clone());
        }

        let ommers: Vec<Header> = Vec::new();
        let receipts_root = trie::ordered_trie_root(receipts.iter().map(alloy_rlp::encode));

        let partial_header = PartialHeader {
            parent_hash,
            beneficiary,
            state_root: self.db.maybe_state_root().unwrap_or_default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty,
            number: block_number,
            gas_limit,
            gas_used: cumulative_gas_used,
            timestamp,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee,
        };

        let block = Block::new(partial_header, transactions.clone(), ommers);
        let block = BlockInfo { block, transactions: transaction_infos, receipts };
        ExecutedTransactions { block, included, invalid }
    }

    fn env_for(&self, tx: &PendingTransaction) -> EnvWithHandlerCfg {
        let mut tx_env = tx.to_revm_tx_env();
        if self.cfg_env.handler_cfg.is_optimism {
            tx_env.optimism.enveloped_tx =
                Some(alloy_rlp::encode(&tx.transaction.transaction).into());
        }

        EnvWithHandlerCfg::new_with_cfg_env(
            self.cfg_env.clone(),
            self.block_env.clone(),
            tx.to_revm_tx_env(),
        )
    }
}

/// Represents the result of a single transaction execution attempt
#[derive(Debug)]
pub enum TransactionExecutionOutcome {
    /// Transaction successfully executed
    Executed(ExecutedTransaction),
    /// Invalid transaction not executed
    Invalid(Arc<PoolTransaction>, InvalidTransactionError),
    /// Execution skipped because could exceed gas limit
    Exhausted(Arc<PoolTransaction>),
    /// When an error occurred during execution
    DatabaseError(Arc<PoolTransaction>, DatabaseError),
}

impl<'a, 'b, DB: Db + ?Sized, Validator: TransactionValidator> Iterator
    for &'b mut TransactionExecutor<'a, DB, Validator>
{
    type Item = TransactionExecutionOutcome;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;
        let sender = *transaction.pending_transaction.sender();
        let account = match self.db.basic(sender).map(|acc| acc.unwrap_or_default()) {
            Ok(account) => account,
            Err(err) => return Some(TransactionExecutionOutcome::DatabaseError(transaction, err)),
        };
        let env = self.env_for(&transaction.pending_transaction);
        // check that we comply with the block's gas limit
        let max_gas = self.gas_used.saturating_add(env.tx.gas_limit as u128);
        if max_gas > env.block.gas_limit.to::<u128>() {
            return Some(TransactionExecutionOutcome::Exhausted(transaction))
        }

        // validate before executing
        if let Err(err) = self.validator.validate_pool_transaction_for(
            &transaction.pending_transaction,
            &account,
            &env,
        ) {
            warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", transaction.hash(), err);
            return Some(TransactionExecutionOutcome::Invalid(transaction, err))
        }

        let nonce = account.nonce;

        // records all call and step traces
        let mut inspector = Inspector::default().with_tracing();
        if self.enable_steps_tracing {
            inspector = inspector.with_steps_tracing();
        }

        let exec_result = {
            let mut evm =
                foundry_evm::utils::new_evm_with_inspector(&mut *self.db, env, &mut inspector);
            if let Some(ref factory) = self.precompile_factory {
                inject_precompiles(&mut evm, factory.precompiles());
            }

            trace!(target: "backend", "[{:?}] executing", transaction.hash());
            // transact and commit the transaction
            match evm.transact_commit() {
                Ok(exec_result) => exec_result,
                Err(err) => {
                    warn!(target: "backend", "[{:?}] failed to execute: {:?}", transaction.hash(), err);
                    match err {
                        EVMError::Database(err) => {
                            return Some(TransactionExecutionOutcome::DatabaseError(
                                transaction,
                                err,
                            ))
                        }
                        EVMError::Transaction(err) => {
                            return Some(TransactionExecutionOutcome::Invalid(
                                transaction,
                                err.into(),
                            ))
                        }
                        // This will correspond to prevrandao not set, and it should never happen.
                        // If it does, it's a bug.
                        e => {
                            panic!("Failed to execute transaction. This is a bug.\n {:?}", e)
                        }
                    }
                }
            }
        };
        inspector.print_logs();

        let (exit_reason, gas_used, out, logs) = match exec_result {
            ExecutionResult::Success { reason, gas_used, logs, output, .. } => {
                (reason.into(), gas_used, Some(output), Some(logs))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)), None)
            }
            ExecutionResult::Halt { reason, gas_used } => (reason.into(), gas_used, None, None),
        };

        if exit_reason == InstructionResult::OutOfGas {
            // this currently useful for debugging estimations
            warn!(target: "backend", "[{:?}] executed with out of gas", transaction.hash())
        }

        trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", transaction.hash(), out);

        self.gas_used = self.gas_used.saturating_add(gas_used as u128);

        trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", transaction.hash(), exit_reason, gas_used);

        let tx = ExecutedTransaction {
            transaction,
            exit_reason,
            out,
            gas_used: gas_used as u128,
            logs: logs.unwrap_or_default(),
            traces: inspector
                .tracer
                .unwrap_or(TracingInspector::new(TracingInspectorConfig::all()))
                .get_traces()
                .clone()
                .into_nodes(),
            nonce,
        };

        Some(TransactionExecutionOutcome::Executed(tx))
    }
}

/// Inserts all logs into the bloom
fn build_logs_bloom(logs: Vec<Log>, bloom: &mut Bloom) {
    for log in logs {
        bloom.accrue(BloomInput::Raw(&log.address[..]));
        for topic in log.topics() {
            bloom.accrue(BloomInput::Raw(&topic[..]));
        }
    }
}
