//! Return types for `eth_getProof`

use crate::eth::trie::KECCAK_NULL_RLP;
use ethers_core::{
    types::{Bytes, H256, U256},
    utils::rlp,
};
use fastrlp::{RlpDecodable, RlpEncodable};
use foundry_evm::revm::KECCAK_EMPTY;
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
    /// Array of rlp-serialized MerkleTree-Nodes, starting with the stateRoot-Node, following the
    /// path of the SHA3 (address) as key.
    pub account_proof: Vec<Bytes>,
    /// the balance of the account
    pub balance: U256,
    /// hash of the code of the account.
    pub code_hash: H256,
    /// nonce of the account
    pub nonce: U256,
    /// SHA3 of the StorageRoot. All storage will deliver a MerkleProof starting with this
    /// rootHash.
    pub storage_hash: H256,
    /// Array of storage-entries as requested. Each entry is a object with these properties:
    pub storage_proof: Vec<StorageProof>,
}

/// Basic account type.
#[derive(Debug, Clone, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct BasicAccount {
    /// Nonce of the account.
    pub nonce: U256,
    /// Balance of the account.
    pub balance: U256,
    /// Storage root of the account.
    pub storage_root: H256,
    /// Code hash of the account.
    pub code_hash: H256,
}

impl Default for BasicAccount {
    fn default() -> Self {
        BasicAccount {
            balance: 0.into(),
            nonce: 0.into(),
            code_hash: KECCAK_EMPTY,
            storage_root: KECCAK_NULL_RLP,
        }
    }
}

impl rlp::Encodable for BasicAccount {
    fn rlp_append(&self, stream: &mut rlp::RlpStream) {
        stream.begin_list(4);
        stream.append(&self.nonce);
        stream.append(&self.balance);
        stream.append(&self.storage_root);
        stream.append(&self.code_hash);
    }
}

impl rlp::Decodable for BasicAccount {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        let result = BasicAccount {
            nonce: rlp.val_at(0)?,
            balance: rlp.val_at(1)?,
            storage_root: rlp.val_at(2)?,
            code_hash: rlp.val_at(3)?,
        };
        Ok(result)
    }
}
