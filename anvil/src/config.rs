use crate::{
    eth::{
        backend::{
            db::Db,
            fork::{ClientFork, ClientForkConfig},
            genesis::GenesisConfig,
            mem::fork_db::ForkedDatabase,
        },
        fees::INITIAL_BASE_FEE,
        pool::transactions::TransactionOrder,
    },
    mem,
    mem::in_memory_db::MemDb,
    FeeManager,
};
use anvil_server::ServerConfig;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{rand::thread_rng, Wallet, U256},
    providers::{Http, Middleware, Provider, RetryClient},
    signers::{
        coins_bip39::{English, Mnemonic},
        MnemonicBuilder, Signer,
    },
    types::BlockNumber,
    utils::{format_ether, hex, WEI_IN_ETHER},
};
use foundry_config::Config;
use foundry_evm::{
    executor::fork::{BlockchainDb, BlockchainDbMeta, SharedBackend},
    revm,
    revm::{BlockEnv, CfgEnv, SpecId, TxEnv},
};
use parking_lot::RwLock;
use std::{net::IpAddr, path::PathBuf, str::FromStr, sync::Arc, time::Duration};
use yansi::Paint;

/// Default port the rpc will open
pub const NODE_PORT: u16 = 8545;
/// Default chain id of the node
pub const CHAIN_ID: u64 = 31337;
/// Default mnemonic for dev accounts
pub const DEFAULT_MNEMONIC: &str = "test test test test test test test test test test test junk";

/// `anvil 0.1.0 (f01b232bc 2022-04-13T23:28:39.493201+00:00)`
pub const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("VERGEN_GIT_SHA_SHORT"),
    " ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    ")"
);

const BANNER: &str = r#"
                             _   _
                            (_) | |
      __ _   _ __   __   __  _  | |
     / _` | | '_ \  \ \ / / | | | |
    | (_| | | | | |  \ V /  | | | |
     \__,_| |_| |_|   \_/   |_| |_|
"#;

/// Configurations of the EVM node
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Chain ID of the EVM chain
    pub chain_id: u64,
    /// Default gas limit for all txs
    pub gas_limit: U256,
    /// Default gas price for all txs
    pub gas_price: U256,
    /// Default base fee
    pub base_fee: U256,
    /// The hardfork to use
    pub hardfork: Hardfork,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub genesis_accounts: Vec<Wallet<SigningKey>>,
    /// Native token balance of every genesis account in the genesis block
    pub genesis_balance: U256,
    /// Signer accounts that can sign messages/transactions from the EVM node
    pub signer_accounts: Vec<Wallet<SigningKey>>,
    /// Configured block time for the EVM chain. Use `None` to mine a new block for every tx
    pub block_time: Option<Duration>,
    /// Disable auto, interval mining mode uns use `MiningMode::None` instead
    pub no_mining: bool,
    /// port to use for the server
    pub port: u16,
    /// maximum number of transactions in a block
    pub max_transactions: usize,
    /// don't print anything on startup
    pub silent: bool,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,
    /// The generator used to generate the dev accounts
    pub account_generator: Option<AccountGenerator>,
    /// whether to enable tracing
    pub enable_tracing: bool,
    /// Explicitly disables the use of RPC caching.
    pub no_storage_caching: bool,
    /// How to configure the server
    pub server_config: ServerConfig,
    /// The host the server will listen on
    pub host: Option<IpAddr>,
    /// How transactions are sorted in the mempool
    pub transaction_order: TransactionOrder,
}

// === impl NodeConfig ===

impl NodeConfig {
    /// Test config
    #[doc(hidden)]
    pub fn test() -> Self {
        Self { enable_tracing: false, silent: true, ..Default::default() }
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        // generate some random wallets
        let genesis_accounts = AccountGenerator::new(10).phrase(DEFAULT_MNEMONIC).gen();
        Self {
            chain_id: CHAIN_ID,
            gas_limit: U256::from(30_000_000),
            gas_price: U256::from(20_000_000_000u64),
            hardfork: Hardfork::default(),
            signer_accounts: genesis_accounts.clone(),
            genesis_accounts,
            // 100ETH default balance
            genesis_balance: WEI_IN_ETHER.saturating_mul(100u64.into()),
            block_time: None,
            no_mining: false,
            port: NODE_PORT,
            // TODO make this something dependent on block capacity
            max_transactions: 1_000,
            silent: false,
            eth_rpc_url: None,
            fork_block_number: None,
            account_generator: None,
            base_fee: INITIAL_BASE_FEE.into(),
            enable_tracing: true,
            no_storage_caching: false,
            server_config: Default::default(),
            host: None,
            transaction_order: Default::default(),
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
    pub fn with_chain_id<U: Into<u64>>(mut self, chain_id: U) -> Self {
        self.set_chain_id(chain_id.into());
        self
    }

    /// Sets the chain id and updates all wallets
    pub fn set_chain_id(&mut self, chain_id: impl Into<u64>) {
        self.chain_id = chain_id.into();
        self.genesis_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(self.chain_id);
        });
        self.signer_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(self.chain_id);
        })
    }

    /// Sets the gas limit
    #[must_use]
    pub fn with_gas_limit<U: Into<U256>>(mut self, gas_limit: Option<U>) -> Self {
        if let Some(gas_limit) = gas_limit {
            self.gas_limit = gas_limit.into();
        }
        self
    }

    /// Sets the gas price
    #[must_use]
    pub fn with_gas_price<U: Into<U256>>(mut self, gas_price: Option<U>) -> Self {
        if let Some(gas_price) = gas_price {
            self.gas_price = gas_price.into();
        }
        self
    }

    /// Sets the base fee
    #[must_use]
    pub fn with_base_fee<U: Into<U256>>(mut self, base_fee: Option<U>) -> Self {
        if let Some(base_fee) = base_fee {
            self.base_fee = base_fee.into();
        }
        self
    }

    /// Sets the hardfork
    #[must_use]
    pub fn with_hardfork(mut self, hardfork: Hardfork) -> Self {
        self.hardfork = hardfork;
        self
    }

    /// Sets the genesis accounts
    #[must_use]
    pub fn with_genesis_accounts(mut self, accounts: Vec<Wallet<SigningKey>>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the signer accounts
    #[must_use]
    pub fn with_signer_accounts(mut self, accounts: Vec<Wallet<SigningKey>>) -> Self {
        self.signer_accounts = accounts;
        self
    }

    /// Sets both the genesis accounts and the signer accounts
    /// so that `genesis_accounts == accounts`
    #[must_use]
    pub fn with_account_generator(mut self, generator: AccountGenerator) -> Self {
        let accounts = generator.gen();
        self.account_generator = Some(generator);
        self.with_signer_accounts(accounts.clone()).with_genesis_accounts(accounts)
    }

    /// Sets the balance of the genesis accounts in the genesis block
    #[must_use]
    pub fn with_genesis_balance<U: Into<U256>>(mut self, balance: U) -> Self {
        self.genesis_balance = balance.into();
        self
    }

    /// Sets the block time to automine blocks
    #[must_use]
    pub fn with_blocktime<D: Into<Duration>>(mut self, block_time: Option<D>) -> Self {
        self.block_time = block_time.map(Into::into);
        self
    }

    /// If set to `true` auto mining will be disabled
    #[must_use]
    pub fn with_no_mining(mut self, no_mining: bool) -> Self {
        self.no_mining = no_mining;
        self
    }

    /// Sets the port to use
    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
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

    /// Makes the node silent to not emit anything on stdout
    #[must_use]
    pub fn no_storage_caching(self) -> Self {
        self.with_storage_caching(true)
    }

    #[must_use]
    pub fn with_storage_caching(mut self, storage_caching: bool) -> Self {
        self.no_storage_caching = storage_caching;
        self
    }

    /// Sets the `eth_rpc_url` to use when forking
    #[must_use]
    pub fn with_eth_rpc_url<U: Into<String>>(mut self, eth_rpc_url: Option<U>) -> Self {
        self.eth_rpc_url = eth_rpc_url.map(Into::into);
        self
    }

    /// Sets the `fork_block_number` to use to fork off from
    #[must_use]
    pub fn with_fork_block_number<U: Into<u64>>(mut self, fork_block_number: Option<U>) -> Self {
        self.fork_block_number = fork_block_number.map(Into::into);
        self
    }

    /// Sets whether to enable tracing
    #[must_use]
    pub fn with_tracing(mut self, enable_tracing: bool) -> Self {
        self.enable_tracing = enable_tracing;
        self
    }

    #[must_use]
    pub fn with_server_config(mut self, config: ServerConfig) -> Self {
        self.server_config = config;
        self
    }

    /// Sets the host the server will listen on
    #[must_use]
    pub fn with_host(mut self, host: Option<IpAddr>) -> Self {
        self.host = host;
        self
    }

    #[must_use]
    pub fn with_transaction_order(mut self, transaction_order: TransactionOrder) -> Self {
        self.transaction_order = transaction_order;
        self
    }

    /// Prints the config info
    pub fn print(&self, fork: Option<&ClientFork>) {
        if self.silent {
            return
        }
        println!("{}", Paint::green(BANNER));
        println!("    {}", VERSION_MESSAGE);
        println!("    {}", Paint::green("https://github.com/foundry-rs/foundry"));

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

        if let Some(ref gen) = self.account_generator {
            print!(
                r#"
Wallet
==================
Mnemonic:          {}
Derivation path:   {}
"#,
                gen.phrase,
                gen.get_derivation_path()
            );
        }

        print!(
            r#"
Base Fee
==================
{}
"#,
            Paint::green(format!("{}", self.base_fee))
        );
        print!(
            r#"
Gas Price
==================
{}
"#,
            Paint::green(format!("{}", self.gas_price))
        );

        print!(
            r#"
Gas Limit
==================
{}
"#,
            Paint::green(format!("{}", self.gas_limit))
        );

        if let Some(fork) = fork {
            print!(
                r#"
Fork
==================
Endpoint:       {}
Block number:   {}
Block hash:     {:?}
Chain ID:       {}
"#,
                fork.eth_rpc_url(),
                fork.block_number(),
                fork.block_hash(),
                fork.chain_id()
            );
        }

        println!();
    }

    /// Returns the path where the cache file should be stored
    ///
    /// See also [ Config::foundry_block_cache_file()]
    pub fn block_cache_path(&self) -> Option<PathBuf> {
        if self.no_storage_caching || self.eth_rpc_url.is_none() {
            return None
        }
        // cache only if block explicitly set
        let block = self.fork_block_number?;
        let chain_id = self.chain_id;

        Config::foundry_block_cache_file(chain_id, block)
    }

    /// Configures everything related to env, backend and database and returns the
    /// [Backend](mem::Backend)
    ///
    /// *Note*: only memory based backend for now
    pub(crate) async fn setup(&mut self) -> mem::Backend {
        // configure the revm environment
        let mut env = revm::Env {
            cfg: CfgEnv {
                spec_id: self.hardfork.into(),
                chain_id: self.chain_id.into(),
                ..Default::default()
            },
            block: BlockEnv {
                gas_limit: self.gas_limit,
                basefee: self.base_fee,
                ..Default::default()
            },
            tx: TxEnv { chain_id: Some(self.chain_id), ..Default::default() },
        };
        let fees = FeeManager::new(self.base_fee, self.gas_price);
        let mut fork_timestamp = None;

        let (db, fork): (Arc<RwLock<dyn Db>>, Option<ClientFork>) = if let Some(eth_rpc_url) =
            self.eth_rpc_url.clone()
        {
            // TODO make provider agnostic
            let provider = Arc::new(
                Provider::<RetryClient<Http>>::new_client(&eth_rpc_url, 10, 1000)
                    .expect("Failed to establish provider to fork url"),
            );

            let fork_block_number = if let Some(fork_block_number) = self.fork_block_number {
                fork_block_number
            } else {
                provider.get_block_number().await.expect("Failed to get fork block number").as_u64()
            };

            let block = provider
                .get_block(BlockNumber::Number(fork_block_number.into()))
                .await
                .expect("Failed to get fork block")
                .unwrap_or_else(|| panic!("Failed to get fork block"));

            env.block.number = fork_block_number.into();
            fork_timestamp = Some(block.timestamp);

            let block_hash = block.hash.unwrap();
            let chain_id = provider.get_chainid().await.unwrap().as_u64();
            // need to update the dev signers and env with the chain id
            self.set_chain_id(chain_id);
            env.cfg.chain_id = chain_id.into();
            env.tx.chain_id = chain_id.into();

            let meta = BlockchainDbMeta::new(env.clone(), eth_rpc_url.clone());

            let block_chain_db = BlockchainDb::new(meta, self.block_cache_path());

            // This will spawn the background thread that will use the provider to fetch blockchain
            // data from the other client
            let backend = SharedBackend::spawn_backend_thread(
                Arc::clone(&provider),
                block_chain_db.clone(),
                Some(fork_block_number.into()),
            );

            let db = Arc::new(RwLock::new(ForkedDatabase::new(backend, block_chain_db)));
            let fork = ClientFork::new(
                ClientForkConfig {
                    eth_rpc_url,
                    block_number: fork_block_number,
                    block_hash,
                    provider,
                    chain_id,
                    timestamp: block.timestamp.as_u64(),
                },
                Arc::clone(&db),
            );

            (db, Some(fork))
        } else {
            (Arc::new(RwLock::new(MemDb::default())), None)
        };

        let genesis = GenesisConfig {
            balance: self.genesis_balance,
            accounts: self.genesis_accounts.iter().map(|acc| acc.address()).collect(),
        };
        // only memory based backend for now

        let backend =
            mem::Backend::with_genesis(db, Arc::new(RwLock::new(env)), genesis, fees, fork);

        if let Some(timestamp) = fork_timestamp {
            backend.time().set_start_timestamp(timestamp.as_u64());
        }
        backend
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Hardfork {
    Frontier,
    Homestead,
    Tangerine,
    SpuriousDragon,
    Byzantine,
    Constantinople,
    Petersburg,
    Istanbul,
    Muirglacier,
    Berlin,
    London,
    Latest,
}

impl FromStr for Hardfork {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let hardfork = match s.as_str() {
            "frontier" | "1" => Hardfork::Frontier,
            "homestead" | "2" => Hardfork::Homestead,
            "tangerine" | "3" => Hardfork::Tangerine,
            "spuriousdragon" | "4" => Hardfork::SpuriousDragon,
            "byzantine" | "5" => Hardfork::Byzantine,
            "constantinople" | "6" => Hardfork::Constantinople,
            "petersburg" | "7" => Hardfork::Petersburg,
            "istanbul" | "8" => Hardfork::Istanbul,
            "muirglacier" | "9" => Hardfork::Muirglacier,
            "berlin" | "10" => Hardfork::Berlin,
            "london" | "11" => Hardfork::London,
            "latest" | "12" => Hardfork::Latest,
            _ => return Err(format!("Unknown hardfork {}", s)),
        };
        Ok(hardfork)
    }
}

impl Default for Hardfork {
    fn default() -> Self {
        Hardfork::Latest
    }
}

impl From<Hardfork> for SpecId {
    fn from(fork: Hardfork) -> Self {
        match fork {
            Hardfork::Frontier => SpecId::FRONTIER,
            Hardfork::Homestead => SpecId::HOMESTEAD,
            Hardfork::Tangerine => SpecId::TANGERINE,
            Hardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
            Hardfork::Byzantine => SpecId::BYZANTINE,
            Hardfork::Constantinople => SpecId::CONSTANTINOPLE,
            Hardfork::Petersburg => SpecId::PETERSBURG,
            Hardfork::Istanbul => SpecId::ISTANBUL,
            Hardfork::Muirglacier => SpecId::MUIRGLACIER,
            Hardfork::Berlin => SpecId::BERLIN,
            Hardfork::London => SpecId::LONDON,
            Hardfork::Latest => SpecId::LATEST,
        }
    }
}

/// Can create dev accounts
#[derive(Debug, Clone)]
pub struct AccountGenerator {
    chain_id: u64,
    amount: usize,
    phrase: String,
    derivation_path: Option<String>,
}

impl AccountGenerator {
    pub fn new(amount: usize) -> Self {
        Self {
            chain_id: CHAIN_ID,
            amount,
            phrase: Mnemonic::<English>::new(&mut thread_rng())
                .to_phrase()
                .expect("Failed to create mnemonic phrase"),
            derivation_path: None,
        }
    }

    #[must_use]
    pub fn phrase(mut self, phrase: impl Into<String>) -> Self {
        self.phrase = phrase.into();
        self
    }

    #[must_use]
    pub fn chain_id(mut self, chain_id: impl Into<u64>) -> Self {
        self.chain_id = chain_id.into();
        self
    }

    #[must_use]
    pub fn derivation_path(mut self, derivation_path: impl Into<String>) -> Self {
        let mut derivation_path = derivation_path.into();
        if !derivation_path.ends_with('/') {
            derivation_path.push('/');
        }
        self.derivation_path = Some(derivation_path);
        self
    }

    fn get_derivation_path(&self) -> &str {
        self.derivation_path.as_deref().unwrap_or("m/44'/60'/0'/0/")
    }
}

impl AccountGenerator {
    pub fn gen(&self) -> Vec<Wallet<SigningKey>> {
        let builder = MnemonicBuilder::<English>::default().phrase(self.phrase.as_str());

        // use the
        let derivation_path = self.get_derivation_path();

        let mut wallets = Vec::with_capacity(self.amount);

        for idx in 0..self.amount {
            let builder =
                builder.clone().derivation_path(&format!("{}{}", derivation_path, idx)).unwrap();
            let wallet = builder.build().unwrap().with_chain_id(self.chain_id);
            wallets.push(wallet)
        }
        wallets
    }
}
