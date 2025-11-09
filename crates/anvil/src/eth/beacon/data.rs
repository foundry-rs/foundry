//! Beacon data structures for the Beacon API responses

use alloy_primitives::{B256, aliases::B32};
use serde::{Deserialize, Serialize};

/// Ethereum Beacon chain genesis details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisDetails {
    /// The genesis_time configured for the beacon chain
    pub genesis_time: u64,
    /// The genesis validators root
    pub genesis_validators_root: B256,
    /// The genesis fork version, as used in the beacon chain
    pub genesis_fork_version: B32,
}
