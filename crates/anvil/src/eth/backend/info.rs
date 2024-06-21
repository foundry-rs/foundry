//! Handler that can get current storage related data

use crate::mem::Backend;
use alloy_primitives::B256;
use alloy_rpc_types::Block as AlloyBlock;
use anvil_core::eth::{block::Block, transaction::TypedReceipt};
use std::{fmt, sync::Arc};

/// A type that can fetch data related to the ethereum storage.
///
/// This is simply a wrapper type for the [`Backend`] but exposes a limited set of functions to
/// fetch ethereum storage related data
// TODO(mattsee): once we have multiple Backend types, this should be turned into a trait
#[derive(Clone)]
pub struct StorageInfo {
    backend: Arc<Backend>,
}

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
    pub fn receipts(&self, hash: B256) -> Option<Vec<TypedReceipt>> {
        self.backend.mined_receipts(hash)
    }

    /// Returns the block with the given hash
    pub fn block(&self, hash: B256) -> Option<Block> {
        self.backend.get_block_by_hash(hash)
    }

    /// Returns the block with the given hash in the format of the ethereum API
    pub fn eth_block(&self, hash: B256) -> Option<AlloyBlock> {
        let block = self.block(hash)?;
        Some(self.backend.convert_block(block))
    }
}

impl fmt::Debug for StorageInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageInfo").finish_non_exhaustive()
    }
}
