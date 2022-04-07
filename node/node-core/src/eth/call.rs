use ethers_core::types::{transaction::eip2930::AccessList, Address, Bytes, U256};
use serde::Deserialize;

/// Call request
#[derive(Debug, Default, PartialEq, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    /// From
    #[serde(default)]
    pub from: Option<Address>,
    /// To
    pub to: Option<Address>,
    /// Gas Price
    #[serde(default)]
    pub gas_price: Option<U256>,
    /// EIP-1559 Max base fee the caller is willing to pay
    #[serde(default)]
    pub max_fee_per_gas: Option<U256>,
    /// EIP-1559 Priority fee the caller is paying to the block author
    #[serde(default)]
    pub max_priority_fee_per_gas: Option<U256>,
    /// Gas
    #[serde(default)]
    pub gas: Option<U256>,
    /// Value
    #[serde(default)]
    pub value: Option<U256>,
    /// Data
    #[serde(default)]
    pub data: Option<Bytes>,
    /// Nonce
    #[serde(default)]
    pub nonce: Option<U256>,
    /// AccessList
    #[serde(default)]
    pub access_list: Option<AccessList>,
    /// EIP-2718 type
    #[serde(default, rename = "type")]
    pub transaction_type: Option<U256>,
}
