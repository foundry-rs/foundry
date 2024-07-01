//! Utility functions for Ethereum adapted from <https://github.com/rust-blockchain/ethereum/blob/755dffaa4903fbec1269f50cde9863cf86269a14/src/util.rs>

use alloy_primitives::{fixed_bytes, B256};
use alloy_trie::{HashBuilder, Nibbles};
use std::collections::BTreeMap;

/// The KECCAK of the RLP encoding of empty data.
pub const KECCAK_NULL_RLP: B256 =
    fixed_bytes!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421");

/// Generates a trie root hash for a vector of values
pub fn ordered_trie_root<I, V>(input: I) -> B256
where
    I: IntoIterator<Item = V>,
    V: AsRef<[u8]>,
{
    let mut builder = HashBuilder::default();

    let input = input
        .into_iter()
        .enumerate()
        .map(|(i, v)| (alloy_rlp::encode(i), v))
        .collect::<BTreeMap<_, _>>();

    for (key, value) in input {
        builder.add_leaf(Nibbles::unpack(key), value.as_ref());
    }

    builder.root()
}
