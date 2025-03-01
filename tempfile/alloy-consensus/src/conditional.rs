//! Helpers for conditional transactions.

/// Contains attributes of a block that are relevant for block conditional transactions.
///
/// These attributes are used to determine preconditions for inclusion in the block with the given
/// attributes (EIP-4337 transactions)
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct BlockConditionalAttributes {
    /// The number of the block.
    pub number: u64,
    /// The block's timestamp
    pub timestamp: u64,
}

impl BlockConditionalAttributes {
    /// Creates a new `BlockConditionalAttributes` with the given block number and timestamp.
    pub const fn new(number: u64, timestamp: u64) -> Self {
        Self { number, timestamp }
    }
}
