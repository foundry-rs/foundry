//! Return types for `eth_getProof`

use crate::eth::trie::KECCAK_NULL_RLP;
use alloy_primitives::{B256, U256};
use revm::primitives::KECCAK_EMPTY;

#[derive(Clone, Debug, PartialEq, Eq, alloy_rlp::RlpEncodable, alloy_rlp::RlpDecodable)]
pub struct BasicAccount {
    pub nonce: U256,
    pub balance: U256,
    pub storage_root: B256,
    pub code_hash: B256,
}

impl Default for BasicAccount {
    fn default() -> Self {
        Self {
            balance: U256::ZERO,
            nonce: U256::ZERO,
            code_hash: KECCAK_EMPTY,
            storage_root: KECCAK_NULL_RLP,
        }
    }
}
