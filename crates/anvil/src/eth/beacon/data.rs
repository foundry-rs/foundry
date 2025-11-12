//! Beacon data structures for the Beacon API responses

use alloy_primitives::{B256, aliases::B32};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

/// Ethereum Beacon chain genesis details
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisDetails {
    /// The genesis_time configured for the beacon chain
    #[serde_as(as = "DisplayFromStr")]
    pub genesis_time: u64,
    /// The genesis validators root
    pub genesis_validators_root: B256,
    /// The genesis fork version, as used in the beacon chain
    pub genesis_fork_version: B32,
}
