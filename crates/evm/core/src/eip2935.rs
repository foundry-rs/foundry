//! EIP-2935 history storage helpers.

use alloy_primitives::{Address, B256, U256};

pub use alloy_eips::eip2935::{
    HISTORY_SERVE_WINDOW, HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE,
};

/// Returns whether `address` is the EIP-2935 history storage contract.
#[inline]
pub fn is_history_storage_address(address: &Address) -> bool {
    *address == HISTORY_STORAGE_ADDRESS
}

/// Returns the history storage ring slot for `block_number`.
#[inline]
pub fn history_storage_slot(block_number: U256) -> U256 {
    block_number % U256::from(HISTORY_SERVE_WINDOW)
}

/// Encodes a block hash as the history contract storage value.
#[inline]
pub const fn history_storage_value(block_hash: B256) -> U256 {
    U256::from_be_bytes(block_hash.0)
}

/// Returns the first block in the valid EIP-2935 window for `current_block`.
#[inline]
pub fn history_window_start(current_block: U256) -> U256 {
    current_block.saturating_sub(U256::from(HISTORY_SERVE_WINDOW))
}

/// Returns the first block to backfill when rolling forward from `old_block` to `new_block`.
#[inline]
pub fn forward_fill_start(old_block: U256, new_block: U256) -> U256 {
    old_block.max(history_window_start(new_block))
}
