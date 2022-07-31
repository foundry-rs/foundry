//! Return types for `eth_getProof`

use ethers_core::types::{Bytes, H256, U256};
use serde::{Deserialize, Serialize};

/// Contains the proof for one single storage-entry
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProof {
    pub key: U256,
    pub value: U256,
    pub proof: Vec<Bytes>,
}

/// Account information.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProof {
    /// Array of rlp-serialized MerkleTree-Nodes, starting with the stateRoot-Node, following the path of the SHA3 (address) as key.
    pub account_proof: Vec<Bytes>,
    /// the balance of the account
    pub balance: U256,
    /// hash of the code of the account.
    pub code_hash: H256,
    /// nonce of the account
    pub nonce: U256,
    /// SHA3 of the StorageRoot. All storage will deliver a MerkleProof starting with this rootHash.
    pub storage_hash: H256,
    /// Array of storage-entries as requested. Each entry is a object with these properties:
    pub storage_proof: Vec<StorageProof>,
}
