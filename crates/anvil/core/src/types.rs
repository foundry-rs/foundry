use alloy_eips::eip7840::BlobParams;
use alloy_primitives::{Address, Bytes, U256};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use foundry_common::serde_helpers::deserialize_number;
use serde::{Deserialize, Serialize};

/// Represents the options used in `anvil_reorg`
#[derive(Debug, Clone, Deserialize)]
pub struct ReorgOptions {
    // The depth of the reorg
    pub depth: u64,
    // List of transaction requests and blocks pairs to be mined into the new chain
    pub tx_block_pairs: Vec<(TransactionData, u64)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[expect(clippy::large_enum_variant)]
pub enum TransactionData {
    JSON(WithOtherFields<TransactionRequest>),
    Raw(Bytes),
}

/// Blob parameters a node prices blobs with, reported as `blobParams` in `anvil_nodeInfo`.
///
/// Unlike the EIP-7840 serialization of [`BlobParams`], this keeps every field, so a fork of the
/// node can tell schedules apart that share their target, max, and update fraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBlobParams {
    pub target_blob_count: u64,
    pub max_blob_count: u64,
    #[serde(with = "alloy_serde::quantity")]
    pub update_fraction: u128,
    #[serde(with = "alloy_serde::quantity")]
    pub min_blob_fee: u128,
    pub max_blobs_per_tx: u64,
    pub blob_base_cost: u64,
}

impl From<BlobParams> for NodeBlobParams {
    fn from(params: BlobParams) -> Self {
        let BlobParams {
            target_blob_count,
            max_blob_count,
            update_fraction,
            min_blob_fee,
            max_blobs_per_tx,
            blob_base_cost,
        } = params;
        Self {
            target_blob_count,
            max_blob_count,
            update_fraction,
            min_blob_fee,
            max_blobs_per_tx,
            blob_base_cost,
        }
    }
}

impl From<NodeBlobParams> for BlobParams {
    fn from(params: NodeBlobParams) -> Self {
        let NodeBlobParams {
            target_blob_count,
            max_blob_count,
            update_fraction,
            min_blob_fee,
            max_blobs_per_tx,
            blob_base_cost,
        } = params;
        Self {
            target_blob_count,
            max_blob_count,
            update_fraction,
            min_blob_fee,
            max_blobs_per_tx,
            blob_base_cost,
        }
    }
}

/// Parameters for creating a native TIP-20 token at a chosen address on Tempo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TempoCreateTokenRequest {
    /// The TIP-20 address to initialize.
    pub address: Address,
    /// The token name.
    pub name: String,
    /// The token symbol.
    pub symbol: String,
    /// The token's currency denomination.
    pub currency: String,
    /// The deployed TIP-20 token used as the quote token.
    pub quote_token: Address,
    /// The account receiving the default admin and issuer roles.
    pub admin: Address,
    /// Initial mints. Omitting this list creates a token with zero supply.
    #[serde(default)]
    pub mint: Vec<TempoTokenMint>,
}

/// An initial mint performed when creating a native TIP-20 token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TempoTokenMint {
    /// The account receiving the tokens.
    pub to: Address,
    /// The amount in the token's smallest units.
    #[serde(deserialize_with = "deserialize_number")]
    pub amount: U256,
}
