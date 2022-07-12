//! The in memory DB

use crate::{
    eth::backend::db::{Db, SerializableAccountRecord, SerializableState, StateDb},
    mem::state::state_merkle_trie_root,
    revm::AccountInfo,
    Address, U256,
};
use ethers::prelude::H256;
use tracing::{trace, warn};
// reexport for convenience
pub use foundry_evm::executor::{backend::MemDb, DatabaseRef};

use forge::revm::KECCAK_EMPTY;

impl Db for MemDb {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.inner.insert_account_info(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.inner.insert_account_storage(address, slot, val)
    }

    fn dump_state(&self) -> Option<SerializableState> {
        let accounts = self
            .inner
            .accounts
            .clone()
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: v.info.balance,
                        code: v
                            .info
                            .code
                            .unwrap_or_else(|| self.inner.code_by_hash(v.info.code_hash))
                            .into(),
                        storage: v.storage.into_iter().collect(),
                    },
                )
            })
            .collect();

        Some(SerializableState { accounts })
    }

    fn load_state(&mut self, state: SerializableState) -> bool {
        for (addr, account) in state.accounts.into_iter() {
            let old_account = self.inner.accounts.get(&addr);

            self.insert_account(
                addr,
                AccountInfo {
                    balance: account.balance,
                    code_hash: KECCAK_EMPTY, // will be set automatically
                    code: if account.code.0.is_empty() { None } else { Some(account.code.0) },
                    // use max nonce in case account is imported multiple times with difference
                    // nonces to prevent collisions
                    nonce: std::cmp::max(
                        old_account.map(|a| a.info.nonce).unwrap_or_default(),
                        account.nonce,
                    ),
                },
            );

            for (k, v) in account.storage.into_iter() {
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

#[cfg(test)]
mod tests {
    use crate::{
        eth::backend::db::{Db, SerializableAccountRecord, SerializableState},
        revm::AccountInfo,
        Address,
    };
    use bytes::Bytes;
    use forge::revm::KECCAK_EMPTY;
    use foundry_evm::{
        executor::{backend::MemDb, DatabaseRef},
        HashMap,
    };
    use std::str::FromStr;

    // verifies that all substantial aspects of a loaded account remain the state after an account
    // is dumped and reloaded
    #[test]
    fn test_dump_reload_cycle() {
        let test_addr: Address =
            Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();

        let mut dump_db = MemDb::default();

        let contract_code: Bytes = Bytes::from("fake contract code");

        dump_db.insert_account(
            test_addr.clone(),
            AccountInfo {
                balance: 123456.into(),
                code_hash: KECCAK_EMPTY,
                code: Some(contract_code.clone()),
                nonce: 1234,
            },
        );

        dump_db.set_storage_at(test_addr, "0x1234567".into(), "0x1".into());

        let state = dump_db.dump_state().unwrap();

        let mut load_db = MemDb::default();

        load_db.load_state(state);

        let loaded_account = load_db.basic(test_addr);

        assert_eq!(loaded_account.balance, 123456.into());
        assert_eq!(load_db.code_by_hash(loaded_account.code_hash), contract_code);
        assert_eq!(loaded_account.nonce, 1234);
        assert_eq!(load_db.storage(test_addr, "0x1234567".into()), "0x1".into());
    }

    // verifies that multiple accounts can be loaded at a time, and storage is merged within those
    // accounts as well.
    #[test]
    fn test_load_state_merge() {
        let test_addr: Address =
            Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();
        let test_addr2: Address =
            Address::from_str("0x70997970c51812dc3a010c7d01b50e0d17dc79c8").unwrap();

        let contract_code: Bytes = Bytes::from("fake contract code");

        let mut db = MemDb::default();

        db.insert_account(
            test_addr.clone(),
            AccountInfo {
                balance: 123456.into(),
                code_hash: KECCAK_EMPTY,
                code: Some(contract_code.clone()),
                nonce: 1234,
            },
        );

        db.set_storage_at(test_addr, "0x1234567".into(), "0x1".into());
        db.set_storage_at(test_addr, "0x1234568".into(), "0x2".into());

        let mut new_state = SerializableState::default();

        new_state.accounts.insert(
            test_addr2,
            SerializableAccountRecord {
                balance: Default::default(),
                code: Default::default(),
                nonce: 1,
                storage: HashMap::default(),
            },
        );

        let mut new_storage = HashMap::new();
        new_storage.insert("0x1234568".into(), "0x5".into());

        new_state.accounts.insert(
            test_addr,
            SerializableAccountRecord {
                balance: 100100.into(),
                code: contract_code.clone().into(),
                nonce: 100,
                storage: new_storage,
            },
        );

        db.load_state(new_state);

        let loaded_account = db.basic(test_addr);
        let loaded_account2 = db.basic(test_addr2);

        assert_eq!(loaded_account2.nonce, 1);

        assert_eq!(loaded_account.balance, 100100.into());
        assert_eq!(db.code_by_hash(loaded_account.code_hash), contract_code);
        assert_eq!(loaded_account.nonce, 1234);
        assert_eq!(db.storage(test_addr, "0x1234567".into()), "0x1".into());
        assert_eq!(db.storage(test_addr, "0x1234568".into()), "0x5".into());
    }
}
