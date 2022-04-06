use ethers_core::types::{BlockNumber, H256};
use serde::{Deserialize, Serialize};

/// Filter
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Eq, Hash)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// From Block
    pub from_block: Option<BlockNumber>,
    /// To Block
    pub to_block: Option<BlockNumber>,
    /// Block hash
    pub block_hash: Option<H256>,
    // /// Address
    // pub address: Option<FilterAddress>,
    // /// Topics
    // pub topics: Option<Topic>,
}
