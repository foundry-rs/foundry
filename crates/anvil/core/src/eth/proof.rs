//! Return types for `eth_getProof`

use crate::eth::trie::KECCAK_NULL_RLP;
use ethers_core::{
    types::{H256, U256},
    utils::rlp,
};
use foundry_common::types::ToEthers;
use revm::primitives::KECCAK_EMPTY;

// reexport for convenience
pub use ethers_core::types::{EIP1186ProofResponse as AccountProof, StorageProof};

/// Basic account type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
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
            code_hash: KECCAK_EMPTY.to_ethers(),
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
