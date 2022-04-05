use crate::eth::{
    backend::{db::Db, duration_since_unix_epoch},
    pool::transactions::PoolTransaction,
};
use ethers::{
    abi::ethereum_types::BloomInput,
    types::{Bloom, H256, U256},
    utils::rlp,
};
use forge_node_core::eth::{
    block::{Block, BlockInfo, Header, PartialHeader},
    receipt::{EIP1559Receipt, EIP2930Receipt, EIP658Receipt, Log, TypedReceipt},
    transaction::{PendingTransaction, TransactionInfo, TypedTransaction},
    trie,
};
use foundry_evm::{
    revm,
    revm::{BlockEnv, CfgEnv, Env, Return, TransactOut},
};
use std::sync::Arc;

/// Represents an executed transaction (transacted on the DB)
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit: Return,
    out: TransactOut,
    gas: u64,
    logs: Vec<Log>,
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

/// An executer for a series of transactions
pub struct TransactionExecutor<'a, Db: ?Sized> {
    /// where to insert the transactions
    pub db: &'a mut Db,
    /// all pending transactions
    pub pending: std::vec::IntoIter<Arc<PoolTransaction>>,
    pub block_env: BlockEnv,
    pub cfg_env: CfgEnv,
    pub parent_hash: H256,
}

impl<'a, DB: Db + ?Sized> TransactionExecutor<'a, DB> {
    /// Executes all transactions and puts them in a new block
    pub fn create_block(self) -> BlockInfo {
        let mut transactions = Vec::new();
        let mut transaction_infos = Vec::new();
        let mut receipts = Vec::new();
        let mut bloom = Bloom::default();
        let mut cumulative_gas_used = U256::zero();
        let gas_limit = self.block_env.gas_limit;
        let parent_hash = self.parent_hash;
        let block_number = self.block_env.number;
        let difficulty = self.block_env.difficulty;
        let beneficiary = self.block_env.coinbase;

        for (idx, tx) in self.enumerate() {
            let receipt = tx.create_receipt();
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
            let ExecutedTransaction { transaction, logs, out, .. } = tx;
            logs_bloom(logs.clone(), &mut bloom);

            let contract_address = if let TransactOut::Create(_, contract_address) = out {
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
            timestamp: duration_since_unix_epoch().as_secs(),
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
        };

        let block = Block::new(partial_header, transactions.clone(), ommers);
        BlockInfo { block, transactions: transaction_infos, receipts }
    }

    fn env_for(&self, tx: &PendingTransaction) -> Env {
        Env { cfg: self.cfg_env.clone(), block: self.block_env.clone(), tx: tx.to_revm_tx_env() }
    }
}

impl<'a, DB: Db + ?Sized> Iterator for TransactionExecutor<'a, DB> {
    type Item = ExecutedTransaction;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;

        let mut evm = revm::EVM::new();
        evm.env = self.env_for(&transaction.pending_transaction);
        evm.database(&mut self.db);

        // transact and commit the transaction
        let (exit, out, gas, logs) = evm.transact_commit();

        Some(ExecutedTransaction {
            transaction,
            exit,
            out,
            gas,
            logs: logs.into_iter().map(Into::into).collect(),
        })
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
