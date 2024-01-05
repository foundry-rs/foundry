use ethers_core::types::{Address, Bytes, H256, U256};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct AccountOverride {
    pub nonce: Option<u64>,
    pub code: Option<Bytes>,
    pub balance: Option<U256>,
    pub state: Option<HashMap<H256, H256>>,
    pub state_diff: Option<HashMap<H256, H256>>,
}

pub type StateOverride = HashMap<Address, AccountOverride>;
