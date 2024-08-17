//! Genesis settings

use crate::eth::backend::db::Db;
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, U256};
use foundry_evm::{
    backend::DatabaseResult,
    revm::primitives::{AccountInfo, Bytecode, KECCAK_EMPTY},
};
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
    /// The `genesis.json` if provided
    pub genesis_init: Option<Genesis>,
}

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
        let code = code.map(Bytecode::new_raw);
        AccountInfo {
            balance,
            nonce: nonce.unwrap_or_default(),
            code_hash: code.as_ref().map(|code| code.hash_slow()).unwrap_or(KECCAK_EMPTY),
            code,
        }
    }
}
