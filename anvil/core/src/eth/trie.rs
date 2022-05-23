//! Utility functions for Ethereum adapted from https://github.dev/rust-blockchain/ethereum/blob/755dffaa4903fbec1269f50cde9863cf86269a14/src/util.rs
use ethers_core::{types::H256, utils::keccak256};
use hash256_std_hasher::Hash256StdHasher;
use hash_db::Hasher;

/// Concrete `Hasher` impl for the Keccak-256 hash
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Keccak256Hasher;
impl Hasher for Keccak256Hasher {
    type Out = H256;
    type StdHasher = Hash256StdHasher;
    const LENGTH: usize = 32;

    fn hash(x: &[u8]) -> Self::Out {
        H256::from_slice(keccak256(x).as_slice())
    }
}

/// Generates a trie root hash for a vector of key-value tuples
pub fn trie_root<I, K, V>(input: I) -> H256
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<[u8]> + Ord,
    V: AsRef<[u8]>,
{
    triehash::trie_root::<Keccak256Hasher, _, _, _>(input)
}

/// Generates a key-hashed (secure) trie root hash for a vector of key-value tuples.
pub fn sec_trie_root<I, K, V>(input: I) -> H256
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
{
    triehash::sec_trie_root::<Keccak256Hasher, _, _, _>(input)
}

/// Generates a trie root hash for a vector of values
pub fn ordered_trie_root<I, V>(input: I) -> H256
where
    I: IntoIterator<Item = V>,
    V: AsRef<[u8]>,
{
    triehash::ordered_trie_root::<Keccak256Hasher, I>(input)
}
