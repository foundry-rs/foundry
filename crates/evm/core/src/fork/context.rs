use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

/// An execution context
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Context {
    /// The block number of the context.
    pub block_number: U256,
}
