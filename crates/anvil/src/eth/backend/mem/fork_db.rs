use crate::{
    eth::backend::db::{
        Db, MaybeForkedDatabase, MaybeFullDatabase, SerializableAccountRecord, SerializableBlock,
        SerializableState, SerializableTransaction, StateDb,
    },
    revm::primitives::AccountInfo,
};
use alloy_primitives::{Address, B256, U256, U64};
use alloy_rpc_types::BlockId;
use foundry_evm::{
    backend::{BlockchainDb, DatabaseResult, RevertSnapshotAction, StateSnapshot},
    fork::database::ForkDbSnapshot,
    revm::Database,
};

pub use foundry_evm::fork::database::ForkedDatabase;
use foundry_evm::revm::primitives::BlockEnv;

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) -> DatabaseResult<()> {
        // this ensures the account is loaded first
        let _ = Database::basic(self, address)?;
        self.database_mut().set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.inner().block_hashes().write().insert(number, hash);
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: U64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
    ) -> DatabaseResult<Option<SerializableState>> {
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
                };
                Ok((
                    k,
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: v.info.balance,
                        code: code.original_bytes(),
                        storage: v.storage.into_iter().collect(),
                    },
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(SerializableState {
            block: Some(at),
            accounts,
            best_block_number: Some(best_number),
            blocks,
            transactions,
        }))
    }

    fn snapshot(&mut self) -> U256 {
        self.insert_snapshot()
    }

    fn revert(&mut self, id: U256, action: RevertSnapshotAction) -> bool {
        self.revert_snapshot(id, action)
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_snapshot())
    }
}

impl MaybeFullDatabase for ForkedDatabase {
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

impl MaybeFullDatabase for ForkDbSnapshot {
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

impl MaybeForkedDatabase for ForkedDatabase {
    fn maybe_reset(&mut self, url: Option<String>, block_number: BlockId) -> Result<(), String> {
        self.reset(url, block_number)
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.flush_cache();
        Ok(())
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        Ok(self.inner())
    }
}
