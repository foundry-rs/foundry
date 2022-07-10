//! The in memory DB

use crate::{
    eth::backend::db::{Db, StateDb, SerializableAccountRecord, SerializableState},
    mem::state::state_merkle_trie_root,
    revm::AccountInfo,
    Address, U256,
};
use ethers::prelude::H256;
use tracing::{trace, warn};
// reexport for convenience
pub use foundry_evm::executor::{DatabaseRef, backend::MemDb};

use forge::revm::KECCAK_EMPTY;

impl Db for MemDb {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.inner.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.inner.insert_account_storage(address, slot, val)
    }

    fn dump_state(&self) -> Option<SerializableState> {
        Some(SerializableState {
            accounts: self.inner.accounts.clone().into_iter().map(|(k,v)| {
                (
                    k,
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: v.info.balance,
                        code: v.info.code.unwrap_or_else(|| self.inner.code_by_hash(v.info.code_hash)).into(),
                        storage: v.storage.into_iter().collect()
                    }
                )
            }).collect()
        })
    }

    fn load_state(&mut self, state: SerializableState) -> bool {
        for (addr, account) in state.accounts.into_iter() {
            self.insert_account(addr.clone(), AccountInfo {
                balance: account.balance,
                code_hash: KECCAK_EMPTY, // will be set automatically
                code: if account.code.0.is_empty() { None } else { Some(account.code.0) },
                nonce: account.nonce,
            });

            for (k,v) in account.storage.into_iter() {
                self.set_storage_at(addr, k, v);
            }
        }

        true
    }

    /// Creates a new snapshot
    fn snapshot(&mut self) -> U256 {
        let id = self.snapshots.insert(self.inner.clone());
        trace!(target: "backend::memdb", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        if let Some(snapshot) = self.snapshots.remove(id) {
            self.inner = snapshot;
            trace!(target: "backend::memdb", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend::memdb", "No snapshot to revert for {}", id);
            false
        }
    }

    fn maybe_state_root(&self) -> Option<H256> {
        Some(state_merkle_trie_root(&self.inner.accounts))
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.inner.clone())
    }
}
