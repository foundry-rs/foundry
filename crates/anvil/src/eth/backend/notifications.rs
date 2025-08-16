//! Notifications emitted from the backed

use alloy_consensus::Header;
use alloy_primitives::B256;
use anvil_core::eth::{block::Block, transaction::TypedReceipt};
use futures::channel::mpsc::UnboundedReceiver;
use std::sync::Arc;

/// A notification that's emitted when a new block was imported
#[derive(Clone, Debug)]
pub struct NewBlockNotification {
    /// Hash of the imported block
    pub hash: B256,
    /// block header
    pub header: Arc<Header>,
}

/// A notification that's emitted when blocks are removed due to reorg
#[derive(Clone, Debug)]
pub struct ReorgedBlockNotification {
    /// Hash of the removed block
    pub hash: B256,
    /// block header
    pub header: Arc<Header>,
    /// The removed block data
    pub block: Block,
    /// The receipts from the removed block
    pub receipts: Vec<TypedReceipt>,
}

/// Type alias for a receiver that receives [NewBlockNotification]
pub type NewBlockNotifications = UnboundedReceiver<NewBlockNotification>;

/// Type alias for a receiver that receives [ReorgedBlockNotification]
pub type ReorgedBlockNotifications = UnboundedReceiver<ReorgedBlockNotification>;
