use crate::eth::{
    error::PoolError,
    pool::transactions::{
        PendingPoolTransaction, PendingTransactions, PoolTransaction, ReadyTransactions,
        TransactionsIterator, TxMarker,
    },
};
use ethers::types::{TxHash, U64};
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

    /// notifies all listeners about the transaction
    fn notify_listener(&self, hash: TxHash) {
        let mut listener = self.transaction_listener.lock();
        // this is basically a retain but with mut
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

    /// Returns true if this pool already contains the transaction
    fn contains(&self, tx_hash: &TxHash) -> bool {
        self.pending_transactions.contains(tx_hash) || self.ready_transactions.contains(tx_hash)
    }

    fn add_transaction(&mut self, tx: PoolTransaction) -> Result<AddedTransaction, PoolError> {
        if self.contains(tx.hash()) {
            return Err(PoolError::AlreadyImported(Box::new(tx)))
        }
        let tx = PendingPoolTransaction::new(tx, self.ready_transactions.provided_markers());
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
        tx: PendingPoolTransaction,
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
