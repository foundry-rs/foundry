#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use alloy_primitives::{BlockHash, Bytes, ChainId, TxHash, B256, U256};
use alloy_rpc_types_eth::TransactionRequest;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

/// Represents the params to set forking which can take various forms:
///  - untagged
///  - tagged forking
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Forking {
    /// The URL of the JSON-RPC endpoint to fork from.
    pub json_rpc_url: Option<String>,
    /// The block number to fork from.
    pub block_number: Option<u64>,
}

impl<'de> serde::Deserialize<'de> for Forking {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ForkOpts {
            json_rpc_url: Option<String>,
            #[serde(default, with = "alloy_serde::quantity::opt")]
            block_number: Option<u64>,
        }

        #[derive(serde::Deserialize)]
        struct Tagged {
            forking: ForkOpts,
        }
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum ForkingVariants {
            Tagged(Tagged),
            Fork(ForkOpts),
        }
        let f = match ForkingVariants::deserialize(deserializer)? {
            ForkingVariants::Fork(ForkOpts { json_rpc_url, block_number }) => {
                Self { json_rpc_url, block_number }
            }
            ForkingVariants::Tagged(f) => {
                Self { json_rpc_url: f.forking.json_rpc_url, block_number: f.forking.block_number }
            }
        };
        Ok(f)
    }
}

/// Anvil equivalent of `node_info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// The current block number
    #[serde(with = "alloy_serde::quantity")]
    pub current_block_number: u64,
    /// The current block timestamp
    pub current_block_timestamp: u64,
    /// The current block hash
    pub current_block_hash: BlockHash,
    /// The enabled hardfork
    pub hard_fork: String,
    /// How transactions are ordered for mining
    #[doc(alias = "tx_order")]
    pub transaction_order: String,
    /// Info about the node's block environment
    pub environment: NodeEnvironment,
    /// Info about the node's fork configuration
    pub fork_config: NodeForkConfig,
}

/// The current block environment of the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEnvironment {
    /// Base fee of the current block
    #[serde(with = "alloy_serde::quantity")]
    pub base_fee: u128,
    /// Chain id of the node.
    pub chain_id: ChainId,
    /// Configured block gas limit
    #[serde(with = "alloy_serde::quantity")]
    pub gas_limit: u64,
    /// Configured gas price
    #[serde(with = "alloy_serde::quantity")]
    pub gas_price: u128,
}

/// The node's fork configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeForkConfig {
    /// URL of the forked network
    pub fork_url: Option<String>,
    /// Block number of the forked network
    pub fork_block_number: Option<u64>,
    /// Retry backoff for requests
    pub fork_retry_backoff: Option<u128>,
}

/// Anvil equivalent of `hardhat_metadata`.
/// Metadata about the current Anvil instance.
/// See <https://hardhat.org/hardhat-network/docs/reference#hardhat_metadata>
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    /// client version
    pub client_version: String,
    /// Chain id of the node.
    pub chain_id: ChainId,
    /// Unique instance id
    pub instance_id: B256,
    /// Latest block number
    pub latest_block_number: u64,
    /// Latest block hash
    pub latest_block_hash: BlockHash,
    /// Forked network info
    pub forked_network: Option<ForkedNetwork>,
    /// Snapshots of the chain
    pub snapshots: BTreeMap<U256, (u64, B256)>,
}

/// Information about the forked network.
/// See <https://hardhat.org/hardhat-network/docs/reference#hardhat_metadata>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedNetwork {
    /// Chain id of the node.
    pub chain_id: ChainId,
    /// Block number of the forked chain
    pub fork_block_number: u64,
    /// Block hash of the forked chain
    pub fork_block_hash: TxHash,
}

/// Additional `evm_mine` options
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MineOptions {
    /// The options for mining
    Options {
        /// The timestamp the block should be mined with
        #[serde(with = "alloy_serde::quantity::opt")]
        timestamp: Option<u64>,
        /// If `blocks` is given, it will mine exactly blocks number of blocks, regardless of any
        /// other blocks mined or reverted during it's operation
        blocks: Option<u64>,
    },
    /// The timestamp the block should be mined with
    #[serde(with = "alloy_serde::quantity::opt")]
    Timestamp(Option<u64>),
}

impl Default for MineOptions {
    fn default() -> Self {
        Self::Options { timestamp: None, blocks: None }
    }
}

/// Represents the options used in `anvil_reorg`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgOptions {
    /// The depth of the reorg
    pub depth: u64,
    /// List of transaction requests and blocks pairs to be mined into the new chain
    pub tx_block_pairs: Vec<(TransactionData, u64)>,
}

/// Type representing txs in `ReorgOptions`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransactionData {
    /// Transaction request
    JSON(TransactionRequest),
    /// Raw transaction bytes
    Raw(Bytes),
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn test_serde_forking_deserialization() {
        // Test full forking object
        let json_data = r#"{"forking": {"jsonRpcUrl": "https://ethereumpublicnode.com","blockNumber": "18441649"}}"#;
        let forking: Forking = serde_json::from_str(json_data).unwrap();
        assert_eq!(
            forking,
            Forking {
                json_rpc_url: Some("https://ethereumpublicnode.com".into()),
                block_number: Some(18441649)
            }
        );

        // Test forking object with only jsonRpcUrl
        let json_data = r#"{"forking": {"jsonRpcUrl": "https://ethereumpublicnode.com"}}"#;
        let forking: Forking = serde_json::from_str(json_data).unwrap();
        assert_eq!(
            forking,
            Forking {
                json_rpc_url: Some("https://ethereumpublicnode.com".into()),
                block_number: None
            }
        );

        // Test forking object with only blockNumber
        let json_data = r#"{"forking": {"blockNumber": "18441649"}}"#;
        let forking: Forking =
            serde_json::from_str(json_data).expect("Failed to deserialize forking object");
        assert_eq!(forking, Forking { json_rpc_url: None, block_number: Some(18441649) });
    }

    #[test]
    fn test_serde_deserialize_options_with_values() {
        let data = r#"{"timestamp": 1620000000, "blocks": 10}"#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(
            deserialized,
            MineOptions::Options { timestamp: Some(1620000000), blocks: Some(10) }
        );

        let data = r#"{"timestamp": "0x608f3d00", "blocks": 10}"#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(
            deserialized,
            MineOptions::Options { timestamp: Some(1620000000), blocks: Some(10) }
        );
    }

    #[test]
    fn test_serde_deserialize_options_with_timestamp() {
        let data = r#"{"timestamp":"1620000000"}"#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(
            deserialized,
            MineOptions::Options { timestamp: Some(1620000000), blocks: None }
        );

        let data = r#"{"timestamp":"0x608f3d00"}"#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(
            deserialized,
            MineOptions::Options { timestamp: Some(1620000000), blocks: None }
        );
    }

    #[test]
    fn test_serde_deserialize_timestamp() {
        let data = r#""1620000000""#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(deserialized, MineOptions::Timestamp(Some(1620000000)));

        let data = r#""0x608f3d00""#;
        let deserialized: MineOptions = serde_json::from_str(data).expect("Deserialization failed");
        assert_eq!(deserialized, MineOptions::Timestamp(Some(1620000000)));
    }
}
