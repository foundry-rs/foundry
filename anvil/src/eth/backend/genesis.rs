//! Genesis settings

use ethers::types::{Address, U256};
use forge::revm::KECCAK_EMPTY;
use foundry_evm::revm::AccountInfo;
use parking_lot::Mutex;
use std::sync::Arc;

/// Genesis settings
#[derive(Debug, Clone, Default)]
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
}
