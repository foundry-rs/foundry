use crate::{
    eth::{
        backend::{db::Db, validate::TransactionValidator},
        error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    mem::inspector::Inspector,
};
use anvil_core::eth::{
    block::{Block, BlockInfo, Header, PartialHeader},
    receipt::{EIP1559Receipt, EIP2930Receipt, EIP658Receipt, Log, TypedReceipt},
    transaction::{PendingTransaction, TransactionInfo, TypedTransaction},
    trie,
};
use ethers::{
    abi::ethereum_types::BloomInput,
    types::{Bloom, H256, U256},
    utils::rlp,
};
use forge::revm::ExecutionResult;
use foundry_evm::{
    executor::backend::DatabaseError,
    revm,
    revm::{BlockEnv, CfgEnv, Env, Return, SpecId, TransactOut},
    trace::{node::CallTraceNode, CallTraceArena},
};
use std::sync::Arc;
use tracing::{trace, warn};

/// Represents an executed transaction (transacted on the DB)
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit_reason: Return,
    out: TransactOut,
    gas_used: u64,
    logs: Vec<Log>,
    traces: Vec<CallTraceNode>,
}

// == impl ExecutedTransaction ==

impl ExecutedTransaction {
    /// Creates the receipt for the transaction
    fn create_receipt(&self) -> TypedReceipt {
        let used_gas: U256 = self.gas_used.into();
        let mut bloom = Bloom::default();
        logs_bloom(self.logs.clone(), &mut bloom);
        let logs = self.logs.clone();

        // successful return see [Return]
        let status_code: u8 =
            if self.exit_reason as u8 <= Return::SelfDestruct as u8 { 1 } else { 0 };
        match &self.transaction.pending_transaction.transaction {
            TypedTransaction::Legacy(_) => TypedReceipt::Legacy(EIP658Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
            TypedTransaction::EIP2930(_) => TypedReceipt::EIP2930(EIP2930Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
            TypedTransaction::EIP1559(_) => TypedReceipt::EIP1559(EIP1559Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
        }
    }
}

/// Represents the outcome of mining a new block
#[derive(Debug, Clone)]
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
    pub cfg_env: CfgEnv,
    pub parent_hash: H256,
    /// Cumulative gas used by all executed transactions
    pub gas_used: U256,
}

impl<'a, DB: Db + ?Sized, Validator: TransactionValidator> TransactionExecutor<'a, DB, Validator> {
    /// Executes all transactions and puts them in a new block with the provided `timestamp`
    pub fn execute(mut self) -> ExecutedTransactions {
        let mut transactions = Vec::new();
        let mut transaction_infos = Vec::new();
        let mut receipts = Vec::new();
        let mut bloom = Bloom::default();
        let mut cumulative_gas_used = U256::zero();
        let mut invalid = Vec::new();
        let mut included = Vec::new();
        let gas_limit = self.block_env.gas_limit;
        let parent_hash = self.parent_hash;
        let block_number = self.block_env.number;
        let difficulty = self.block_env.difficulty;
        let beneficiary = self.block_env.coinbase;
        let timestamp = self.block_env.timestamp.as_u64();
        let base_fee = if (self.cfg_env.spec_id as u8) >= (SpecId::LONDON as u8) {
            Some(self.block_env.basefee)
        } else {
            None
        };

        for tx in self.into_iter() {
            let tx = match tx {
                TransactionExecutionOutcome::Executed(tx) => {
                    included.push(tx.transaction.clone());
                    tx
                }
                TransactionExecutionOutcome::Exhausted(_) => continue,
                TransactionExecutionOutcome::Invalid(tx, _) => {
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
            let receipt = tx.create_receipt();
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
            let ExecutedTransaction { transaction, logs, out, traces, exit_reason: exit, .. } = tx;
            logs_bloom(logs.clone(), &mut bloom);

            let contract_address = if let TransactOut::Create(_, contract_address) = out {
                trace!(target: "backend", "New contract deployed: at {:?}", contract_address);
                contract_address
            } else {
                None
            };

            let transaction_index = transaction_infos.len() as u32;
            let info = TransactionInfo {
                transaction_hash: *transaction.hash(),
                transaction_index,
                from: *transaction.pending_transaction.sender(),
                to: transaction.pending_transaction.transaction.to().copied(),
                contract_address,
                logs,
                logs_bloom: *receipt.logs_bloom(),
                traces: CallTraceArena { arena: traces },
                exit,
                out: match out {
                    TransactOut::Call(b) => Some(b.into()),
                    TransactOut::Create(b, _) => Some(b.into()),
                    _ => None,
                },
            };

            transaction_infos.push(info);
            receipts.push(receipt);
            transactions.push(transaction.pending_transaction.transaction.clone());
        }

        let ommers: Vec<Header> = Vec::new();
        let receipts_root = trie::ordered_trie_root(receipts.iter().map(rlp::encode));

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

    fn env_for(&self, tx: &PendingTransaction) -> Env {
        Env { cfg: self.cfg_env.clone(), block: self.block_env.clone(), tx: tx.to_revm_tx_env() }
    }
}

/// Represents the result of a single transaction execution attempt
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
        let max_gas = self.gas_used.saturating_add(U256::from(env.tx.gas_limit));
        if max_gas > env.block.gas_limit {
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

        let mut evm = revm::EVM::new();
        evm.env = env;
        evm.database(&mut self.db);

        // records all call and step traces
        let mut inspector = Inspector::default().with_tracing().with_steps_tracing();

        trace!(target: "backend", "[{:?}] executing", transaction.hash());
        // transact and commit the transaction
        let ExecutionResult { exit_reason, out, gas_used, logs, .. } =
            evm.inspect_commit(&mut inspector);
        inspector.print_logs();

        if exit_reason == Return::OutOfGas {
            // this currently useful for debugging estimations
            warn!(target: "backend", "[{:?}] executed with out of gas", transaction.hash())
        }

        trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", transaction.hash(), out);

        self.gas_used.saturating_add(U256::from(gas_used));

        trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", transaction.hash(), exit_reason, gas_used);

        let tx = ExecutedTransaction {
            transaction,
            exit_reason,
            out,
            gas_used,
            logs: logs.into_iter().map(Into::into).collect(),
            traces: inspector.tracer.unwrap_or_default().traces.arena,
        };

        Some(TransactionExecutionOutcome::Executed(tx))
    }
}

/// Inserts all logs into the bloom
fn logs_bloom(logs: Vec<Log>, bloom: &mut Bloom) {
    for log in logs {
        bloom.accrue(BloomInput::Raw(&log.address[..]));
        for topic in log.topics {
            bloom.accrue(BloomInput::Raw(&topic[..]));
        }
    }
}
