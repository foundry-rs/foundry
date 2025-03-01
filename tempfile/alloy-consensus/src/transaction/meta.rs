//! Commonly used types that contain metadata about a transaction.

use alloy_primitives::{BlockHash, TxHash, B256};

/// Additional fields in the context of a block that contains this _mined_ transaction.
///
/// This contains mandatory block fields (block hash, number, timestamp, index).
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct TransactionMeta {
    /// Hash of the transaction.
    pub tx_hash: B256,
    /// Index of the transaction in the block
    pub index: u64,
    /// Hash of the block.
    pub block_hash: B256,
    /// Number of the block.
    pub block_number: u64,
    /// Base fee of the block.
    pub base_fee: Option<u64>,
    /// The excess blob gas of the block.
    pub excess_blob_gas: Option<u64>,
    /// The block's timestamp.
    pub timestamp: u64,
}

/// Additional fields in the context of a (maybe) pending block that contains this transaction.
///
/// This is commonly used when dealing with transactions for rpc where the block context is not
/// known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(alias = "TxInfo")]
pub struct TransactionInfo {
    /// Hash of the transaction.
    pub hash: Option<TxHash>,
    /// Index of the transaction in the block
    pub index: Option<u64>,
    /// Hash of the block.
    pub block_hash: Option<BlockHash>,
    /// Number of the block.
    pub block_number: Option<u64>,
    /// Base fee of the block.
    pub base_fee: Option<u64>,
}

impl TransactionInfo {
    /// Returns a new [`TransactionInfo`] with the provided base fee.
    pub const fn with_base_fee(mut self, base_fee: u64) -> Self {
        self.base_fee = Some(base_fee);
        self
    }
}
