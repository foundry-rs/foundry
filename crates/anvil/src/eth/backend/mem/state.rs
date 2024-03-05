//! Support for generating the state root for memdb storage

use crate::eth::{backend::db::AsHashDB, error::BlockchainError};
use alloy_primitives::{Address, Bytes, B256, U256};
use alloy_rlp::Encodable;
use alloy_rpc_types::state::StateOverride;
use anvil_core::eth::trie::RefSecTrieDBMut;
use foundry_evm::{
    backend::DatabaseError,
    hashbrown::HashMap as Map,
    revm::{
        db::{CacheDB, DatabaseRef, DbAccount},
        primitives::{AccountInfo, Bytecode},
    },
};
use memory_db::HashKey;
use trie_db::TrieMut;

/// Returns storage trie of an account as `HashDB`
pub fn storage_trie_db(storage: &Map<U256, U256>) -> (AsHashDB, B256) {
    // Populate DB with full trie from entries.
    let (db, root) = {
        let mut db = <memory_db::MemoryDB<_, HashKey<_>, _>>::default();
        let mut root = Default::default();
        {
            let mut trie = RefSecTrieDBMut::new(&mut db, &mut root);
            for (k, v) in storage.iter().filter(|(_k, v)| *v != &U256::from(0)) {
                let key = B256::from(*k);
                let mut value: Vec<u8> = Vec::new();
                U256::encode(v, &mut value);
                trie.insert(key.as_slice(), value.as_ref()).unwrap();
            }
        }
        (db, root)
    };

    (Box::new(db), B256::from(root))
}

/// Returns the account data as `HashDB`
pub fn trie_hash_db(accounts: &Map<Address, DbAccount>) -> (AsHashDB, B256) {
    let accounts = trie_accounts(accounts);

    // Populate DB with full trie from entries.
    let (db, root) = {
        let mut db = <memory_db::MemoryDB<_, HashKey<_>, _>>::default();
        let mut root = Default::default();
        {
            let mut trie = RefSecTrieDBMut::new(&mut db, &mut root);
            for (address, value) in accounts {
                trie.insert(address.as_ref(), value.as_ref()).unwrap();
            }
        }
        (db, root)
    };

    (Box::new(db), B256::from(root))
}

/// Returns all RLP-encoded Accounts
pub fn trie_accounts(accounts: &Map<Address, DbAccount>) -> Vec<(Address, Bytes)> {
    accounts
        .iter()
        .map(|(address, account)| {
            let storage_root = trie_account_rlp(&account.info, &account.storage);
            (*address, storage_root)
        })
        .collect()
}

pub fn state_merkle_trie_root(accounts: &Map<Address, DbAccount>) -> B256 {
    trie_hash_db(accounts).1
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &Map<U256, U256>) -> Bytes {
    let mut out: Vec<u8> = Vec::new();
    let list: [&dyn Encodable; 4] =
        [&info.nonce, &info.balance, &storage_trie_db(storage).1, &info.code_hash];

    alloy_rlp::encode_list::<_, dyn Encodable>(&list, &mut out);

    out.into()
}

/// Applies the given state overrides to the state, returning a new CacheDB state
pub fn apply_state_override<D>(
    overrides: StateOverride,
    state: D,
) -> Result<CacheDB<D>, BlockchainError>
where
    D: DatabaseRef<Error = DatabaseError>,
{
    let mut cache_db = CacheDB::new(state);
    for (account, account_overrides) in overrides.iter() {
        let mut account_info = cache_db.basic_ref(*account)?.unwrap_or_default();

        if let Some(nonce) = account_overrides.nonce {
            account_info.nonce = nonce.to::<u64>();
        }
        if let Some(code) = &account_overrides.code {
            account_info.code = Some(Bytecode::new_raw(code.to_vec().into()));
        }
        if let Some(balance) = account_overrides.balance {
            account_info.balance = balance;
        }

        cache_db.insert_account_info(*account, account_info);

        // We ensure that not both state and state_diff are set.
        // If state is set, we must mark the account as "NewlyCreated", so that the old storage
        // isn't read from
        match (&account_overrides.state, &account_overrides.state_diff) {
            (Some(_), Some(_)) => {
                return Err(BlockchainError::StateOverrideError(
                    "state and state_diff can't be used together".to_string(),
                ))
            }
            (None, None) => (),
            (Some(new_account_state), None) => {
                cache_db.replace_account_storage(
                    *account,
                    new_account_state
                        .iter()
                        .map(|(key, value)| ((*key).into(), (*value)))
                        .collect(),
                )?;
            }
            (None, Some(account_state_diff)) => {
                for (key, value) in account_state_diff.iter() {
                    cache_db.insert_account_storage(*account, (*key).into(), *value)?;
                }
            }
        };
    }
    Ok(cache_db)
}
