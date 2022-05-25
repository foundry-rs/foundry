//! # Transaction Pool implementation
//!
//! The transaction pool is responsible for managing a set of transactions that can be included in
//! upcoming blocks.
//!
//! The main task of the pool is to prepare an ordered list of transactions that are ready to be
//! included in a new block.
//!
//! Each imported block can affect the validity of transactions already in the pool.
//! The miner expects the most up-to-date transactions when attempting to create a new block.
//! After being included in a block, a transaction should be removed from the pool, this process is
//! called _pruning_ and due to separation of concerns is triggered externally.
//! The pool essentially performs following services:
//!   * import transactions
//!   * order transactions
//!   * provide ordered set of transactions that are ready for inclusion
//!   * prune transactions
//!
//! Each transaction in the pool contains markers that it _provides_ or _requires_. This property is
//! used to determine whether it can be included in a block (transaction is ready) or whether it
//! still _requires_ other transactions to be mined first (transaction is pending).
//! A transaction is associated with the nonce of the account it's sent from. A unique identifying
//! marker for a transaction is therefor the pair `(nonce + account)`. An incoming transaction with
//! a `nonce > nonce on chain` will _require_ `(nonce -1, account)` first, before it is ready to be
//! included in a block.
//!
//! This implementation is adapted from <https://github.com/paritytech/substrate/tree/master/client/transaction-pool>

use crate::{
    eth::{
        error::PoolError,
        pool::transactions::{
            PendingPoolTransaction, PendingTransactions, PoolTransaction, ReadyTransactions,
            TransactionsIterator, TxMarker,
        },
    },
    mem::storage::MinedBlockOutcome,
};
use anvil_core::eth::transaction::PendingTransaction;
use ethers::{
    prelude::TxpoolStatus,
    types::{TxHash, U64},
};
use futures::channel::mpsc::{channel, Receiver, Sender};
use parking_lot::{Mutex, RwLock};
use std::{collections::VecDeque, fmt, sync::Arc};
use tracing::{debug, trace, warn};

pub mod transactions;

/// Transaction pool that performs validation.
#[derive(Default)]
pub struct Pool {
    /// processes all pending transactions
    inner: RwLock<PoolInner>,
    /// listeners for new ready transactions
    transaction_listener: Mutex<Vec<Sender<TxHash>>>,
}

// == impl Pool ==

impl Pool {
    /// Returns an iterator that yields all transactions that are currently ready
    pub fn ready_transactions(&self) -> TransactionsIterator {
        self.inner.read().ready_transactions()
    }

    /// Returns all transactions that are not ready to be included in a block yet
    pub fn pending_transactions(&self) -> Vec<Arc<PoolTransaction>> {
        self.inner.read().pending_transactions.transactions().collect()
    }

    /// Returns the _pending_ transaction for that `hash` if it exists in the mempool
    pub fn get_transaction(&self, hash: TxHash) -> Option<PendingTransaction> {
        self.inner.read().get_transaction(hash)
    }

    /// Returns the number of tx that are ready and queued for further execution
    pub fn txpool_status(&self) -> TxpoolStatus {
        // Note: naming differs here compared to geth's `TxpoolStatus`
        let pending = self.ready_transactions().count().into();
        let queued = self.inner.read().pending_transactions.len().into();
        TxpoolStatus { pending, queued }
    }

    /// Invoked when a set of transactions ([Self::ready_transactions()]) was executed.
    ///
    /// This will remove the transactions from the pool.
    pub fn on_mined_block(&self, outcome: MinedBlockOutcome) -> PruneResult {
        let MinedBlockOutcome { block_number, included, invalid } = outcome;

        // remove invalid transactions from the pool
        self.remove_invalid(invalid.into_iter().map(|tx| *tx.hash()).collect());

        // prune all the markers the mined transactions provide
        let res = self
            .prune_markers(block_number, included.into_iter().flat_map(|tx| tx.provides.clone()));
        trace!(target: "node", "pruned transaction markers {:?}", res);
        res
    }

    /// Removes ready transactions for the given iterator of identifying markers.
    ///
    /// For each marker we can remove transactions in the pool that either provide the marker
    /// directly or are a dependency of the transaction associated with that marker.
    pub fn prune_markers(
        &self,
        block_number: U64,
        markers: impl IntoIterator<Item = TxMarker>,
    ) -> PruneResult {
        debug!(target: "txpool", "pruning transactions for block {}", block_number);
        self.inner.write().prune_markers(markers)
    }

    /// Adds a new transaction to the pool
    pub fn add_transaction(&self, tx: PoolTransaction) -> Result<AddedTransaction, PoolError> {
        let added = self.inner.write().add_transaction(tx)?;
        if let AddedTransaction::Ready(ref ready) = added {
            self.notify_listener(ready.hash)
        }
        Ok(added)
    }

    /// Adds a new transaction listener to the pool that gets notified about every new ready
    /// transaction
    pub fn add_ready_listener(&self) -> Receiver<TxHash> {
        const TX_LISTENER_BUFFER_SIZE: usize = 2048;
        let (tx, rx) = channel(TX_LISTENER_BUFFER_SIZE);
        self.transaction_listener.lock().push(tx);
        rx
    }

    /// Returns true if this pool already contains the transaction
    pub fn contains(&self, tx_hash: &TxHash) -> bool {
        self.inner.read().contains(tx_hash)
    }

    /// Remove the given transactions from the pool
    pub fn remove_invalid(&self, tx_hashes: Vec<TxHash>) -> Vec<Arc<PoolTransaction>> {
        self.inner.write().remove_invalid(tx_hashes)
    }

    /// Removes a single transaction from the pool
    ///
    /// This is similar to `[Pool::remove_invalid()]` but for a single transaction.
    ///
    /// **Note**: this will also drop any transaction that depend on the `tx`
    pub fn drop_transaction(&self, tx: TxHash) -> Option<Arc<PoolTransaction>> {
        trace!(target: "txpool", "Dropping transaction: {:?}", tx);
        let removed = {
            let mut pool = self.inner.write();
            pool.ready_transactions.remove_with_markers(vec![tx], None)
        };
        trace!(target: "txpool", "Dropped transactions: {:?}", removed);

        let mut dropped = None;
        if !removed.is_empty() {
            dropped = removed.into_iter().find(|t| *t.pending_transaction.hash() == tx);
        }
        dropped
    }

    /// notifies all listeners about the transaction
    fn notify_listener(&self, hash: TxHash) {
        let mut listener = self.transaction_listener.lock();
        // this is basically a retain but with mut reference
        for n in (0..listener.len()).rev() {
            let mut listener_tx = listener.swap_remove(n);
            let retain = match listener_tx.try_send(hash) {
                Ok(()) => true,
                Err(e) => {
                    if e.is_full() {
                        warn!(
                            target: "txpool",
                            "[{:?}] Failed to send tx notification because channel is full",
                            hash,
                        );
                        true
                    } else {
                        false
                    }
                }
            };
            if retain {
                listener.push(listener_tx)
            }
        }
    }
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
    /// Returns an iterator over transactions that are ready.
    fn ready_transactions(&self) -> TransactionsIterator {
        self.ready_transactions.get_transactions()
    }

    /// checks both pools for the matching transaction
    ///
    /// Returns `None` if the transaction does not exist in the pool
    fn get_transaction(&self, hash: TxHash) -> Option<PendingTransaction> {
        if let Some(pending) = self.pending_transactions.get(&hash) {
            return Some(pending.transaction.pending_transaction.clone())
        }
        Some(
            self.ready_transactions.get(&hash)?.transaction.transaction.pending_transaction.clone(),
        )
    }

    /// Returns true if this pool already contains the transaction
    fn contains(&self, tx_hash: &TxHash) -> bool {
        self.pending_transactions.contains(tx_hash) || self.ready_transactions.contains(tx_hash)
    }

    fn add_transaction(&mut self, tx: PoolTransaction) -> Result<AddedTransaction, PoolError> {
        if self.contains(tx.hash()) {
            warn!(target: "txpool", "[{:?}] Already imported", tx.hash());
            return Err(PoolError::AlreadyImported(Box::new(tx)))
        }

        let tx = PendingPoolTransaction::new(tx, self.ready_transactions.provided_markers());
        trace!(target: "txpool", "[{:?}] {:?}", tx.transaction.hash(), tx);

        // If all markers are not satisfied import to future
        if !tx.is_ready() {
            let hash = *tx.transaction.hash();
            self.pending_transactions.add_transaction(tx)?;
            return Ok(AddedTransaction::Pending { hash })
        }
        self.add_ready_transaction(tx)
    }

    /// Adds the transaction to the ready queue
    fn add_ready_transaction(
        &mut self,
        tx: PendingPoolTransaction,
    ) -> Result<AddedTransaction, PoolError> {
        let hash = *tx.transaction.hash();
        trace!(target: "txpool", "adding ready transaction [{:?}]", hash);
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
                Ok(replaced_transactions) => {
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

    /// Prunes the transactions that provide the given markers
    ///
    /// This will effectively remove those transactions that satisfy the markers and transactions
    /// from the pending queue might get promoted to if the markers unlock them.
    pub fn prune_markers(&mut self, markers: impl IntoIterator<Item = TxMarker>) -> PruneResult {
        let mut imports = vec![];
        let mut pruned = vec![];

        for marker in markers {
            // mark as satisfied and store the transactions that got unlocked
            imports.extend(self.pending_transactions.mark_and_unlock(Some(&marker)));
            // prune transactions
            pruned.extend(self.ready_transactions.prune_tags(marker.clone()));
        }

        let mut promoted = vec![];
        let mut failed = vec![];
        for tx in imports {
            let hash = *tx.transaction.hash();
            match self.add_ready_transaction(tx) {
                Ok(res) => promoted.push(res),
                Err(e) => {
                    warn!(target: "txpool", "Failed to promote tx [{:?}] : {:?}", hash, e);
                    failed.push(hash)
                }
            }
        }

        PruneResult { pruned, failed, promoted }
    }

    /// Remove the given transactions from the pool
    pub fn remove_invalid(&mut self, tx_hashes: Vec<TxHash>) -> Vec<Arc<PoolTransaction>> {
        // early exit in case there is no invalid transactions.
        if tx_hashes.is_empty() {
            return vec![]
        }
        trace!(target: "txpool", "Removing invalid transactions: {:?}", tx_hashes);

        let mut removed = self.ready_transactions.remove_with_markers(tx_hashes.clone(), None);
        removed.extend(self.pending_transactions.remove(tx_hashes));

        trace!(target: "txpool", "Removed invalid transactions: {:?}", removed);

        removed
    }
}

/// Represents the outcome of a prune
pub struct PruneResult {
    /// a list of added transactions that a pruned marker satisfied
    pub promoted: Vec<AddedTransaction>,
    /// all transactions that  failed to be promoted and now are discarded
    pub failed: Vec<TxHash>,
    /// all transactions that were pruned from the ready pool
    pub pruned: Vec<Arc<PoolTransaction>>,
}

impl fmt::Debug for PruneResult {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "PruneResult {{ ")?;
        write!(
            fmt,
            "promoted: {:?}, ",
            self.promoted.iter().map(|tx| *tx.hash()).collect::<Vec<_>>()
        )?;
        write!(fmt, "failed: {:?}, ", self.failed)?;
        write!(
            fmt,
            "pruned: {:?}, ",
            self.pruned.iter().map(|tx| *tx.pending_transaction.hash()).collect::<Vec<_>>()
        )?;
        write!(fmt, "}}")?;
        Ok(())
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

impl AddedTransaction {
    pub fn hash(&self) -> &TxHash {
        match self {
            AddedTransaction::Ready(tx) => &tx.hash,
            AddedTransaction::Pending { hash } => hash,
        }
    }
}
