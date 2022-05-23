use ethers_core::types::{transaction::eip2930::AccessList, Address, Bytes, U256};
use serde::{Deserialize, Serialize};

/// Call request
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    /// From
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Address>,
    /// To
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    /// Gas Price
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<U256>,
    /// EIP-1559 Max base fee the caller is willing to pay
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<U256>,
    /// EIP-1559 Priority fee the caller is paying to the block author
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<U256>,
    /// Gas
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas: Option<U256>,
    /// Value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<U256>,
    /// Data
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Bytes>,
    /// Nonce
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<U256>,
    /// AccessList
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_list: Option<AccessList>,
    /// EIP-2718 type
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub transaction_type: Option<U256>,
}
