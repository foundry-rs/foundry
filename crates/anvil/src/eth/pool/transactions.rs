use crate::eth::{error::PoolError, util::hex_fmt_many};
use alloy_consensus::{
    Transaction, Typed2718,
    crypto::RecoveryError,
    transaction::{SignerRecoverable, TxHashRef},
};
use alloy_network::AnyRpcTransaction;
use alloy_primitives::{
    Address, TxHash,
    map::{HashMap, HashSet},
};
use alloy_rlp::Encodable;
use anvil_core::eth::transaction::PendingTransaction;
use parking_lot::RwLock;
use std::{cmp::Ordering, collections::BTreeSet, fmt, str::FromStr, sync::Arc, time::Instant};

/// A unique identifying marker for a transaction
pub type TxMarker = Vec<u8>;

/// Result type for replaced transactions: the replaced pool transactions and the hashes they
/// unlock.
type ReplacedTransactions<T> = (Vec<Arc<PoolTransaction<T>>>, Vec<TxHash>);

/// creates an unique identifier for aan (`nonce` + `Address`) combo
pub fn to_marker(nonce: u64, from: Address) -> TxMarker {
    let mut data = [0u8; 28];
    data[..8].copy_from_slice(&nonce.to_le_bytes()[..]);
    data[8..].copy_from_slice(&from.0[..]);
    data.to_vec()
}

/// Modes that determine the transaction ordering of the mempool
///
/// This type controls the transaction order via the priority metric of a transaction
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransactionOrder {
    /// Keep the pool transaction transactions sorted in the order they arrive.
    ///
    /// This will essentially assign every transaction the exact priority so the order is
    /// determined by their internal id
    Fifo,
    /// This means that it prioritizes transactions based on the fees paid to the miner.
    #[default]
    Fees,
}

impl TransactionOrder {
    /// Returns the priority of the transactions
    pub fn priority<T: Transaction>(&self, tx: &T) -> TransactionPriority {
        match self {
            Self::Fifo => TransactionPriority::default(),
            Self::Fees => TransactionPriority(tx.max_fee_per_gas()),
        }
    }
}

impl FromStr for TransactionOrder {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let order = match s.as_str() {
            "fees" => Self::Fees,
            "fifo" => Self::Fifo,
            _ => return Err(format!("Unknown TransactionOrder: `{s}`")),
        };
        Ok(order)
    }
}

/// Metric value for the priority of a transaction.
///
/// The `TransactionPriority` determines the ordering of two transactions that have all their
/// markers satisfied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransactionPriority(pub u128);

/// Internal Transaction type
#[derive(Clone, PartialEq, Eq)]
pub struct PoolTransaction<T> {
    /// the pending eth transaction
    pub pending_transaction: PendingTransaction<T>,
    /// Markers required by the transaction
    pub requires: Vec<TxMarker>,
    /// Markers that this transaction provides
    pub provides: Vec<TxMarker>,
    /// priority of the transaction
    pub priority: TransactionPriority,
}

// == impl PoolTransaction ==

impl<T> PoolTransaction<T> {
    pub fn new(transaction: PendingTransaction<T>) -> Self {
        Self {
            pending_transaction: transaction,
            requires: vec![],
            provides: vec![],
            priority: TransactionPriority(0),
        }
    }

    /// Returns the hash of this transaction
    pub fn hash(&self) -> TxHash {
        *self.pending_transaction.hash()
    }
}

impl<T: Transaction> PoolTransaction<T> {
    /// Returns the max fee per gas of this transaction
    pub fn max_fee_per_gas(&self) -> u128 {
        self.pending_transaction.transaction.max_fee_per_gas()
    }
}

impl<T: Typed2718> PoolTransaction<T> {
    /// Returns the type of the transaction
    pub fn tx_type(&self) -> u8 {
        self.pending_transaction.transaction.ty()
    }
}

impl<T: fmt::Debug> fmt::Debug for PoolTransaction<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "Transaction {{ ")?;
        write!(fmt, "hash: {:?}, ", &self.pending_transaction.hash())?;
        write!(fmt, "requires: [{}], ", hex_fmt_many(self.requires.iter()))?;
        write!(fmt, "provides: [{}], ", hex_fmt_many(self.provides.iter()))?;
        write!(fmt, "raw tx: {:?}", &self.pending_transaction)?;
        write!(fmt, "}}")?;
        Ok(())
    }
}

impl<T> TryFrom<AnyRpcTransaction> for PoolTransaction<T>
where
    T: SignerRecoverable + TxHashRef + Encodable + TryFrom<AnyRpcTransaction>,
    <T as TryFrom<AnyRpcTransaction>>::Error: Into<eyre::Error>,
    RecoveryError: Into<eyre::Error>,
{
    type Error = eyre::Error;
    fn try_from(value: AnyRpcTransaction) -> Result<Self, Self::Error> {
        let typed_transaction = T::try_from(value).map_err(Into::into)?;
        let pending_transaction = PendingTransaction::new(typed_transaction)?;
        Ok(Self {
            pending_transaction,
            requires: vec![],
            provides: vec![],
            priority: TransactionPriority(0),
        })
    }
}

/// A waiting pool of transaction that are pending, but not yet ready to be included in a new block.
///
/// Keeps a set of transactions that are waiting for other transactions
#[derive(Clone, Debug)]
pub struct PendingTransactions<T> {
    /// markers that aren't yet provided by any transaction
    required_markers: HashMap<TxMarker, HashSet<TxHash>>,
    /// mapping of the markers of a transaction to the hash of the transaction
    waiting_markers: HashMap<Vec<TxMarker>, TxHash>,
    /// the transactions that are not ready yet are waiting for another tx to finish
    waiting_queue: HashMap<TxHash, PendingPoolTransaction<T>>,
}

impl<T> Default for PendingTransactions<T> {
    fn default() -> Self {
        Self {
            required_markers: Default::default(),
            waiting_markers: Default::default(),
            waiting_queue: Default::default(),
        }
    }
}

impl<T> PendingTransactions<T> {
    /// Returns the number of transactions that are currently waiting
    pub fn len(&self) -> usize {
        self.waiting_queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiting_queue.is_empty()
    }

    /// Clears internal state
    pub fn clear(&mut self) {
        self.required_markers.clear();
        self.waiting_markers.clear();
        self.waiting_queue.clear();
    }

    /// Returns an iterator over all transactions in the waiting pool
    pub fn transactions(&self) -> impl Iterator<Item = Arc<PoolTransaction<T>>> + '_ {
        self.waiting_queue.values().map(|tx| tx.transaction.clone())
    }

    /// Returns true if given transaction is part of the queue
    pub fn contains(&self, hash: &TxHash) -> bool {
        self.waiting_queue.contains_key(hash)
    }

    /// Returns the transaction for the hash if it's pending
    pub fn get(&self, hash: &TxHash) -> Option<&PendingPoolTransaction<T>> {
        self.waiting_queue.get(hash)
    }

    /// This will check off the markers of pending transactions.
    ///
    /// Returns the those transactions that become unlocked (all markers checked) and can be moved
    /// to the ready queue.
    pub fn mark_and_unlock(
        &mut self,
        markers: impl IntoIterator<Item = impl AsRef<TxMarker>>,
    ) -> Vec<PendingPoolTransaction<T>> {
        let mut unlocked_ready = Vec::new();
        for mark in markers {
            let mark = mark.as_ref();
            if let Some(tx_hashes) = self.required_markers.remove(mark) {
                for hash in tx_hashes {
                    let tx = self.waiting_queue.get_mut(&hash).expect("tx is included;");
                    tx.mark(mark);

                    if tx.is_ready() {
                        let tx = self.waiting_queue.remove(&hash).expect("tx is included;");
                        self.waiting_markers.remove(&tx.transaction.provides);

                        unlocked_ready.push(tx);
                    }
                }
            }
        }

        unlocked_ready
    }

    /// Removes the transactions associated with the given hashes
    ///
    /// Returns all removed transactions.
    pub fn remove(&mut self, hashes: Vec<TxHash>) -> Vec<Arc<PoolTransaction<T>>> {
        let mut removed = vec![];
        for hash in hashes {
            if let Some(waiting_tx) = self.waiting_queue.remove(&hash) {
                self.waiting_markers.remove(&waiting_tx.transaction.provides);
                for marker in waiting_tx.missing_markers {
                    let remove = if let Some(required) = self.required_markers.get_mut(&marker) {
                        required.remove(&hash);
                        required.is_empty()
                    } else {
                        false
                    };
                    if remove {
                        self.required_markers.remove(&marker);
                    }
                }
                removed.push(waiting_tx.transaction)
            }
        }
        removed
    }
}

impl<T: Transaction> PendingTransactions<T> {
    /// Adds a transaction to Pending queue of transactions
    pub fn add_transaction(&mut self, tx: PendingPoolTransaction<T>) -> Result<(), PoolError> {
        assert!(!tx.is_ready(), "transaction must not be ready");
        assert!(
            !self.waiting_queue.contains_key(&tx.transaction.hash()),
            "transaction is already added"
        );

        if let Some(replace) = self
            .waiting_markers
            .get(&tx.transaction.provides)
            .and_then(|hash| self.waiting_queue.get(hash))
        {
            // check if underpriced
            if tx.transaction.max_fee_per_gas() <= replace.transaction.max_fee_per_gas() {
                warn!(target: "txpool", "pending replacement transaction underpriced [{:?}]", tx.transaction.hash());
                return Err(PoolError::ReplacementUnderpriced(tx.transaction.hash()));
            }
        }

        // add all missing markers
        for marker in &tx.missing_markers {
            self.required_markers.entry(marker.clone()).or_default().insert(tx.transaction.hash());
        }

        // also track identifying markers
        self.waiting_markers.insert(tx.transaction.provides.clone(), tx.transaction.hash());
        // add tx to the queue
        self.waiting_queue.insert(tx.transaction.hash(), tx);

        Ok(())
    }
}

/// A transaction in the pool
#[derive(Clone)]
pub struct PendingPoolTransaction<T> {
    pub transaction: Arc<PoolTransaction<T>>,
    /// markers required and have not been satisfied yet by other transactions in the pool
    pub missing_markers: HashSet<TxMarker>,
    /// timestamp when the tx was added
    pub added_at: Instant,
}

impl<T> PendingPoolTransaction<T> {
    /// Creates a new `PendingPoolTransaction`.
    ///
    /// Determines the markers that are still missing before this transaction can be moved to the
    /// ready queue.
    pub fn new(transaction: PoolTransaction<T>, provided: &HashMap<TxMarker, TxHash>) -> Self {
        let missing_markers = transaction
            .requires
            .iter()
            .filter(|marker| {
                // is true if the marker is already satisfied either via transaction in the pool
                !provided.contains_key(&**marker)
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

impl<T: fmt::Debug> fmt::Debug for PendingPoolTransaction<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "PendingTransaction {{ ")?;
        write!(fmt, "added_at: {:?}, ", self.added_at)?;
        write!(fmt, "tx: {:?}, ", self.transaction)?;
        write!(fmt, "missing_markers: {{{}}}", hex_fmt_many(self.missing_markers.iter()))?;
        write!(fmt, "}}")
    }
}

pub struct TransactionsIterator<T> {
    all: HashMap<TxHash, ReadyTransaction<T>>,
    awaiting: HashMap<TxHash, (usize, PoolTransactionRef<T>)>,
    independent: BTreeSet<PoolTransactionRef<T>>,
    _invalid: HashSet<TxHash>,
}

impl<T> TransactionsIterator<T> {
    /// Depending on number of satisfied requirements insert given ref
    /// either to awaiting set or to best set.
    fn independent_or_awaiting(&mut self, satisfied: usize, tx_ref: PoolTransactionRef<T>) {
        if satisfied >= tx_ref.transaction.requires.len() {
            // If we have satisfied all deps insert to best
            self.independent.insert(tx_ref);
        } else {
            // otherwise we're still awaiting for some deps
            self.awaiting.insert(tx_ref.transaction.hash(), (satisfied, tx_ref));
        }
    }
}

impl<T> Iterator for TransactionsIterator<T> {
    type Item = Arc<PoolTransaction<T>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let best = self.independent.iter().next_back()?.clone();
            let best = self.independent.take(&best)?;
            let hash = best.transaction.hash();

            let ready =
                if let Some(ready) = self.all.get(&hash).cloned() { ready } else { continue };

            // Insert transactions that just got unlocked.
            for hash in &ready.unlocks {
                // first check local awaiting transactions
                let res = if let Some((mut satisfied, tx_ref)) = self.awaiting.remove(hash) {
                    satisfied += 1;
                    Some((satisfied, tx_ref))
                    // then get from the pool
                } else {
                    self.all
                        .get(hash)
                        .map(|next| (next.requires_offset + 1, next.transaction.clone()))
                };
                if let Some((satisfied, tx_ref)) = res {
                    self.independent_or_awaiting(satisfied, tx_ref)
                }
            }

            return Some(best.transaction);
        }
    }
}

/// transactions that are ready to be included in a block.
#[derive(Clone, Debug)]
pub struct ReadyTransactions<T> {
    /// keeps track of transactions inserted in the pool
    ///
    /// this way we can determine when transactions where submitted to the pool
    id: u64,
    /// markers that are provided by `ReadyTransaction`s
    provided_markers: HashMap<TxMarker, TxHash>,
    /// transactions that are ready
    ready_tx: Arc<RwLock<HashMap<TxHash, ReadyTransaction<T>>>>,
    /// independent transactions that can be included directly and don't require other transactions
    /// Sorted by their id
    independent_transactions: BTreeSet<PoolTransactionRef<T>>,
}

impl<T> Default for ReadyTransactions<T> {
    fn default() -> Self {
        Self {
            id: 0,
            provided_markers: Default::default(),
            ready_tx: Default::default(),
            independent_transactions: Default::default(),
        }
    }
}

impl<T> ReadyTransactions<T> {
    /// Returns an iterator over all transactions
    pub fn get_transactions(&self) -> TransactionsIterator<T> {
        TransactionsIterator {
            all: self.ready_tx.read().clone(),
            independent: self.independent_transactions.clone(),
            awaiting: Default::default(),
            _invalid: Default::default(),
        }
    }

    /// Clears the internal state
    pub fn clear(&mut self) {
        self.provided_markers.clear();
        self.ready_tx.write().clear();
        self.independent_transactions.clear();
    }

    /// Returns true if the transaction is part of the queue.
    pub fn contains(&self, hash: &TxHash) -> bool {
        self.ready_tx.read().contains_key(hash)
    }

    /// Returns the number of ready transactions without cloning the snapshot
    pub fn len(&self) -> usize {
        self.ready_tx.read().len()
    }

    /// Returns true if there are no ready transactions
    pub fn is_empty(&self) -> bool {
        self.ready_tx.read().is_empty()
    }

    /// Returns the transaction for the hash if it's in the ready pool but not yet mined
    pub fn get(&self, hash: &TxHash) -> Option<ReadyTransaction<T>> {
        self.ready_tx.read().get(hash).cloned()
    }

    pub fn provided_markers(&self) -> &HashMap<TxMarker, TxHash> {
        &self.provided_markers
    }

    fn next_id(&mut self) -> u64 {
        let id = self.id;
        self.id = self.id.wrapping_add(1);
        id
    }

    /// Removes the transactions from the ready queue and returns the removed transactions.
    /// This will also remove all transactions that depend on those.
    pub fn clear_transactions(&mut self, tx_hashes: &[TxHash]) -> Vec<Arc<PoolTransaction<T>>> {
        self.remove_with_markers(tx_hashes.to_vec(), None)
    }

    /// Removes the transactions that provide the marker
    ///
    /// This will also remove all transactions that lead to the transaction that provides the
    /// marker.
    pub fn prune_tags(&mut self, marker: TxMarker) -> Vec<Arc<PoolTransaction<T>>> {
        let mut removed_tx = vec![];

        // the markers to remove
        let mut remove = vec![marker];

        while let Some(marker) = remove.pop() {
            let res = self
                .provided_markers
                .remove(&marker)
                .and_then(|hash| self.ready_tx.write().remove(&hash));

            if let Some(tx) = res {
                let unlocks = tx.unlocks;
                self.independent_transactions.remove(&tx.transaction);
                let tx = tx.transaction.transaction;

                // also prune previous transactions
                {
                    let hash = tx.hash();
                    let mut ready = self.ready_tx.write();

                    let mut previous_markers = |marker| -> Option<Vec<TxMarker>> {
                        let prev_hash = self.provided_markers.get(marker)?;
                        let tx2 = ready.get_mut(prev_hash)?;
                        // remove hash
                        if let Some(idx) = tx2.unlocks.iter().position(|i| i == &hash) {
                            tx2.unlocks.swap_remove(idx);
                        }
                        tx2.unlocks.is_empty().then(|| tx2.transaction.transaction.provides.clone())
                    };

                    // find previous transactions
                    for marker in &tx.requires {
                        if let Some(mut tags_to_remove) = previous_markers(marker) {
                            remove.append(&mut tags_to_remove);
                        }
                    }
                }

                // add the transactions that just got unlocked to independent set
                for hash in unlocks {
                    if let Some(tx) = self.ready_tx.write().get_mut(&hash) {
                        tx.requires_offset += 1;
                        if tx.requires_offset == tx.transaction.transaction.requires.len() {
                            self.independent_transactions.insert(tx.transaction.clone());
                        }
                    }
                }
                // finally, remove the markers that this transaction provides
                let current_marker = &marker;
                for marker in &tx.provides {
                    let removed = self.provided_markers.remove(marker);
                    assert_eq!(
                        removed,
                        if current_marker == marker { None } else { Some(tx.hash()) },
                        "The pool contains exactly one transaction providing given tag; the removed transaction
						claims to provide that tag, so it has to be mapped to it's hash; qed"
                    );
                }
                removed_tx.push(tx);
            }
        }

        removed_tx
    }

    /// Removes transactions and those that depend on them and satisfy at least one marker in the
    /// given filter set.
    pub fn remove_with_markers(
        &mut self,
        mut tx_hashes: Vec<TxHash>,
        marker_filter: Option<HashSet<TxMarker>>,
    ) -> Vec<Arc<PoolTransaction<T>>> {
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
                    if let Some(provider_hash) = self.provided_markers.get(mark)
                        && let Some(provider_tx) = ready.get_mut(provider_hash)
                        && let Some(idx) = provider_tx.unlocks.iter().position(|i| i == &hash)
                    {
                        provider_tx.unlocks.swap_remove(idx);
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

impl<T: Transaction> ReadyTransactions<T> {
    /// Adds a new transactions to the ready queue.
    ///
    /// # Panics
    ///
    /// If the pending transaction is not ready ([`PendingPoolTransaction::is_ready`])
    /// or the transaction is already included.
    pub fn add_transaction(
        &mut self,
        tx: PendingPoolTransaction<T>,
    ) -> Result<Vec<Arc<PoolTransaction<T>>>, PoolError> {
        assert!(tx.is_ready(), "transaction must be ready",);
        assert!(
            !self.ready_tx.read().contains_key(&tx.transaction.hash()),
            "transaction already included"
        );

        let (replaced_tx, unlocks) = self.replaced_transactions(&tx.transaction)?;

        let id = self.next_id();
        let hash = tx.transaction.hash();

        let mut independent = true;
        let mut requires_offset = 0;
        let mut ready = self.ready_tx.write();
        // Add links to transactions that unlock the current one
        for mark in &tx.transaction.requires {
            // Check if the transaction that satisfies the mark is still in the queue.
            if let Some(other) = self.provided_markers.get(mark) {
                let tx = ready.get_mut(other).expect("hash included;");
                tx.unlocks.push(hash);
                // tx still depends on other tx
                independent = false;
            } else {
                requires_offset += 1;
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
        ready.insert(hash, ReadyTransaction { transaction, unlocks, requires_offset });

        Ok(replaced_tx)
    }

    /// Removes and returns those transactions that got replaced by the `tx`
    fn replaced_transactions(
        &mut self,
        tx: &PoolTransaction<T>,
    ) -> Result<ReplacedTransactions<T>, PoolError> {
        // check if we are replacing transactions
        let remove_hashes: HashSet<_> =
            tx.provides.iter().filter_map(|mark| self.provided_markers.get(mark)).collect();

        // early exit if we are not replacing anything.
        if remove_hashes.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        // check if we're replacing the same transaction and if it can be replaced
        let mut unlocked_tx = Vec::new();
        {
            // construct a list of unlocked transactions
            // also check for transactions that shouldn't be replaced because underpriced
            let ready = self.ready_tx.read();
            for to_remove in remove_hashes.iter().filter_map(|hash| ready.get(*hash)) {
                // if we're attempting to replace a transaction that provides the exact same markers
                // (addr + nonce) then we check for gas price
                if to_remove.provides() == tx.provides {
                    // check if underpriced
                    if tx.pending_transaction.transaction.max_fee_per_gas()
                        <= to_remove.max_fee_per_gas()
                    {
                        warn!(target: "txpool", "ready replacement transaction underpriced [{:?}]", tx.hash());
                        return Err(PoolError::ReplacementUnderpriced(tx.hash()));
                    }
                    trace!(target: "txpool", "replacing ready transaction [{:?}] with higher gas price [{:?}]", to_remove.transaction.transaction.hash(), tx.hash());
                }

                unlocked_tx.extend(to_remove.unlocks.iter().copied())
            }
        }

        let remove_hashes = remove_hashes.into_iter().copied().collect::<Vec<_>>();

        let new_provides = tx.provides.iter().cloned().collect::<HashSet<_>>();
        let removed_tx = self.remove_with_markers(remove_hashes, Some(new_provides));

        Ok((removed_tx, unlocked_tx))
    }
}

/// A reference to a transaction in the pool
#[derive(Debug)]
pub struct PoolTransactionRef<T> {
    /// actual transaction
    pub transaction: Arc<PoolTransaction<T>>,
    /// identifier used to internally compare the transaction in the pool
    pub id: u64,
}

impl<T> Clone for PoolTransactionRef<T> {
    fn clone(&self) -> Self {
        Self { transaction: Arc::clone(&self.transaction), id: self.id }
    }
}

impl<T> Eq for PoolTransactionRef<T> {}

impl<T> PartialEq<Self> for PoolTransactionRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<T> PartialOrd<Self> for PoolTransactionRef<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for PoolTransactionRef<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.transaction
            .priority
            .cmp(&other.transaction.priority)
            .then_with(|| other.id.cmp(&self.id))
    }
}

#[derive(Debug)]
pub struct ReadyTransaction<T> {
    /// ref to the actual transaction
    pub transaction: PoolTransactionRef<T>,
    /// tracks the transactions that get unlocked by this transaction
    pub unlocks: Vec<TxHash>,
    /// amount of required markers that are inherently provided
    pub requires_offset: usize,
}

impl<T> Clone for ReadyTransaction<T> {
    fn clone(&self) -> Self {
        Self {
            transaction: self.transaction.clone(),
            unlocks: self.unlocks.clone(),
            requires_offset: self.requires_offset,
        }
    }
}

impl<T> ReadyTransaction<T> {
    pub fn provides(&self) -> &[TxMarker] {
        &self.transaction.transaction.provides
    }
}

impl<T: Transaction> ReadyTransaction<T> {
    pub fn max_fee_per_gas(&self) -> u128 {
        self.transaction.transaction.max_fee_per_gas()
    }
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
