//! Support for generating the state root for memdb storage
use std::collections::BTreeMap;

use crate::eth::backend::db::AsHashDB;
use anvil_core::eth::trie::RefSecTrieDBMut;
use bytes::Bytes;
use ethers::{
    types::{Address, H256, U256},
    utils::{rlp, rlp::RlpStream},
};
use forge::revm::db::DbAccount;
use foundry_evm::revm::{AccountInfo, Log};
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
pub fn storage_trie_db(storage: &BTreeMap<U256, U256>) -> (AsHashDB, H256) {
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
pub fn trie_hash_db(accounts: &BTreeMap<Address, DbAccount>) -> (AsHashDB, H256) {
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
pub fn trie_accounts(accounts: &BTreeMap<Address, DbAccount>) -> Vec<(Address, Bytes)> {
    accounts
        .iter()
        .map(|(address, account)| {
            let storage_root = trie_account_rlp(&account.info, &account.storage);
            (*address, storage_root)
        })
        .collect()
}

pub fn state_merkle_trie_root(accounts: &BTreeMap<Address, DbAccount>) -> H256 {
    trie_hash_db(accounts).1
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &BTreeMap<U256, U256>) -> Bytes {
    let mut stream = RlpStream::new_list(4);
    stream.append(&info.nonce);
    stream.append(&info.balance);
    stream.append(&storage_trie_db(storage).1);
    stream.append(&info.code_hash.as_bytes());
    stream.out().freeze()
}
