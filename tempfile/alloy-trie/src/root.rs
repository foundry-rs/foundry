use crate::{HashBuilder, EMPTY_ROOT_HASH};
use alloc::vec::Vec;
use alloy_primitives::B256;
use alloy_rlp::Encodable;
use nybbles::Nibbles;

/// Adjust the index of an item for rlp encoding.
pub const fn adjust_index_for_rlp(i: usize, len: usize) -> usize {
    if i > 0x7f {
        i
    } else if i == 0x7f || i + 1 == len {
        0
    } else {
        i + 1
    }
}

/// Compute a trie root of the collection of rlp encodable items.
pub fn ordered_trie_root<T: Encodable>(items: &[T]) -> B256 {
    ordered_trie_root_with_encoder(items, |item, buf| item.encode(buf))
}

/// Compute a trie root of the collection of items with a custom encoder.
pub fn ordered_trie_root_with_encoder<T, F>(items: &[T], mut encode: F) -> B256
where
    F: FnMut(&T, &mut Vec<u8>),
{
    if items.is_empty() {
        return EMPTY_ROOT_HASH;
    }

    let mut value_buffer = Vec::new();

    let mut hb = HashBuilder::default();
    let items_len = items.len();
    for i in 0..items_len {
        let index = adjust_index_for_rlp(i, items_len);

        let index_buffer = alloy_rlp::encode_fixed_size(&index);

        value_buffer.clear();
        encode(&items[index], &mut value_buffer);

        hb.add_leaf(Nibbles::unpack(&index_buffer), &value_buffer);
    }

    hb.root()
}

/// Ethereum specific trie root functions.
#[cfg(feature = "ethereum")]
pub use ethereum::*;
#[cfg(feature = "ethereum")]
mod ethereum {
    use super::*;
    use crate::TrieAccount;
    use alloy_primitives::{keccak256, Address, U256};

    /// Hashes storage keys, sorts them and them calculates the root hash of the storage trie.
    /// See [`storage_root_unsorted`] for more info.
    pub fn storage_root_unhashed(storage: impl IntoIterator<Item = (B256, U256)>) -> B256 {
        storage_root_unsorted(storage.into_iter().map(|(slot, value)| (keccak256(slot), value)))
    }

    /// Sorts and calculates the root hash of account storage trie.
    /// See [`storage_root`] for more info.
    pub fn storage_root_unsorted(storage: impl IntoIterator<Item = (B256, U256)>) -> B256 {
        let mut v = Vec::from_iter(storage);
        v.sort_unstable_by_key(|(key, _)| *key);
        storage_root(v)
    }

    /// Calculates the root hash of account storage trie.
    ///
    /// # Panics
    ///
    /// If the items are not in sorted order.
    pub fn storage_root(storage: impl IntoIterator<Item = (B256, U256)>) -> B256 {
        let mut hb = HashBuilder::default();
        for (hashed_slot, value) in storage {
            hb.add_leaf(
                Nibbles::unpack(hashed_slot),
                alloy_rlp::encode_fixed_size(&value).as_ref(),
            );
        }
        hb.root()
    }

    /// Hashes and sorts account keys, then proceeds to calculating the root hash of the state
    /// represented as MPT.
    /// See [`state_root_unsorted`] for more info.
    pub fn state_root_ref_unhashed<'a, A: Into<TrieAccount> + Clone + 'a>(
        state: impl IntoIterator<Item = (&'a Address, &'a A)>,
    ) -> B256 {
        state_root_unsorted(
            state.into_iter().map(|(address, account)| (keccak256(address), account.clone())),
        )
    }

    /// Hashes and sorts account keys, then proceeds to calculating the root hash of the state
    /// represented as MPT.
    /// See [`state_root_unsorted`] for more info.
    pub fn state_root_unhashed<A: Into<TrieAccount>>(
        state: impl IntoIterator<Item = (Address, A)>,
    ) -> B256 {
        state_root_unsorted(
            state.into_iter().map(|(address, account)| (keccak256(address), account)),
        )
    }

    /// Sorts the hashed account keys and calculates the root hash of the state represented as MPT.
    /// See [`state_root`] for more info.
    pub fn state_root_unsorted<A: Into<TrieAccount>>(
        state: impl IntoIterator<Item = (B256, A)>,
    ) -> B256 {
        let mut vec = Vec::from_iter(state);
        vec.sort_unstable_by_key(|(key, _)| *key);
        state_root(vec)
    }

    /// Calculates the root hash of the state represented as MPT.
    ///
    /// Corresponds to [geth's `deriveHash`](https://github.com/ethereum/go-ethereum/blob/6c149fd4ad063f7c24d726a73bc0546badd1bc73/core/genesis.go#L119).
    ///
    /// # Panics
    ///
    /// If the items are not in sorted order.
    pub fn state_root<A: Into<TrieAccount>>(state: impl IntoIterator<Item = (B256, A)>) -> B256 {
        let mut hb = HashBuilder::default();
        let mut account_rlp_buf = Vec::new();
        for (hashed_key, account) in state {
            account_rlp_buf.clear();
            account.into().encode(&mut account_rlp_buf);
            hb.add_leaf(Nibbles::unpack(hashed_key), &account_rlp_buf);
        }
        hb.root()
    }
}
