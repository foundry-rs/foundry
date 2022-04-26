use crate::eth::{
    backend::{db::Db, validate::TransactionValidator},
    error::InvalidTransactionError,
    pool::transactions::PoolTransaction,
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
use foundry_evm::{
    executor::inspector::Tracer,
    revm,
    revm::{BlockEnv, CfgEnv, Env, Return, TransactOut},
    trace::node::CallTraceNode,
};
use std::sync::Arc;
use tracing::trace;

/// Represents an executed transaction (transacted on the DB)
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit: Return,
    out: TransactOut,
    gas: u64,
    logs: Vec<Log>,
    traces: Vec<CallTraceNode>,
}

// == impl ExecutedTransaction ==

impl ExecutedTransaction {
    /// Creates the receipt for the transaction
    fn create_receipt(&self) -> TypedReceipt {
        let used_gas: U256 = self.gas.into();
        let mut bloom = Bloom::default();
        logs_bloom(self.logs.clone(), &mut bloom);
        let logs = self.logs.clone();

        // successful return see [Return]
        let status_code: u8 = if self.exit as u8 <= Return::SelfDestruct as u8 { 1 } else { 0 };
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
}

impl<'a, DB: Db + ?Sized, Validator: TransactionValidator> TransactionExecutor<'a, DB, Validator> {
    /// Executes all transactions and puts them in a new block with the provided `timestamp`
    pub fn execute(self, timestamp: u64) -> ExecutedTransactions {
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

        for (idx, tx) in self.enumerate() {
            let tx = match tx {
                Ok(tx) => {
                    included.push(tx.transaction.clone());
                    tx
                }
                Err((tx, _)) => {
                    invalid.push(tx);
                    continue
                }
            };
            let receipt = tx.create_receipt();
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
            let ExecutedTransaction { transaction, logs, out, traces, .. } = tx;
            logs_bloom(logs.clone(), &mut bloom);

            let contract_address = if let TransactOut::Create(_, contract_address) = out {
                trace!(target: "backend", "New contract deployed: at {:?}", contract_address);
                contract_address
            } else {
                None
            };
            let info = TransactionInfo {
                transaction_hash: *transaction.hash(),
                transaction_index: idx as u32,
                from: *transaction.pending_transaction.sender(),
                to: transaction.pending_transaction.transaction.to().copied(),
                contract_address,
                logs,
                logs_bloom: *receipt.logs_bloom(),
                traces,
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
            // TODO need a triebackend to get this efficiently
            state_root: Default::default(),
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
type TransactionExecutionResult =
    Result<ExecutedTransaction, (Arc<PoolTransaction>, InvalidTransactionError)>;

impl<'a, DB: Db + ?Sized, Validator: TransactionValidator> Iterator
    for TransactionExecutor<'a, DB, Validator>
{
    type Item = TransactionExecutionResult;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;

        // validate before executing
        if let Err(err) = self.validator.validate_pool_transaction(&transaction.pending_transaction)
        {
            trace!(target: "backend", "Skipping invalid tx execution [{:?}] {}", transaction.hash(), err);
            return Some(Err((transaction, err)))
        }

        let mut evm = revm::EVM::new();
        evm.env = self.env_for(&transaction.pending_transaction);
        evm.database(&mut self.db);

        // records all call traces
        let mut tracer = Tracer::default();

        // transact and commit the transaction
        let (exit, out, gas, logs) = evm.inspect_commit(&mut tracer);

        trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", transaction.hash(), exit, gas);

        let tx = ExecutedTransaction {
            transaction,
            exit,
            out,
            gas,
            logs: logs.into_iter().map(Into::into).collect(),
            traces: tracer.traces.arena,
        };

        Some(Ok(tx))
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
