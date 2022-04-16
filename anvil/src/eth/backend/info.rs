//! Handler that can get current storage related data

use crate::mem::Backend;
use anvil_core::eth::{block::Block, receipt::TypedReceipt};
use ethers::types::H256;
use std::sync::Arc;

/// A type that can fetch data related to the ethereum storage.
///
/// This is simply a wrapper type for the [`Backend`] but exposes a limited set of functions to
/// fetch ethereum storage related data
// TODO(mattsee): once we have multiple Backend types, this should be turned into a trait
#[derive(Clone)]
pub struct StorageInfo {
    backend: Arc<Backend>,
}

// === impl StorageInfo ===

impl StorageInfo {
    pub(crate) fn new(backend: Arc<Backend>) -> Self {
        Self { backend }
    }

    /// Returns the receipts of the current block
    pub fn current_receipts(&self) -> Option<Vec<TypedReceipt>> {
        self.backend.mined_receipts(self.backend.best_hash())
    }

    /// Returns the current block
    pub fn current_block(&self) -> Option<Block> {
        self.backend.get_block(self.backend.best_number())
    }

    /// Returns the receipts of the block with the given hash
    pub fn receipts(&self, hash: H256) -> Option<Vec<TypedReceipt>> {
        self.backend.mined_receipts(hash)
    }

    /// Returns the block with the given hash
    pub fn block(&self, hash: H256) -> Option<Block> {
        self.backend.get_block_by_hash(hash)
    }
}
