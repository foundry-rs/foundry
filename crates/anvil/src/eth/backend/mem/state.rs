//! Support for generating the state root for memdb storage

use crate::eth::error::BlockchainError;
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rlp::Encodable;
use alloy_rpc_types::state::StateOverride;
use alloy_trie::{HashBuilder, Nibbles};
use foundry_evm::{
    backend::DatabaseError,
    revm::{
        db::{CacheDB, DatabaseRef, DbAccount},
        primitives::{AccountInfo, Bytecode, HashMap},
    },
};

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
            account_info.nonce = nonce;
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
                        .map(|(key, value)| ((*key).into(), (*value).into()))
                        .collect(),
                )?;
            }
            (None, Some(account_state_diff)) => {
                for (key, value) in account_state_diff.iter() {
                    cache_db.insert_account_storage(*account, (*key).into(), (*value).into())?;
                }
            }
        };
    }
    Ok(cache_db)
}
