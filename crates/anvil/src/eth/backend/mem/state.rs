//! Support for generating the state root for memdb storage

use alloy_primitives::{Address, B256, U256, keccak256, map::HashMap};
use alloy_rlp::Encodable;
use alloy_trie::{HashBuilder, Nibbles};
use revm::{database::DbAccount, state::AccountInfo};

pub fn build_root(values: impl IntoIterator<Item = (Nibbles, Vec<u8>)>) -> B256 {
    let mut builder = HashBuilder::default();
    for (key, value) in values {
        builder.add_leaf(key, value.as_ref());
    }
    builder.root()
}

/// Builds state root from the given accounts
pub fn state_root(accounts: &HashMap<Address, DbAccount>) -> B256 {
    build_root(trie_accounts(accounts))
}

/// Builds storage root from the given storage
pub fn storage_root(storage: &HashMap<U256, U256>) -> B256 {
    build_root(trie_storage(storage))
}

/// Builds iterator over stored key-value pairs ready for storage trie root calculation.
pub fn trie_storage(storage: &HashMap<U256, U256>) -> Vec<(Nibbles, Vec<u8>)> {
    let mut storage = storage
        .iter()
        .map(|(key, value)| {
            let data = alloy_rlp::encode(value);
            (Nibbles::unpack(keccak256(key.to_be_bytes::<32>())), data)
        })
        .collect::<Vec<_>>();
    storage.sort_by(|(key1, _), (key2, _)| key1.cmp(key2));

    storage
}

/// Builds iterator over stored key-value pairs ready for account trie root calculation.
pub fn trie_accounts(accounts: &HashMap<Address, DbAccount>) -> Vec<(Nibbles, Vec<u8>)> {
    let mut accounts = accounts
        .iter()
        .map(|(address, account)| {
            let data = trie_account_rlp(&account.info, &account.storage);
            (Nibbles::unpack(keccak256(*address)), data)
        })
        .collect::<Vec<_>>();
    accounts.sort_by(|(key1, _), (key2, _)| key1.cmp(key2));

    accounts
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &HashMap<U256, U256>) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let list: [&dyn Encodable; 4] =
        [&info.nonce, &info.balance, &storage_root(storage), &info.code_hash];

    alloy_rlp::encode_list::<_, dyn Encodable>(&list, &mut out);

    out
}
