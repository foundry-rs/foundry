//! Support for generating the state root for memdb storage

use crate::eth::{backend::db::AsHashDB, error::BlockchainError};
use anvil_core::eth::{state::StateOverride, trie::RefSecTrieDBMut};
use bytes::Bytes;
use ethers::{
    abi::ethereum_types::BigEndianHash,
    types::{Address, H256, U256},
    utils::{rlp, rlp::RlpStream},
};
use forge::{
    executor::DatabaseRef,
    revm::{
        db::{CacheDB, DbAccount},
        Bytecode,
    },
};
use foundry_evm::{
    executor::backend::DatabaseError,
    revm::{AccountInfo, Log},
    HashMap as Map,
};
use memory_db::HashKey;
use trie_db::TrieMut;

/// Returns the log hash for all `logs`
///
/// The log hash is `keccak(rlp(logs[]))`, <https://github.com/ethereum/go-ethereum/blob/356bbe343a30789e77bb38f25983c8f2f2bfbb47/cmd/evm/internal/t8ntool/execution.go#L255>
pub fn log_rlp_hash(logs: Vec<Log>) -> H256 {
    let mut stream = RlpStream::new();
    stream.begin_unbounded_list();
    for log in logs {
        stream.begin_list(3);
        stream.append(&log.address);
        stream.append_list(&log.topics);
        stream.append(&log.data);
    }
    stream.finalize_unbounded_list();
    let out = stream.out().freeze();

    let out = ethers::utils::keccak256(out);
    H256::from_slice(out.as_slice())
}

/// Returns storage trie of an account as `HashDB`
pub fn storage_trie_db(storage: &Map<U256, U256>) -> (AsHashDB, H256) {
    // Populate DB with full trie from entries.
    let (db, root) = {
        let mut db = <memory_db::MemoryDB<_, HashKey<_>, _>>::default();
        let mut root = Default::default();
        {
            let mut trie = RefSecTrieDBMut::new(&mut db, &mut root);
            for (k, v) in storage.iter().filter(|(_k, v)| *v != &U256::zero()) {
                let mut temp: [u8; 32] = [0; 32];
                k.to_big_endian(&mut temp);
                let key = H256::from(temp);
                let value = rlp::encode(v);
                trie.insert(key.as_bytes(), value.as_ref()).unwrap();
            }
        }
        (db, root)
    };

    (Box::new(db), H256::from(root))
}

/// Returns the account data as `HashDB`
pub fn trie_hash_db(accounts: &Map<Address, DbAccount>) -> (AsHashDB, H256) {
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

    (Box::new(db), H256::from(root))
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

pub fn state_merkle_trie_root(accounts: &Map<Address, DbAccount>) -> H256 {
    trie_hash_db(accounts).1
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &Map<U256, U256>) -> Bytes {
    let mut stream = RlpStream::new_list(4);
    stream.append(&info.nonce);
    stream.append(&info.balance);
    stream.append(&storage_trie_db(storage).1);
    stream.append(&info.code_hash.as_bytes());
    stream.out().freeze()
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
        let mut account_info = cache_db.basic(*account)?.unwrap_or_default();

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
                        .map(|(key, value)| (key.into_uint(), value.into_uint()))
                        .collect(),
                )?;
            }
            (None, Some(account_state_diff)) => {
                for (key, value) in account_state_diff.iter() {
                    cache_db.insert_account_storage(
                        *account,
                        key.into_uint(),
                        value.into_uint(),
                    )?;
                }
            }
        };
    }
    Ok(cache_db)
}
