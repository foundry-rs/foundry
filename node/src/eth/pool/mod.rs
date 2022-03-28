use crate::eth::{
    error::{BlockchainError, PoolError},
    pool::transactions::{
        PendingTransaction, PendingTransactions, PoolTransaction, ReadyTransactions,
    },
};
use ethers::types::{Transaction, TxHash};
use std::collections::VecDeque;

use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{debug, trace};

pub mod transactions;

/// Transaction pool that performs validation.
pub struct Pool {
    /// processes all pending transactions
    pool: RwLock<PoolInner>,
}

/// A Transaction Pool
///
/// Contains all transactions that are ready to be executed
#[derive(Debug, Default)]
struct PoolInner {
    ready_transactions: ReadyTransactions,
    pending_transactions: PendingTransactions,
}

// == impl PoolInner ==

impl PoolInner {
    /// Returns true if this pool already contains the transaction
    pub fn contains(&self, tx_hash: &TxHash) -> bool {
        self.pending_transactions.contains(tx_hash) || self.ready_transactions.contains(tx_hash)
    }

    /// Adds a new transaction to the pool
    pub fn add_transaction(&mut self, tx: PoolTransaction) -> Result<AddedTransaction, PoolError> {
        if self.contains(tx.hash()) {
            return Err(PoolError::AlreadyImported(Box::new(tx)))
        }
        let tx = PendingTransaction::new(tx, self.ready_transactions.provided_markers());
        trace!(target: "txpool", "[{:?}] {:?}", tx.transaction.hash(), tx);

        // If all markers are not satisfied import to future
        if !tx.is_ready() {
            let hash = *tx.transaction.hash();
            self.pending_transactions.add_transaction(tx);
            return Ok(AddedTransaction::Pending { hash })
        }
        self.add_ready_transaction(tx)
    }

    /// Adds the transaction to the ready queue
    fn add_ready_transaction(
        &mut self,
        tx: PendingTransaction,
    ) -> Result<AddedTransaction, PoolError> {
        let hash = *tx.transaction.hash();
        let mut ready = ReadyTransaction::new(hash);

        let mut tx_queue = VecDeque::from([tx]);
        // tracks whether we're processing the given `tx`
        let mut is_new_tx = true;

        // take first transaction from the list
        while let Some(current_tx) = tx_queue.pop_front() {
            // also add the transaction that the current transaction unlocks
            tx_queue.extend(
                self.pending_transactions.mark_and_unlock(&current_tx.transaction.provides),
            );

            let current_hash = *current_tx.transaction.hash();
            // try to add the transaction to the ready pool
            match self.ready_transactions.add_transaction(current_tx) {
                Ok(mut replaced_transactions) => {
                    if !is_new_tx {
                        ready.promoted.push(current_hash);
                    }
                    // tx removed from ready pool
                    ready.removed.extend(replaced_transactions);
                }
                Err(err) => {
                    // failed to add transaction
                    if is_new_tx {
                        debug!(target: "txpool", "[{:?}] Failed to add tx: {:?}", current_hash,
        err);
                        return Err(err)
                    } else {
                        ready.discarded.push(current_hash);
                    }
                }
            }
            is_new_tx = false;
        }

        // check for a cycle where importing a transaction resulted in pending transactions to be
        // added while removing current transaction. in which case we move this transaction back to
        // the pending queue
        if ready.removed.iter().any(|tx| *tx.hash() == hash) {
            self.ready_transactions.clear_transactions(&ready.promoted);
            return Err(PoolError::CyclicTransaction)
        }

        Ok(AddedTransaction::Ready(ready))
    }
}

#[derive(Debug, Clone)]
pub struct ReadyTransaction {
    /// the hash of the submitted transaction
    hash: TxHash,
    /// transactions promoted to the ready queue
    promoted: Vec<TxHash>,
    /// transaction that failed and became discarded
    discarded: Vec<TxHash>,
    /// Transactions removed from the Ready pool
    removed: Vec<Arc<PoolTransaction>>,
}

impl ReadyTransaction {
    pub fn new(hash: TxHash) -> Self {
        Self {
            hash,
            promoted: Default::default(),
            discarded: Default::default(),
            removed: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AddedTransaction {
    /// transaction was successfully added and being processed
    Ready(ReadyTransaction),
    /// Transaction was successfully added but not yet queued for processing
    Pending {
        /// the hash of the submitted transaction
        hash: TxHash,
    },
}

/// A validated transaction
#[derive(Debug)]
pub enum ValidatedTransaction {
    /// Transaction that has been validated successfully.
    Valid(Transaction),
    /// Transaction that is invalid.
    Invalid(TxHash, BlockchainError),
}
