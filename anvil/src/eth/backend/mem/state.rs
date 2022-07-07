//! Support for generating the state root for memdb storage
use std::collections::BTreeMap;

use anvil_core::eth::trie::{sec_trie_root, trie_root};
use bytes::Bytes;
use ethers::{
    types::{Address, H256, U256},
    utils::{rlp, rlp::RlpStream},
};
use forge::revm::db::DbAccount;
use foundry_evm::revm::{AccountInfo, Log};

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

pub fn state_merkle_trie_root(accounts: &BTreeMap<Address, DbAccount>) -> H256 {
    let vec = accounts
        .iter()
        .map(|(address, account)| {
            let storage_root = trie_account_rlp(&account.info, &account.storage);
            (*address, storage_root)
        })
        .collect::<Vec<_>>();

    trie_root(vec)
}

/// Returns the RLP for this account.
pub fn trie_account_rlp(info: &AccountInfo, storage: &BTreeMap<U256, U256>) -> Bytes {
    let mut stream = RlpStream::new_list(4);
    stream.append(&info.nonce);
    stream.append(&info.balance);
    stream.append(&{
        sec_trie_root(storage.iter().filter(|(_k, v)| *v != &U256::zero()).map(|(k, v)| {
            let mut temp: [u8; 32] = [0; 32];
            k.to_big_endian(&mut temp);
            (H256::from(temp), rlp::encode(v))
        }))
    });
    stream.append(&info.code_hash.as_bytes());
    stream.out().freeze()
}
