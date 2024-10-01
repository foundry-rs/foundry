use crate::{
    eth::backend::db::{
        Db, MaybeForkedDatabase, MaybeFullDatabase, SerializableAccountRecord, SerializableBlock,
        SerializableHistoricalStates, SerializableState, SerializableTransaction, StateDb,
    },
    revm::primitives::AccountInfo,
};
use alloy_primitives::{Address, B256, U256, U64};
use alloy_rpc_types::BlockId;
use foundry_evm::{
    backend::{
        BlockchainDb, DatabaseError, DatabaseResult, RevertStateSnapshotAction, StateSnapshot,
    },
    fork::database::ForkDbStateSnapshot,
    revm::{primitives::BlockEnv, Database},
};
use revm::DatabaseRef;

pub use foundry_evm::fork::database::ForkedDatabase;

impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
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
        historical_states: Option<SerializableHistoricalStates>,
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
                        storage: v.storage.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
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
            historical_states,
        }))
    }

    fn snapshot_state(&mut self) -> U256 {
        self.insert_state_snapshot()
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        self.revert_state_snapshot(id, action)
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_state_snapshot())
    }
}

impl MaybeFullDatabase for ForkedDatabase {
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = std::mem::take(&mut *db.accounts.write());
        let storage = std::mem::take(&mut *db.storage.write());
        let block_hashes = std::mem::take(&mut *db.block_hashes.write());
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = db.accounts.read().clone();
        let storage = db.storage.read().clone();
        let block_hashes = db.block_hashes.read().clone();
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn clear(&mut self) {
        self.flush_cache();
        self.clear_into_state_snapshot();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        let db = self.inner().db();
        let StateSnapshot { accounts, storage, block_hashes } = state_snapshot;
        *db.accounts.write() = accounts;
        *db.storage.write() = storage;
        *db.block_hashes.write() = block_hashes;
    }
}

impl MaybeFullDatabase for ForkDbStateSnapshot {
    fn as_dyn(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        std::mem::take(&mut self.state_snapshot)
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.state_snapshot.clone()
    }

    fn clear(&mut self) {
        std::mem::take(&mut self.state_snapshot);
        self.local.clear()
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        self.state_snapshot = state_snapshot;
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
