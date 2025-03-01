use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

use crate::TransactionResponse;
use alloc::{vec, vec::Vec};
use alloy_eips::Encodable2718;
use core::slice;

/// Block Transactions depending on the boolean attribute of `eth_getBlockBy*`,
/// or if used by `eth_getUncle*`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockTransactions<T> {
    /// Full transactions
    Full(Vec<T>),
    /// Only hashes
    Hashes(Vec<B256>),
    /// Special case for uncle response.
    Uncle,
}

impl<T> Default for BlockTransactions<T> {
    fn default() -> Self {
        Self::Hashes(Vec::default())
    }
}

impl<T> BlockTransactions<T> {
    /// Check if the enum variant is used for hashes.
    #[inline]
    pub const fn is_hashes(&self) -> bool {
        matches!(self, Self::Hashes(_))
    }

    /// Fallibly cast to a slice of hashes.
    pub fn as_hashes(&self) -> Option<&[B256]> {
        match self {
            Self::Hashes(hashes) => Some(hashes),
            _ => None,
        }
    }

    /// Returns true if the enum variant is used for full transactions.
    #[inline]
    pub const fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// Converts the transaction type by applying a function to each transaction.
    ///
    /// Returns the block with the new transaction type.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> BlockTransactions<U> {
        match self {
            Self::Full(txs) => BlockTransactions::Full(txs.into_iter().map(f).collect()),
            Self::Hashes(hashes) => BlockTransactions::Hashes(hashes),
            Self::Uncle => BlockTransactions::Uncle,
        }
    }

    /// Converts the transaction type by applying a fallible function to each transaction.
    ///
    /// Returns the block with the new transaction type if all transactions were successfully.
    pub fn try_map<U, E>(
        self,
        f: impl FnMut(T) -> Result<U, E>,
    ) -> Result<BlockTransactions<U>, E> {
        match self {
            Self::Full(txs) => {
                Ok(BlockTransactions::Full(txs.into_iter().map(f).collect::<Result<_, _>>()?))
            }
            Self::Hashes(hashes) => Ok(BlockTransactions::Hashes(hashes)),
            Self::Uncle => Ok(BlockTransactions::Uncle),
        }
    }

    /// Fallibly cast to a slice of transactions.
    ///
    /// Returns `None` if the enum variant is not `Full`.
    pub fn as_transactions(&self) -> Option<&[T]> {
        match self {
            Self::Full(txs) => Some(txs),
            _ => None,
        }
    }

    /// Calculate the transaction root for the full transactions.
    ///
    /// Returns `None` if this is not the [`BlockTransactions::Full`] variant
    pub fn calculate_transactions_root(&self) -> Option<B256>
    where
        T: Encodable2718,
    {
        self.as_transactions().map(alloy_consensus::proofs::calculate_transaction_root)
    }

    /// Returns true if the enum variant is used for an uncle response.
    #[inline]
    pub const fn is_uncle(&self) -> bool {
        matches!(self, Self::Uncle)
    }

    /// Returns an iterator over the transactions (if any). This will be empty
    /// if the block is an uncle or if the transaction list contains only
    /// hashes.
    #[doc(alias = "transactions")]
    pub fn txns(&self) -> impl Iterator<Item = &T> {
        self.as_transactions().map(|txs| txs.iter()).unwrap_or_else(|| [].iter())
    }

    /// Returns an iterator over the transactions (if any). This will be empty if the block is not
    /// full.
    pub fn into_transactions(self) -> vec::IntoIter<T> {
        match self {
            Self::Full(txs) => txs.into_iter(),
            _ => vec::IntoIter::default(),
        }
    }

    /// Consumes the type and returns the transactions as a vector.
    ///
    /// Note: if this is an uncle or hashes, this will return an empty vector.
    pub fn into_transactions_vec(self) -> Vec<T> {
        match self {
            Self::Full(txs) => txs,
            _ => vec![],
        }
    }

    /// Returns an instance of BlockTransactions with the Uncle special case.
    #[inline]
    pub const fn uncle() -> Self {
        Self::Uncle
    }

    /// Returns the number of transactions.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Hashes(h) => h.len(),
            Self::Full(f) => f.len(),
            Self::Uncle => 0,
        }
    }

    /// Whether the block has no transactions.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: TransactionResponse> BlockTransactions<T> {
    /// Creates a new [`BlockTransactions::Hashes`] variant from the given iterator of transactions.
    pub fn new_hashes(txs: impl IntoIterator<Item = impl AsRef<T>>) -> Self {
        Self::Hashes(txs.into_iter().map(|tx| tx.as_ref().tx_hash()).collect())
    }

    /// Converts `self` into `Hashes`.
    #[inline]
    pub fn convert_to_hashes(&mut self) {
        if !self.is_hashes() {
            *self = Self::Hashes(self.hashes().collect());
        }
    }

    /// Converts `self` into `Hashes`.
    #[inline]
    pub fn into_hashes(mut self) -> Self {
        self.convert_to_hashes();
        self
    }

    /// Returns an iterator over the transaction hashes.
    #[deprecated = "use `hashes` instead"]
    #[inline]
    pub fn iter(&self) -> BlockTransactionHashes<'_, T> {
        self.hashes()
    }

    /// Returns an iterator over references to the transaction hashes.
    #[inline]
    pub fn hashes(&self) -> BlockTransactionHashes<'_, T> {
        BlockTransactionHashes::new(self)
    }
}

impl<T> From<Vec<B256>> for BlockTransactions<T> {
    fn from(hashes: Vec<B256>) -> Self {
        Self::Hashes(hashes)
    }
}

impl<T: TransactionResponse> From<Vec<T>> for BlockTransactions<T> {
    fn from(transactions: Vec<T>) -> Self {
        Self::Full(transactions)
    }
}

/// An iterator over the transaction hashes of a block.
///
/// See [`BlockTransactions::hashes`].
#[derive(Clone, Debug)]
pub struct BlockTransactionHashes<'a, T>(BlockTransactionHashesInner<'a, T>);

#[derive(Clone, Debug)]
enum BlockTransactionHashesInner<'a, T> {
    Hashes(slice::Iter<'a, B256>),
    Full(slice::Iter<'a, T>),
    Uncle,
}

impl<'a, T> BlockTransactionHashes<'a, T> {
    #[inline]
    fn new(txs: &'a BlockTransactions<T>) -> Self {
        Self(match txs {
            BlockTransactions::Hashes(txs) => BlockTransactionHashesInner::Hashes(txs.iter()),
            BlockTransactions::Full(txs) => BlockTransactionHashesInner::Full(txs.iter()),
            BlockTransactions::Uncle => BlockTransactionHashesInner::Uncle,
        })
    }
}

impl<T: TransactionResponse> Iterator for BlockTransactionHashes<'_, T> {
    type Item = B256;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            BlockTransactionHashesInner::Hashes(txs) => txs.next().copied(),
            BlockTransactionHashesInner::Full(txs) => txs.next().map(|tx| tx.tx_hash()),
            BlockTransactionHashesInner::Uncle => None,
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            BlockTransactionHashesInner::Full(txs) => txs.size_hint(),
            BlockTransactionHashesInner::Hashes(txs) => txs.size_hint(),
            BlockTransactionHashesInner::Uncle => (0, Some(0)),
        }
    }
}

impl<T: TransactionResponse> ExactSizeIterator for BlockTransactionHashes<'_, T> {
    #[inline]
    fn len(&self) -> usize {
        match &self.0 {
            BlockTransactionHashesInner::Full(txs) => txs.len(),
            BlockTransactionHashesInner::Hashes(txs) => txs.len(),
            BlockTransactionHashesInner::Uncle => 0,
        }
    }
}

impl<T: TransactionResponse> DoubleEndedIterator for BlockTransactionHashes<'_, T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            BlockTransactionHashesInner::Full(txs) => txs.next_back().map(|tx| tx.tx_hash()),
            BlockTransactionHashesInner::Hashes(txs) => txs.next_back().copied(),
            BlockTransactionHashesInner::Uncle => None,
        }
    }
}

#[cfg(feature = "std")]
impl<T: TransactionResponse> std::iter::FusedIterator for BlockTransactionHashes<'_, T> {}

/// Determines how the `transactions` field of block should be filled.
///
/// This essentially represents the `full:bool` argument in RPC calls that determine whether the
/// response should include full transaction objects or just the hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlockTransactionsKind {
    /// Only include hashes: [BlockTransactions::Hashes]
    #[default]
    Hashes,
    /// Include full transaction objects: [BlockTransactions::Full]
    Full,
}

impl From<bool> for BlockTransactionsKind {
    fn from(is_full: bool) -> Self {
        if is_full {
            Self::Full
        } else {
            Self::Hashes
        }
    }
}

impl From<BlockTransactionsKind> for bool {
    fn from(kind: BlockTransactionsKind) -> Self {
        match kind {
            BlockTransactionsKind::Full => true,
            BlockTransactionsKind::Hashes => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_conversion() {
        let full = true;
        assert_eq!(BlockTransactionsKind::Full, full.into());

        let full = false;
        assert_eq!(BlockTransactionsKind::Hashes, full.into());
    }

    #[test]
    fn test_block_transactions_default() {
        let default: BlockTransactions<()> = BlockTransactions::default();
        assert!(default.is_hashes());
        assert_eq!(default.len(), 0);
    }

    #[test]
    fn test_block_transactions_is_methods() {
        let hashes: BlockTransactions<()> = BlockTransactions::Hashes(vec![B256::ZERO]);
        let full: BlockTransactions<u32> = BlockTransactions::Full(vec![42]);
        let uncle: BlockTransactions<()> = BlockTransactions::Uncle;

        assert!(hashes.is_hashes());
        assert!(!hashes.is_full());
        assert!(!hashes.is_uncle());

        assert!(full.is_full());
        assert!(!full.is_hashes());
        assert!(!full.is_uncle());

        assert!(uncle.is_uncle());
        assert!(!uncle.is_full());
        assert!(!uncle.is_hashes());
    }

    #[test]
    fn test_as_hashes() {
        let hashes = vec![B256::ZERO, B256::repeat_byte(1)];
        let tx_hashes: BlockTransactions<()> = BlockTransactions::Hashes(hashes.clone());

        assert_eq!(tx_hashes.as_hashes(), Some(hashes.as_slice()));
    }

    #[test]
    fn test_as_transactions() {
        let transactions = vec![42, 43];
        let txs = BlockTransactions::Full(transactions.clone());

        assert_eq!(txs.as_transactions(), Some(transactions.as_slice()));
    }

    #[test]
    fn test_block_transactions_len_and_is_empty() {
        let hashes: BlockTransactions<()> = BlockTransactions::Hashes(vec![B256::ZERO]);
        let full = BlockTransactions::Full(vec![42]);
        let uncle: BlockTransactions<()> = BlockTransactions::Uncle;

        assert_eq!(hashes.len(), 1);
        assert_eq!(full.len(), 1);
        assert_eq!(uncle.len(), 0);

        assert!(!hashes.is_empty());
        assert!(!full.is_empty());
        assert!(uncle.is_empty());
    }

    #[test]
    fn test_block_transactions_txns_iterator() {
        let transactions = vec![42, 43];
        let txs = BlockTransactions::Full(transactions);
        let mut iter = txs.txns();

        assert_eq!(iter.next(), Some(&42));
        assert_eq!(iter.next(), Some(&43));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_block_transactions_into_transactions() {
        let transactions = vec![42, 43];
        let txs = BlockTransactions::Full(transactions.clone());
        let collected: Vec<_> = txs.into_transactions().collect();

        assert_eq!(collected, transactions);
    }

    #[test]
    fn test_block_transactions_kind_conversion() {
        let full: BlockTransactionsKind = true.into();
        assert_eq!(full, BlockTransactionsKind::Full);

        let hashes: BlockTransactionsKind = false.into();
        assert_eq!(hashes, BlockTransactionsKind::Hashes);

        let bool_full: bool = BlockTransactionsKind::Full.into();
        assert!(bool_full);

        let bool_hashes: bool = BlockTransactionsKind::Hashes.into();
        assert!(!bool_hashes);
    }
}
