//! Genesis settings

use crate::eth::backend::db::{Db, MaybeHashDatabase};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, B256, U256};
use foundry_evm::{
    backend::{DatabaseError, DatabaseResult, StateSnapshot},
    revm::{
        db::DatabaseRef,
        primitives::{AccountInfo, Bytecode, KECCAK_EMPTY},
    },
};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLockWriteGuard;

/// Genesis settings
#[derive(Clone, Debug, Default)]
pub struct GenesisConfig {
    /// The initial timestamp for the genesis block
    pub timestamp: u64,
    /// Balance for genesis accounts
    pub balance: U256,
    /// All accounts that should be initialised at genesis
    pub accounts: Vec<Address>,
    /// The account object stored in the [`revm::Database`]
    ///
    /// We store this for forking mode so we can cheaply reset the dev accounts and don't
    /// need to fetch them again.
    pub fork_genesis_account_infos: Arc<Mutex<Vec<AccountInfo>>>,
    /// The `genesis.json` if provided
    pub genesis_init: Option<Genesis>,
}

// === impl GenesisConfig ===

impl GenesisConfig {
    /// Returns fresh `AccountInfo`s for the configured `accounts`
    pub fn account_infos(&self) -> impl Iterator<Item = (Address, AccountInfo)> + '_ {
        self.accounts.iter().copied().map(|address| {
            let info = AccountInfo {
                balance: self.balance,
                code_hash: KECCAK_EMPTY,
                // we set this to empty so `Database::code_by_hash` doesn't get called
                code: Some(Default::default()),
                nonce: 0,
            };
            (address, info)
        })
    }

    /// If an initial `genesis.json` was provided, this applies the account alloc to the db
    pub fn apply_genesis_json_alloc(
        &self,
        mut db: RwLockWriteGuard<'_, Box<dyn Db>>,
    ) -> DatabaseResult<()> {
        if let Some(ref genesis) = self.genesis_init {
            for (addr, mut acc) in genesis.alloc.clone() {
                let storage = std::mem::take(&mut acc.storage);
                // insert all accounts
                db.insert_account(addr, self.genesis_to_account_info(&acc));
                // insert all storage values
                for (k, v) in storage.unwrap_or_default().iter() {
                    db.set_storage_at(addr, U256::from_be_bytes(k.0), U256::from_be_bytes(v.0))?;
                }
            }
        }
        Ok(())
    }

    /// Converts a [`GenesisAccount`] to an [`AccountInfo`]
    fn genesis_to_account_info(&self, acc: &GenesisAccount) -> AccountInfo {
        let GenesisAccount { code, balance, nonce, .. } = acc.clone();
        let code = code.map(|code| Bytecode::new_raw(code));
        AccountInfo {
            balance,
            nonce: nonce.unwrap_or_default(),
            code_hash: code.as_ref().map(|code| code.hash_slow()).unwrap_or(KECCAK_EMPTY),
            code,
        }
    }

    /// Returns a database wrapper that points to the genesis and is aware of all provided
    /// [AccountInfo]
    pub(crate) fn state_db_at_genesis<'a>(
        &self,
        db: Box<dyn MaybeHashDatabase + 'a>,
    ) -> AtGenesisStateDb<'a> {
        AtGenesisStateDb {
            genesis: self.genesis_init.clone(),
            accounts: self.account_infos().collect(),
            db,
        }
    }
}

/// A Database implementation that is at the genesis state.
///
/// This is only used in forking mode where we either need to fetch the state from remote if the
/// account was not provided via custom genesis, which would override anything available from remote
/// starting at the genesis, Note: "genesis" in the context of the Backend means, the block the
/// backend was created, which is `0` in normal mode and `fork block` in forking mode.
pub(crate) struct AtGenesisStateDb<'a> {
    genesis: Option<Genesis>,
    accounts: HashMap<Address, AccountInfo>,
    db: Box<dyn MaybeHashDatabase + 'a>,
}

impl<'a> DatabaseRef for AtGenesisStateDb<'a> {
    type Error = DatabaseError;
    fn basic_ref(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        if let Some(acc) = self.accounts.get(&(address)).cloned() {
            return Ok(Some(acc))
        }
        self.db.basic_ref(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> DatabaseResult<Bytecode> {
        if let Some((_, acc)) = self.accounts.iter().find(|(_, acc)| acc.code_hash == code_hash) {
            return Ok(acc.code.clone().unwrap_or_default())
        }
        self.db.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        if let Some(acc) = self.genesis.as_ref().and_then(|genesis| genesis.alloc.get(&(address))) {
            if let Some(storage) = acc.storage.as_ref() {
                return Ok(U256::from_be_bytes(
                    storage.get(&B256::from(index)).copied().unwrap_or_default().0,
                ))
            }
        }
        self.db.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: U256) -> DatabaseResult<B256> {
        self.db.block_hash_ref(number)
    }
}

impl<'a> MaybeHashDatabase for AtGenesisStateDb<'a> {
    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        self.db.clear_into_snapshot()
    }

    fn clear(&mut self) {
        self.db.clear()
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        self.db.init_from_snapshot(snapshot)
    }
}
