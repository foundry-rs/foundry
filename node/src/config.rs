use std::{collections::HashMap, time::Duration};

use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{Address, Wallet, U256},
};

pub const NODE_PORT: u16 = 8545;

/// Configurations of the EVM node
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Chain ID of the EVM chain
    pub(crate) chain_id: u64,
    /// Default gas limit for all txs
    pub(crate) gas_limit: U256,
    /// Default gas price for all txs
    pub(crate) gas_price: U256,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub(crate) genesis_accounts: Vec<Wallet<SigningKey>>,
    /// Native token balance of every genesis account in the genesis block
    pub(crate) genesis_balance: U256,
    /// Signer accounts that can sign messages/transactions from the EVM node
    pub(crate) accounts: HashMap<Address, Wallet<SigningKey>>,
    /// Configured block time for the EVM chain. Use `None` to mine a new block for every tx
    pub(crate) automine: Option<Duration>,
    /// port to use for the server
    pub(crate) port: u16,
    /// maximumg number of transactions in a block
    pub(crate) max_transactions: usize,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            chain_id: 1337,
            gas_limit: U256::from(100_000),
            gas_price: U256::from(1_000_000_000),
            genesis_accounts: Vec::new(),
            genesis_balance: U256::zero(),
            accounts: HashMap::new(),
            automine: None,
            port: NODE_PORT,
            // TODO make this something dependent on block capacity
            max_transactions: 1_000,
        }
    }
}

impl NodeConfig {
    /// Returns the default node configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the chain ID
    #[must_use]
    pub fn chain_id<U: Into<u64>>(mut self, chain_id: U) -> Self {
        self.chain_id = chain_id.into();
        self
    }

    /// Sets the gas limit
    #[must_use]
    pub fn gas_limit<U: Into<U256>>(mut self, gas_limit: U) -> Self {
        self.gas_limit = gas_limit.into();
        self
    }

    /// Sets the gas price
    #[must_use]
    pub fn gas_price<U: Into<U256>>(mut self, gas_price: U) -> Self {
        self.gas_price = gas_price.into();
        self
    }

    /// Sets the genesis accounts
    #[must_use]
    pub fn genesis_accounts(mut self, accounts: Vec<Wallet<SigningKey>>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the balance of the genesis accounts in the genesis block
    #[must_use]
    pub fn genesis_balance<U: Into<U256>>(mut self, balance: U) -> Self {
        self.genesis_balance = balance.into();
        self
    }

    /// Sets the block time to automine blocks
    #[must_use]
    pub fn automine<D: Into<Duration>>(mut self, block_time: D) -> Self {
        self.automine = Some(block_time.into());
        self
    }

    /// Sets the port to use
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}
