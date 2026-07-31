//! Notifications emitted from the backed

use alloy_consensus::Header;
use alloy_primitives::B256;
use alloy_rpc_types::Log;
use futures::channel::mpsc::UnboundedReceiver;
use std::sync::Arc;

/// A notification that's emitted when the canonical chain was updated.
#[derive(Clone, Debug)]
pub enum ChainNotification {
    /// A new block was added to the canonical chain.
    Block(NewBlockNotification),
    /// Logs of blocks that were removed from the canonical chain due to a reorg.
    ///
    /// The logs are marked as `removed` and retain the metadata of the removed blocks they were
    /// originally included in.
    RemovedLogs(Arc<Vec<Log>>),
}

impl ChainNotification {
    /// Returns the [`NewBlockNotification`] if this notification is for an imported block.
    pub const fn as_new_block(&self) -> Option<&NewBlockNotification> {
        match self {
            Self::Block(block) => Some(block),
            Self::RemovedLogs(_) => None,
        }
    }
}

/// A notification that's emitted when a new block was imported
#[derive(Clone, Debug)]
pub struct NewBlockNotification {
    /// Hash of the imported block
    pub hash: B256,
    /// block header
    pub header: Arc<Header>,
}

/// Type alias for a receiver that receives [ChainNotification]
pub type ChainNotifications = UnboundedReceiver<ChainNotification>;
