//! Genesis settings

use ethers::types::{Address, U256};

/// Genesis settings
#[derive(Debug, Clone, Default)]
pub struct GenesisConfig {
    /// Balance for genesis accounts
    pub balance: U256,
    /// All accounts that should be initialised at genesis
    pub accounts: Vec<Address>,
}
