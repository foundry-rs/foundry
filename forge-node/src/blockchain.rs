use std::collections::{HashMap, VecDeque};

use ethers::prelude::{Block, Log, Transaction, TransactionReceipt, TxHash, H256, U256, U64};

#[derive(Default)]
/// Stores the blockchain data (blocks, transactions)
pub struct Blockchain {
    /// Mapping from block hash to the block number
    pub(crate) blocks_by_hash: HashMap<H256, U64>,
    /// Mapping from block number to the block
    pub(crate) blocks: Vec<Block<TxHash>>,
    /// Mapping from txhash to a tuple containing the transaction as well as the transaction
    /// receipt
    pub(crate) txs: HashMap<TxHash, (Transaction, TransactionReceipt)>,

    // TODO(rohit): this should be completely moved to a tx pool module
    /// Pending txs that haven't yet been included in the blockchain
    pub(crate) pending_txs: VecDeque<(Transaction, Vec<Log>, Option<String>)>,
}

impl Blockchain {
    /// Gets transaction by transaction hash
    pub fn tx(&self, tx_hash: TxHash) -> Option<Transaction> {
        self.txs.get(&tx_hash).cloned().map(|t| t.0)
    }

    /// Gets transaction receipt by transaction hash
    pub fn tx_receipt(&self, tx_hash: TxHash) -> Option<TransactionReceipt> {
        self.txs.get(&tx_hash).cloned().map(|t| t.1)
    }

    /// Gets block by block hash
    pub fn block_by_hash(&self, hash: H256) -> Option<Block<TxHash>> {
        self.blocks_by_hash
            .get(&hash)
            .map(|i| self.block_by_number(*i).expect("block should exist if block hash was found"))
    }

    /// Gets block by block number
    pub fn block_by_number(&self, n: U64) -> Option<Block<TxHash>> {
        if self.blocks.len() > n.as_usize() {
            Some(self.blocks[n.as_usize()].clone())
        } else {
            None
        }
    }

    /// Gets the latest block
    #[allow(dead_code)]
    pub fn latest_block(&self) -> Option<Block<TxHash>> {
        self.blocks.last().cloned()
    }
}

impl Blockchain {
    /// Add a pending transaction eligible to be included in the next block
    pub fn insert_pending_tx(
        &mut self,
        tx: Transaction,
        logs: Vec<Log>,
        revert_reason: Option<String>,
    ) {
        self.pending_txs.push_back((tx, logs, revert_reason))
    }

    /// Gets a list of pending txs, each of which is a tuple of Transaction, its logs and an
    /// optional revert reason.
    pub fn pending_txs(
        &mut self,
        block_gas_limit: U256,
    ) -> Vec<(Transaction, Vec<Log>, Option<String>)> {
        let mut cumulative_gas_used = U256::zero();
        let mut chosen_txs = vec![];

        while let Some((pending_tx, _, _)) = self.pending_txs.front() {
            cumulative_gas_used += pending_tx.gas;
            if cumulative_gas_used < block_gas_limit {
                chosen_txs.push(self.pending_txs.pop_front().expect("element should be present"));
            }
        }

        chosen_txs
    }
}
