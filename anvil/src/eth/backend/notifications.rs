//! Notifications emitted from the backed

use anvil_core::eth::block::Header;
use ethers::types::H256;
use futures::channel::mpsc::UnboundedReceiver;
use std::sync::Arc;

/// A notification that's emitted when a new block was imported
#[derive(Clone, Debug)]
pub struct NewBlockNotification {
    /// Hash of the imported block
    pub hash: H256,
    /// block header
    pub header: Arc<Header>,
}

/// Type alias for a receiver that receives [NewBlockNotification]
pub type NewBlockNotifications = UnboundedReceiver<NewBlockNotification>;
