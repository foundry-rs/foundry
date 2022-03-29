use crate::eth::{error::PoolError, util::hex_fmt_many};
use ethers::types::{Address, TxHash};
use forge_node_core::eth::transaction::PendingTransaction;
use parking_lot::RwLock;
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Instant,
};

/// A unique identifying marker for a transaction
pub type TxMarker = Vec<u8>;

/// creates an unique identifier for aan (`nonce` + `Address`) combo
pub fn to_marker(nonce: u64, from: Address) -> TxMarker {
    let mut data = [0u8; 28];
    data[..8].copy_from_slice(&nonce.to_le_bytes()[..]);
    data[8..].copy_from_slice(&from.0[..]);
    data.to_vec()
}

/// Internal Transaction type
#[derive(Clone, PartialEq)]
pub struct PoolTransaction {
    /// the pending eth transaction
    pub pending_transaction: PendingTransaction,
    /// Markers required by the transaction
    pub requires: Vec<TxMarker>,
    /// Markers that this transaction provides
    pub provides: Vec<TxMarker>,
}

// == impl PoolTransaction ==

impl PoolTransaction {
    /// Returns the hash of this transaction
    pub fn hash(&self) -> &TxHash {
        self.pending_transaction.hash()
    }
}

impl fmt::Debug for PoolTransaction {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "Transaction {{ ")?;
        write!(fmt, "hash: {:?}, ", &self.pending_transaction.hash())?;
        write!(fmt, "requires: [{}], ", hex_fmt_many(self.requires.iter()))?;
        write!(fmt, "provides: [{}], ", hex_fmt_many(self.provides.iter()))?;
        write!(fmt, "raw tx: {:?}", &self.pending_transaction)?;
        write!(fmt, "}}")?;
        Ok(())
    }
}

/// A waiting pool of transaction that are pending, but not yet ready to be included in a new block.
///
/// Keeps a set of transactions that are waiting for other transactions
#[derive(Debug, Clone, Default)]
pub struct PendingTransactions {
    /// markers that aren't yet provided by any transaction
    required_markers: HashMap<TxMarker, HashSet<TxHash>>,
    /// the transactions that are not ready yet are waiting for another tx to finish
    waiting_queue: HashMap<TxHash, PendingPoolTransaction>,
}

// == impl PendingTransactions ==

impl PendingTransactions {
    /// Adds a transaction to Pending queue of transactions
    pub fn add_transaction(&mut self, tx: PendingPoolTransaction) {
        assert!(!tx.is_ready(), "transaction must not be ready");
        assert!(
            !self.waiting_queue.contains_key(tx.transaction.hash()),
            "transaction is already added"
        );

        // add all missing markers
        for marker in &tx.missing_markers {
            self.required_markers.entry(marker.clone()).or_default().insert(*tx.transaction.hash());
        }
        // add tx to the queue
        self.waiting_queue.insert(*tx.transaction.hash(), tx);
    }

    /// Returns true if given transaction is part of the queue
    pub fn contains(&self, hash: &TxHash) -> bool {
        self.waiting_queue.contains_key(hash)
    }

    /// This will check off the markers of pending transactions.
    ///
    /// Returns the those transactions that become unlocked (all markers checked) and can be moved
    /// to the ready queue.
    pub fn mark_and_unlock(
        &mut self,
        markers: impl IntoIterator<Item = impl AsRef<TxMarker>>,
    ) -> Vec<PendingPoolTransaction> {
        let mut unlocked_ready = Vec::new();
        for mark in markers {
            let mark = mark.as_ref();
            if let Some(tx_hashes) = self.required_markers.remove(mark) {
                for hash in tx_hashes {
                    let tx = self.waiting_queue.get_mut(&hash).expect("tx is included;");
                    tx.mark(mark);

                    if tx.is_ready() {
                        let tx = self.waiting_queue.remove(&hash).expect("tx is included;");
                        unlocked_ready.push(tx);
                    }
                }
            }
        }

        unlocked_ready
    }
}

/// A transaction in the poo
#[derive(Clone)]
pub struct PendingPoolTransaction {
    pub transaction: Arc<PoolTransaction>,
    /// markers required and have not been satisfied yet by other transactions in the pool
    pub missing_markers: HashSet<TxMarker>,
    /// timestamp when the tx was added
    pub added_at: Instant,
}

// == impl PendingTransaction ==

impl PendingPoolTransaction {
    /// Creates a new `PendingTransaction`.
    ///
    /// Determines the markers that are still missing before this transaction can be moved to the
    /// ready queue.
    pub fn new(transaction: PoolTransaction, provided: &HashMap<TxMarker, TxHash>) -> Self {
        let missing_markers = transaction
            .requires
            .iter()
            .filter(|marker| {
                // is true if the marker is already satisfied either via transaction in the pool
                provided.contains_key(&**marker)
            })
            .cloned()
            .collect();

        Self { transaction: Arc::new(transaction), missing_markers, added_at: Instant::now() }
    }

    /// Removes the required marker
    pub fn mark(&mut self, marker: &TxMarker) {
        self.missing_markers.remove(marker);
    }

    /// Returns true if transaction has all requirements satisfied.
    pub fn is_ready(&self) -> bool {
        self.missing_markers.is_empty()
    }
}

impl fmt::Debug for PendingPoolTransaction {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "PendingTransaction {{ ")?;
        write!(fmt, "added_at: {:?}, ", self.added_at)?;
        write!(fmt, "tx: {:?}, ", self.transaction)?;
        write!(fmt, "missing_markers: {{{}}}", hex_fmt_many(self.missing_markers.iter()))?;
        write!(fmt, "}}")
    }
}

pub struct TransactionsIterator {
    all: HashMap<TxHash, ReadyTransaction>,
    awaiting: HashMap<TxHash, (usize, PoolTransactionRef)>,
    independent: BTreeSet<PoolTransactionRef>,
    invalid: HashSet<TxHash>,
}

/// transactions that are ready to be included in a block.
#[derive(Debug, Clone, Default)]
pub struct ReadyTransactions {
    /// keeps track of transactions inserted in the pool
    ///
    /// this way we can determine when transactions where submitted to the pool
    id: u64,
    /// markers that are provided by `ReadyTransaction`s
    provided_markers: HashMap<TxMarker, TxHash>,
    /// transactions that are ready
    ready_tx: Arc<RwLock<HashMap<TxHash, ReadyTransaction>>>,
    /// independent transactions that can be included directly and don't require other transactions
    /// Sorted by their id
    independent_transactions: BTreeSet<PoolTransactionRef>,
}

// == impl ReadyTransactions ==

impl ReadyTransactions {
    /// Returns an iterator over all transactions
    pub fn get_transactions(&self) -> TransactionsIterator {
        TransactionsIterator {
            all: self.ready_tx.read().clone(),
            independent: self.independent_transactions.clone(),
            awaiting: Default::default(),
            invalid: Default::default(),
        }
    }

    /// Returns true if the transaction is part of the queue.
    pub fn contains(&self, hash: &TxHash) -> bool {
        self.ready_tx.read().contains_key(hash)
    }

    pub fn provided_markers(&self) -> &HashMap<TxMarker, TxHash> {
        &self.provided_markers
    }

    fn next_id(&mut self) -> u64 {
        let id = self.id;
        self.id = self.id.wrapping_add(1);
        id
    }

    /// Adds a new transactions to the ready queue
    ///
    /// # Panics
    ///
    /// if the pending transaction is not ready: [PendingTransaction::is_ready()]
    /// or the transaction is already included
    pub fn add_transaction(
        &mut self,
        tx: PendingPoolTransaction,
    ) -> Result<Vec<Arc<PoolTransaction>>, PoolError> {
        assert!(tx.is_ready(), "transaction must be ready",);
        assert!(
            !self.ready_tx.read().contains_key(tx.transaction.hash()),
            "transaction already included"
        );

        let id = self.next_id();
        let hash = *tx.transaction.hash();
        let (replaced_tx, unlocks) = self.replaced_transactions(&tx.transaction)?;

        let mut independent = true;
        let mut ready = self.ready_tx.write();
        // Add links to transactions that unlock the current one
        for mark in &tx.transaction.requires {
            // Check if the transaction that satisfies the mark is still in the queue.
            if let Some(other) = self.provided_markers.get(mark) {
                let tx = ready.get_mut(other).expect("hash included;");
                tx.unlocks.push(hash);
                // tx still depends on other tx
                independent = false;
            }
        }

        // update markers
        for mark in tx.transaction.provides.iter().cloned() {
            self.provided_markers.insert(mark, hash);
        }

        let transaction = PoolTransactionRef { id, transaction: tx.transaction };

        // add to the independent set
        if independent {
            self.independent_transactions.insert(transaction.clone());
        }

        // insert to ready queue
        ready.insert(hash, ReadyTransaction { transaction, unlocks });

        Ok(replaced_tx)
    }

    /// Removes and returns those transactions that got replaced by the `tx`
    fn replaced_transactions(
        &mut self,
        tx: &PoolTransaction,
    ) -> Result<(Vec<Arc<PoolTransaction>>, Vec<TxHash>), PoolError> {
        // check if we are replacing transactions
        let remove_hashes: HashSet<_> =
            tx.provides.iter().filter_map(|mark| self.provided_markers.get(mark)).collect();

        // early exit if we are not replacing anything.
        if remove_hashes.is_empty() {
            return Ok((Vec::new(), Vec::new()))
        }

        let mut unlocked_tx = Vec::new();
        {
            // construct a list of unlocked transactions
            let ready = self.ready_tx.read();
            for tx in remove_hashes.iter().filter_map(|hash| ready.get(hash)) {
                unlocked_tx.extend(tx.unlocks.iter().cloned())
            }
        }

        let remove_hashes = remove_hashes.into_iter().copied().collect::<Vec<_>>();

        let new_provides = tx.provides.iter().cloned().collect::<HashSet<_>>();
        let removed_tx = self.remove_with_markers(remove_hashes, Some(new_provides));

        Ok((removed_tx, unlocked_tx))
    }

    /// Removes the transactions from the ready queue and returns the removed transactions.
    /// This will also remove all transactions that depend on those.
    pub fn clear_transactions(&mut self, tx_hashes: &[TxHash]) -> Vec<Arc<PoolTransaction>> {
        self.remove_with_markers(tx_hashes.to_vec(), None)
    }

    /// Removes transactions and those that depend on them and satisfy at least one marker in the
    /// given filter set.
    fn remove_with_markers(
        &mut self,
        mut tx_hashes: Vec<TxHash>,
        marker_filter: Option<HashSet<TxMarker>>,
    ) -> Vec<Arc<PoolTransaction>> {
        let mut removed = Vec::new();
        let mut ready = self.ready_tx.write();

        while let Some(hash) = tx_hashes.pop() {
            if let Some(mut tx) = ready.remove(&hash) {
                let invalidated = tx.transaction.transaction.provides.iter().filter(|mark| {
                    marker_filter.as_ref().map(|filter| !filter.contains(&**mark)).unwrap_or(true)
                });

                let mut removed_some_marks = false;
                // remove entries from provided_markers
                for mark in invalidated {
                    removed_some_marks = true;
                    self.provided_markers.remove(mark);
                }

                // remove from unlocks
                for mark in &tx.transaction.transaction.requires {
                    if let Some(hash) = self.provided_markers.get(mark) {
                        if let Some(tx) = ready.get_mut(hash) {
                            if let Some(idx) = tx.unlocks.iter().position(|i| i == hash) {
                                tx.unlocks.swap_remove(idx);
                            }
                        }
                    }
                }

                // remove from the independent set
                self.independent_transactions.remove(&tx.transaction);

                if removed_some_marks {
                    // remove all transactions that the current one unlocks
                    tx_hashes.append(&mut tx.unlocks);
                }

                // remove transaction
                removed.push(tx.transaction.transaction);
            }
        }

        removed
    }
}

/// A reference to a transaction in the pool
#[derive(Debug, Clone)]
pub struct PoolTransactionRef {
    /// actual transaction
    pub transaction: Arc<PoolTransaction>,
    /// identifier used to internally compare the transaction in the pool
    pub id: u64,
}

impl Eq for PoolTransactionRef {}

impl PartialEq<Self> for PoolTransactionRef {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl PartialOrd<Self> for PoolTransactionRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PoolTransactionRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

#[derive(Debug, Clone)]
struct ReadyTransaction {
    /// ref to the actual transaction
    pub transaction: PoolTransactionRef,
    /// tracks the transactions that get unlocked by this transaction
    pub unlocks: Vec<TxHash>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_id_txs() {
        let addr = Address::random();
        assert_eq!(to_marker(1, addr), to_marker(1, addr));
        assert_ne!(to_marker(2, addr), to_marker(1, addr));
    }
}
