//! Genesis settings

use crate::eth::backend::db::{AccountInfo, Bytecode, DatabaseResult, Db, KECCAK_EMPTY};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, U256};

/// Genesis settings
#[derive(Clone, Debug, Default)]
pub struct GenesisConfig {
    /// The initial number for the genesis block
    pub number: u64,
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
            // The empty code is set so `code_by_hash` doesn't get called.
            let info = AccountInfo::new(self.balance, 0, KECCAK_EMPTY, Some(Default::default()));
            (address, info)
        })
    }

    /// If an initial `genesis.json` was provided, this applies the account alloc to the db
    pub fn apply_genesis_json_alloc(&self, db: &mut dyn Db) -> DatabaseResult<()> {
        if let Some(ref genesis) = self.genesis_init {
            for (addr, mut acc) in genesis.alloc.clone() {
                let storage = std::mem::take(&mut acc.storage);
                // insert all accounts
                db.insert_account(addr, self.genesis_to_account_info(&acc));
                // insert all storage values
                for (k, v) in &storage.unwrap_or_default() {
                    db.set_storage_at(addr, *k, *v)?;
                }
            }
        }
        Ok(())
    }

    /// Converts a [`GenesisAccount`] to an [`AccountInfo`]
    fn genesis_to_account_info(&self, acc: &GenesisAccount) -> AccountInfo {
        let GenesisAccount { code, balance, nonce, .. } = acc.clone();
        let code = code.map(Bytecode::new_raw);
        AccountInfo::new(
            balance,
            nonce.unwrap_or_default(),
            code.as_ref().map(|code| code.hash_slow()).unwrap_or(KECCAK_EMPTY),
            code,
        )
    }
}
