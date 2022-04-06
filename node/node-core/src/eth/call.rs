use ethers_core::types::{transaction::eip2930::AccessList, Address, Bytes, U256};
use serde::Deserialize;

/// Call request
#[derive(Debug, Default, PartialEq, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    /// From
    pub from: Option<Address>,
    /// To
    pub to: Option<Address>,
    /// Gas Price
    pub gas_price: Option<U256>,
    /// EIP-1559 Max base fee the caller is willing to pay
    pub max_fee_per_gas: Option<U256>,
    /// EIP-1559 Priority fee the caller is paying to the block author
    pub max_priority_fee_per_gas: Option<U256>,
    /// Gas
    pub gas: Option<U256>,
    /// Value
    pub value: Option<U256>,
    /// Data
    pub data: Option<Bytes>,
    /// Nonce
    pub nonce: Option<U256>,
    /// AccessList
    pub access_list: Option<AccessList>,
    /// EIP-2718 type
    #[serde(rename = "type")]
    pub transaction_type: Option<U256>,
}
