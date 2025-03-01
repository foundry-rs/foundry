use crate::{EMPTY_ROOT_HASH, KECCAK_EMPTY};
use alloy_primitives::{keccak256, B256, U256};
use alloy_rlp::{RlpDecodable, RlpEncodable};

/// Represents an TrieAccount in the account trie.
#[derive(Copy, Clone, Debug, PartialEq, Eq, RlpDecodable, RlpEncodable)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct TrieAccount {
    /// The account's nonce.
    #[cfg_attr(feature = "serde", serde(with = "quantity"))]
    pub nonce: u64,
    /// The account's balance.
    pub balance: U256,
    /// The hash of the storage account data.
    pub storage_root: B256,
    /// The hash of the code of the account.
    pub code_hash: B256,
}

impl Default for TrieAccount {
    fn default() -> Self {
        Self {
            nonce: 0,
            balance: U256::ZERO,
            storage_root: EMPTY_ROOT_HASH,
            code_hash: KECCAK_EMPTY,
        }
    }
}

impl TrieAccount {
    /// Compute  hash as committed to in the MPT trie without memorizing.
    pub fn trie_hash_slow(&self) -> B256 {
        keccak256(alloy_rlp::encode(self))
    }
}

#[cfg(feature = "serde")]
mod quantity {
    use alloy_primitives::U64;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serializes a primitive number as a "quantity" hex string.
    pub(crate) fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        U64::from(*value).serialize(serializer)
    }

    /// Deserializes a primitive number from a "quantity" hex string.
    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        U64::deserialize(deserializer).map(|value| value.to())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{hex, U256};
    use alloy_rlp::Decodable;

    #[test]
    fn test_account_encoding() {
        let account = TrieAccount {
            nonce: 1,
            balance: U256::from(1000),
            storage_root: B256::from_slice(&hex!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )),
            code_hash: keccak256(hex!("5a465a905090036002900360015500")),
        };

        let encoded = alloy_rlp::encode(account);

        let decoded = TrieAccount::decode(&mut &encoded[..]).unwrap();
        assert_eq!(account, decoded);
    }

    #[test]
    fn test_trie_hash_slow() {
        let account = TrieAccount {
            nonce: 1,
            balance: U256::from(1000),
            storage_root: B256::from_slice(&hex!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )),
            code_hash: keccak256(hex!("5a465a905090036002900360015500")),
        };

        let expected_hash = keccak256(alloy_rlp::encode(account));
        let actual_hash = account.trie_hash_slow();
        assert_eq!(expected_hash, actual_hash);
    }
}
