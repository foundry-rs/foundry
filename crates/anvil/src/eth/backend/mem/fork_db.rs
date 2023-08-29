use crate::{
    eth::backend::db::{
        Db, MaybeHashDatabase, SerializableAccountRecord, SerializableState, StateDb,
    },
    revm::primitives::AccountInfo,
    Address, U256,
};
use ethers::prelude::H256;
pub use foundry_evm::executor::fork::database::ForkedDatabase;
use foundry_evm::{
    executor::{
        backend::{snapshot::StateSnapshot, DatabaseResult},
        fork::database::ForkDbSnapshot,
    },
    revm::Database, utils::{h160_to_b160, u256_to_ru256, h256_to_b256, b160_to_h160, ru256_to_u256},
};

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) -> DatabaseResult<()> {
        // this ensures the account is loaded first
        let _ = Database::basic(self, h160_to_b160(address))?;
        self.database_mut().set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: H256) {
        self.inner().block_hashes().write().insert(u256_to_ru256(number), h256_to_b256(hash));
    }

    fn dump_state(&self) -> DatabaseResult<Option<SerializableState>> {
        let mut db = self.database().clone();
        let accounts = self
            .database()
            .accounts
            .clone()
            .into_iter()
            .map(|(k, v)| -> DatabaseResult<_> {
                let code = if let Some(code) = v.info.code {
                    code
                } else {
                    db.code_by_hash(v.info.code_hash)?
                }
                .to_checked();
                Ok((
                    b160_to_h160(k),
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: ru256_to_u256(v.info.balance),
                        code: code.bytes()[..code.len()].to_vec().into(),
                        storage: v
                            .storage
                            .into_iter()
                            .map(|kv| (ru256_to_u256(kv.0), ru256_to_u256(kv.1)))
                            .collect(),
                    },
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(SerializableState { accounts }))
    }

    fn snapshot(&mut self) -> U256 {
        self.insert_snapshot()
    }

    fn revert(&mut self, id: U256) -> bool {
        self.revert_snapshot(id)
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_snapshot())
    }
}

impl MaybeHashDatabase for ForkedDatabase {
    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = std::mem::take(&mut *db.accounts.write());
        let storage = std::mem::take(&mut *db.storage.write());
        let block_hashes = std::mem::take(&mut *db.block_hashes.write());
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn clear(&mut self) {
        self.flush_cache();
        self.clear_into_snapshot();
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        let db = self.inner().db();
        let StateSnapshot { accounts, storage, block_hashes } = snapshot;
        *db.accounts.write() = accounts;
        *db.storage.write() = storage;
        *db.block_hashes.write() = block_hashes;
    }
}
impl MaybeHashDatabase for ForkDbSnapshot {
    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        std::mem::take(&mut self.snapshot)
    }

    fn clear(&mut self) {
        std::mem::take(&mut self.snapshot);
        self.local.clear()
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        self.snapshot = snapshot;
    }
}
