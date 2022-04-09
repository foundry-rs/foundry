use colored::Colorize;
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    eth::backend::db::{Db, ForkedDatabase},
    fork::ForkInfo,
    mem,
    revm::db::CacheDB,
};
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{Address, Wallet, U256},
    providers::{Middleware, Provider},
    signers::{coins_bip39::English, MnemonicBuilder, Signer},
    utils::{format_ether, hex, WEI_IN_ETHER},
};
use foundry_evm::{
    executor::fork::{BlockchainDb, BlockchainDbMeta, SharedBackend},
    revm,
    revm::{BlockEnv, CfgEnv, TxEnv},
};
use parking_lot::RwLock;

pub const NODE_PORT: u16 = 8545;

/// Configurations of the EVM node
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Chain ID of the EVM chain
    pub chain_id: u64,
    /// Default gas limit for all txs
    pub gas_limit: U256,
    /// Default gas price for all txs
    pub gas_price: U256,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub genesis_accounts: Vec<Wallet<SigningKey>>,
    /// Native token balance of every genesis account in the genesis block
    pub genesis_balance: U256,
    /// Signer accounts that can sign messages/transactions from the EVM node
    pub accounts: HashMap<Address, Wallet<SigningKey>>,
    /// Configured block time for the EVM chain. Use `None` to mine a new block for every tx
    pub automine: Option<Duration>,
    /// port to use for the server
    pub port: u16,
    /// maximumg number of transactions in a block
    pub max_transactions: usize,
    /// don't print anything on startup
    pub silent: bool,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        // generate some random wallets
        let genesis_accounts = random_wallets(10);
        Self {
            chain_id: 1337,
            gas_limit: U256::from(6_721_975),
            gas_price: U256::from(20_000_000_000u64),
            accounts: genesis_accounts.iter().map(|w| (w.address(), w.clone())).collect(),
            genesis_accounts,
            // 100ETH default balance
            genesis_balance: WEI_IN_ETHER.saturating_mul(100u64.into()),
            automine: None,
            port: NODE_PORT,
            // TODO make this something dependent on block capacity
            max_transactions: 1_000,
            silent: false,
            eth_rpc_url: None,
            fork_block_number: None,
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

    /// Makes the node silent to not emit anything on stdout
    #[must_use]
    pub fn silent(self) -> Self {
        self.set_silent(true)
    }

    #[must_use]
    pub fn set_silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    /// Sets the `eth_rpc_url` to use when forking
    #[must_use]
    pub fn eth_rpc_url<U: Into<String>>(mut self, eth_rpc_url: U) -> Self {
        self.eth_rpc_url = Some(eth_rpc_url.into());
        self
    }

    /// Sets the `fork_block_number` to use to fork off from
    #[must_use]
    pub fn fork_block_number<U: Into<u64>>(mut self, fork_block_number: U) -> Self {
        self.fork_block_number = Some(fork_block_number.into());
        self
    }

    /// Prints the config info
    pub fn print(&self) {
        if self.silent {
            return
        }
        println!("  {}", BANNER.green());
        println!("      {}", "https://github.com/gakonst/foundry".green());

        print!(
            r#"
Available Accounts
==================
"#
        );
        let balance = format_ether(self.genesis_balance);
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            println!("({}) {:?} ({} ETH)", idx, wallet.address(), balance);
        }
        println!();

        print!(
            r#"
Private Keys
==================
"#
        );

        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let hex = hex::encode(wallet.signer().to_bytes());
            println!("({}) 0x{}", idx, hex);
        }

        // TODO also print the Mnemonic used to gen keys

        print!(
            r#"
Gas Price
==================
{}
"#,
            format!("{}", self.gas_price).green()
        );

        print!(
            r#"
Gas Limit
==================
{}

"#,
            format!("{}", self.gas_limit).green()
        );
    }

    /// Configures everything related to env, backend and database and returns the
    /// [Backend](mem::Backend)
    ///
    /// *Note*: only memory based backend for now
    pub(crate) async fn setup(&self) -> mem::Backend {
        // configure the revm environment
        let mut env = revm::Env {
            cfg: CfgEnv { ..Default::default() },
            block: BlockEnv { gas_limit: self.gas_limit, ..Default::default() },
            tx: TxEnv { chain_id: Some(self.chain_id), ..Default::default() },
        };

        let (db, fork): (Arc<RwLock<dyn Db>>, Option<ForkInfo>) = if let Some(eth_rpc_url) =
            self.eth_rpc_url.clone()
        {
            // TODO make provider agnostic
            let provider = Arc::new(
                Provider::try_from(&eth_rpc_url).expect("Failed to establish provider to fork url"),
            );

            let fork_block_number = if let Some(fork_block_number) = self.fork_block_number {
                fork_block_number
            } else {
                provider.get_block_number().await.expect("Failed to get fork block number").as_u64()
            };
            env.block.number = fork_block_number.into();

            let block_hash =
                provider.get_block(fork_block_number).await.unwrap().unwrap().hash.unwrap();

            let meta = BlockchainDbMeta::new(env.clone(), eth_rpc_url.clone());

            // TODO support cache path
            let block_chain_db = BlockchainDb::new(meta, None);
            let db = Arc::clone(block_chain_db.db());

            // This will spawn the background service that will use the provider to fetch blockchain
            // data from the other client
            let backend = SharedBackend::spawn_backend(
                Arc::clone(&provider),
                block_chain_db,
                Some(fork_block_number.into()),
            )
            .await;

            let db = Arc::new(RwLock::new(ForkedDatabase::new(backend, db)));

            let fork =
                ForkInfo { eth_rpc_url, block_number: fork_block_number, block_hash, provider };

            (db, Some(fork))
        } else {
            (Arc::new(RwLock::new(CacheDB::default())), None)
        };

        // only memory based backend for now
        let backend = mem::Backend::with_genesis_balance(
            db,
            Arc::new(RwLock::new(env)),
            self.genesis_balance,
            self.genesis_accounts.iter().map(|acc| acc.address()),
            self.gas_price,
            fork,
        );

        backend
    }
}

/// Generates random private-public key pair which can be used for signing messages
pub fn random_wallets(num_accounts: usize) -> Vec<Wallet<SigningKey>> {
    let builder = MnemonicBuilder::<English>::default()
        .phrase("member yard spread wall vanish absorb hill lawn fetch equal purse shiver");
    let mut wallets = Vec::with_capacity(num_accounts);

    for i in 0..num_accounts {
        let wallet = builder.clone().index(i as u32).unwrap().build().unwrap();
        wallets.push(wallet)
    }
    wallets
}

const BANNER: &str = r#"
                          _   _
                         (_) | |
   __ _   _ __   __   __  _  | |
  / _` | | '_ \  \ \ / / | | | |
 | (_| | | | | |  \ V /  | | | |
  \__,_| |_| |_|   \_/   |_| |_|
"#;
