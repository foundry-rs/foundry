use crate::{
    eth::{
        backend::{
            db::Db,
            fork::{ClientFork, ClientForkConfig},
            genesis::GenesisConfig,
            mem::fork_db::ForkedDatabase,
            time::duration_since_unix_epoch,
        },
        fees::{INITIAL_BASE_FEE, INITIAL_GAS_PRICE},
        pool::transactions::TransactionOrder,
    },
    genesis::Genesis,
    mem,
    mem::in_memory_db::MemDb,
    FeeManager, Hardfork,
};
use anvil_server::ServerConfig;
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{rand::thread_rng, Wallet, U256},
    providers::Middleware,
    signers::{
        coins_bip39::{English, Mnemonic},
        MnemonicBuilder, Signer,
    },
    types::BlockNumber,
    utils::{format_ether, hex, WEI_IN_ETHER},
};
use foundry_common::{ProviderBuilder, ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT};
use foundry_config::Config;
use foundry_evm::{
    executor::fork::{BlockchainDb, BlockchainDbMeta, SharedBackend},
    revm,
    revm::{BlockEnv, CfgEnv, SpecId, TxEnv},
};
use parking_lot::RwLock;
use serde_json::{json, to_writer, Value};
use std::{
    collections::HashMap, fmt::Write as FmtWrite, fs::File, net::IpAddr, path::PathBuf, sync::Arc,
    time::Duration,
};
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
    pub chain_id: Option<u64>,
    /// Default gas limit for all txs
    pub gas_limit: U256,
    /// Default gas price for all txs
    pub gas_price: Option<U256>,
    /// Default base fee
    pub base_fee: Option<U256>,
    /// The hardfork to use
    pub hardfork: Option<Hardfork>,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub genesis_accounts: Vec<Wallet<SigningKey>>,
    /// Native token balance of every genesis account in the genesis block
    pub genesis_balance: U256,
    /// Genesis block timestamp
    pub genesis_timestamp: Option<u64>,
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
    /// Filename to write anvil output as json
    pub config_out: Option<String>,
    /// The genesis to use to initialize the node
    pub genesis: Option<Genesis>,
    /// Timeout in for requests sent to remote JSON-RPC server in forking mode
    pub fork_request_timeout: Duration,
    /// Number of request retries for spurious networks
    pub fork_request_retries: u32,
    /// The initial retry backoff
    pub fork_retry_backoff: Duration,
    /// available CUPS
    pub compute_units_per_second: u64,
    /// The ipc path
    pub ipc_path: Option<Option<String>>,
    /// Enable transaction/call steps tracing for debug calls returning geth-style traces
    pub enable_steps_tracing: bool,
}

impl NodeConfig {
    fn as_string(&self, fork: Option<&ClientFork>) -> String {
        let mut config_string: String = "".to_owned();
        let _ = write!(config_string, "\n{}", Paint::green(BANNER));
        let _ = write!(config_string, "\n    {}", VERSION_MESSAGE);
        let _ = write!(
            config_string,
            "\n    {}",
            Paint::green("https://github.com/foundry-rs/foundry")
        );

        let _ = write!(
            config_string,
            r#"

Available Accounts
==================
"#
        );
        let balance = format_ether(self.genesis_balance);
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let _ = write!(config_string, "\n({}) {:?} ({} ETH)", idx, wallet.address(), balance);
        }

        let _ = write!(
            config_string,
            r#"

Private Keys
==================
"#
        );

        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let hex = hex::encode(wallet.signer().to_bytes());
            let _ = write!(config_string, "\n({}) 0x{}", idx, hex);
        }

        if let Some(ref gen) = self.account_generator {
            let _ = write!(
                config_string,
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

        if (SpecId::from(self.get_hardfork()) as u8) < (SpecId::LONDON as u8) {
            let _ = write!(
                config_string,
                r#"
Gas Price
==================
{}
"#,
                Paint::green(format!("\n{}", self.get_gas_price()))
            );
        } else {
            let _ = write!(
                config_string,
                r#"

Base Fee
==================
{}
"#,
                Paint::green(format!("\n{}", self.get_base_fee()))
            );
        }

        let _ = write!(
            config_string,
            r#"
Gas Limit
==================
{}
"#,
            Paint::green(format!("\n{}", self.gas_limit))
        );

        let _ = write!(
            config_string,
            r#"
Genesis Timestamp
==================
{}
"#,
            Paint::green(format!("\n{}", self.get_genesis_timestamp()))
        );

        if let Some(fork) = fork {
            let _ = write!(
                config_string,
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

        config_string
    }

    fn as_json(&self, fork: Option<&ClientFork>) -> Value {
        let mut wallet_description = HashMap::new();
        let mut available_accounts = Vec::with_capacity(self.genesis_accounts.len());
        let mut private_keys = Vec::with_capacity(self.genesis_accounts.len());

        for wallet in &self.genesis_accounts {
            available_accounts.push(format!("{:?}", wallet.address()));
            private_keys.push(format!("0x{}", hex::encode(wallet.signer().to_bytes())));
        }

        if let Some(ref gen) = self.account_generator {
            let phrase = gen.get_phrase().to_string();
            let derivation_path = gen.get_derivation_path().to_string();

            wallet_description.insert("derivation_path".to_string(), derivation_path);
            wallet_description.insert("mnemonic".to_string(), phrase);
        };

        if let Some(fork) = fork {
            json!({
              "available_accounts": available_accounts,
              "private_keys": private_keys,
              "endpoint": fork.eth_rpc_url(),
              "block_number": fork.block_number(),
              "block_hash": fork.block_hash(),
              "chain_id": fork.chain_id(),
              "wallet": wallet_description,
              "base_fee": format!("{}", self.get_base_fee()),
              "gas_price": format!("{}", self.get_gas_price()),
              "gas_limit": format!("{}", self.gas_limit),
            })
        } else {
            json!({
              "available_accounts": available_accounts,
              "private_keys": private_keys,
              "wallet": wallet_description,
              "base_fee": format!("{}", self.get_base_fee()),
              "gas_price": format!("{}", self.get_gas_price()),
              "gas_limit": format!("{}", self.gas_limit),
              "genesis_timestamp": format!("{}", self.get_genesis_timestamp()),
            })
        }
    }
}

// === impl NodeConfig ===

impl NodeConfig {
    /// Returns a new config intended to be used in tests, which does not print and binds to a
    /// random, free port by setting it to `0`
    #[doc(hidden)]
    pub fn test() -> Self {
        Self { enable_tracing: false, silent: true, port: 0, ..Default::default() }
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        // generate some random wallets
        let genesis_accounts = AccountGenerator::new(10).phrase(DEFAULT_MNEMONIC).gen();
        Self {
            chain_id: None,
            gas_limit: U256::from(30_000_000),
            gas_price: None,
            hardfork: None,
            signer_accounts: genesis_accounts.clone(),
            genesis_timestamp: None,
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
            base_fee: None,
            enable_tracing: true,
            enable_steps_tracing: false,
            no_storage_caching: false,
            server_config: Default::default(),
            host: None,
            transaction_order: Default::default(),
            config_out: None,
            genesis: None,
            fork_request_timeout: REQUEST_TIMEOUT,
            fork_request_retries: 5,
            fork_retry_backoff: Duration::from_millis(1_000),
            // alchemy max cpus <https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups>
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
            ipc_path: None,
        }
    }
}

impl NodeConfig {
    /// Returns the base fee to use
    pub fn get_base_fee(&self) -> U256 {
        self.base_fee
            .or_else(|| self.genesis.as_ref().and_then(|g| g.base_fee_per_gas))
            .unwrap_or_else(|| INITIAL_BASE_FEE.into())
    }

    /// Returns the base fee to use
    pub fn get_gas_price(&self) -> U256 {
        self.gas_price.unwrap_or_else(|| INITIAL_GAS_PRICE.into())
    }

    /// Returns the base fee to use
    pub fn get_hardfork(&self) -> Hardfork {
        self.hardfork.unwrap_or_default()
    }

    /// Sets the chain ID
    #[must_use]
    pub fn with_chain_id<U: Into<u64>>(mut self, chain_id: Option<U>) -> Self {
        self.set_chain_id(chain_id);
        self
    }

    /// Returns the chain ID to use
    pub fn get_chain_id(&self) -> u64 {
        self.chain_id
            .or_else(|| self.genesis.as_ref().and_then(|g| g.chain_id()))
            .unwrap_or(CHAIN_ID)
    }

    /// Sets the chain id and updates all wallets
    pub fn set_chain_id(&mut self, chain_id: Option<impl Into<u64>>) {
        self.chain_id = chain_id.map(Into::into);
        let chain_id = self.get_chain_id();
        self.genesis_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(chain_id);
        });
        self.signer_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(chain_id);
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
        self.gas_price = gas_price.map(Into::into);
        self
    }

    /// Sets the base fee
    #[must_use]
    pub fn with_base_fee<U: Into<U256>>(mut self, base_fee: Option<U>) -> Self {
        self.base_fee = base_fee.map(Into::into);
        self
    }

    /// Sets the init genesis (genesis.json)
    #[must_use]
    pub fn with_genesis(mut self, genesis: Option<Genesis>) -> Self {
        self.genesis = genesis;
        self
    }

    /// Returns the genesis timestamp to use
    pub fn get_genesis_timestamp(&self) -> u64 {
        self.genesis_timestamp
            .or_else(|| self.genesis.as_ref().and_then(|g| g.timestamp))
            .unwrap_or_else(|| duration_since_unix_epoch().as_secs())
    }

    /// Sets the genesis timestamp
    #[must_use]
    pub fn with_genesis_timestamp<U: Into<u64>>(mut self, timestamp: Option<U>) -> Self {
        if let Some(timestamp) = timestamp {
            self.genesis_timestamp = Some(timestamp.into());
        }
        self
    }

    /// Sets the hardfork
    #[must_use]
    pub fn with_hardfork(mut self, hardfork: Option<Hardfork>) -> Self {
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

    /// Sets the ipc path to use
    ///
    /// Note: this is a double Option for
    ///     - `None` -> no ipc
    ///     - `Some(None)` -> use default path
    ///     - `Some(Some(path))` -> use custom path
    #[must_use]
    pub fn with_ipc(mut self, ipc_path: Option<Option<String>>) -> Self {
        self.ipc_path = ipc_path;
        self
    }

    /// Sets the file path to write the Anvil node's config info to.
    #[must_use]
    pub fn set_config_out(mut self, config_out: Option<String>) -> Self {
        self.config_out = config_out;
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

    /// Sets the `fork_request_timeout` to use for requests
    #[must_use]
    pub fn fork_request_timeout(mut self, fork_request_timeout: Option<Duration>) -> Self {
        if let Some(fork_request_timeout) = fork_request_timeout {
            self.fork_request_timeout = fork_request_timeout;
        }
        self
    }

    /// Sets the `fork_request_retries` to use for spurious networks
    #[must_use]
    pub fn fork_request_retries(mut self, fork_request_retries: Option<u32>) -> Self {
        if let Some(fork_request_retries) = fork_request_retries {
            self.fork_request_retries = fork_request_retries;
        }
        self
    }

    /// Sets the initial `fork_retry_backoff` for rate limits
    #[must_use]
    pub fn fork_retry_backoff(mut self, fork_retry_backoff: Option<Duration>) -> Self {
        if let Some(fork_retry_backoff) = fork_retry_backoff {
            self.fork_retry_backoff = fork_retry_backoff;
        }
        self
    }

    /// Sets the number of assumed available compute units per second
    ///
    /// See also, <https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups>
    #[must_use]
    pub fn fork_compute_units_per_second(mut self, compute_units_per_second: Option<u64>) -> Self {
        if let Some(compute_units_per_second) = compute_units_per_second {
            self.compute_units_per_second = compute_units_per_second;
        }
        self
    }

    /// Sets whether to enable tracing
    #[must_use]
    pub fn with_tracing(mut self, enable_tracing: bool) -> Self {
        self.enable_tracing = enable_tracing;
        self
    }

    /// Sets whether to enable steps tracing
    #[must_use]
    pub fn with_steps_tracing(mut self, enable_steps_tracing: bool) -> Self {
        self.enable_steps_tracing = enable_steps_tracing;
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

    /// Returns the ipc path for the ipc endpoint if any
    pub fn get_ipc_path(&self) -> Option<String> {
        match self.ipc_path.as_ref() {
            Some(path) => path.clone().or_else(|| {
                #[cfg(windows)]
                {
                    Some(r"\\.\pipe\anvil.ipc".to_string())
                }
                #[cfg(not(windows))]
                {
                    Some("/tmp/anvil.ipc".to_string())
                }
            }),
            None => None,
        }
    }

    /// Prints the config info
    pub fn print(&self, fork: Option<&ClientFork>) {
        if self.config_out.is_some() {
            let config_out = self.config_out.as_deref().unwrap();
            to_writer(
                &File::create(config_out).expect("Unable to create anvil config description file"),
                &self.as_json(fork),
            )
            .expect("Failed writing json");
        }
        if self.silent {
            return
        }

        println!("{}", self.as_string(fork))
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
        let chain_id = self.get_chain_id();

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
                spec_id: self.get_hardfork().into(),
                chain_id: self.get_chain_id().into(),
                limit_contract_code_size: Some(usize::MAX),
                ..Default::default()
            },
            block: BlockEnv {
                gas_limit: self.gas_limit,
                basefee: self.get_base_fee(),
                ..Default::default()
            },
            tx: TxEnv { chain_id: self.get_chain_id().into(), ..Default::default() },
        };
        let fees = FeeManager::new(env.cfg.spec_id, self.get_base_fee(), self.get_gas_price());

        let (db, fork): (Arc<tokio::sync::RwLock<dyn Db>>, Option<ClientFork>) =
            if let Some(eth_rpc_url) = self.eth_rpc_url.clone() {
                // TODO make provider agnostic
                let provider = Arc::new(
                    ProviderBuilder::new(&eth_rpc_url)
                        .timeout(self.fork_request_timeout)
                        .timeout_retry(self.fork_request_retries)
                        .initial_backoff(self.fork_retry_backoff.as_millis() as u64)
                        .compute_units_per_second(self.compute_units_per_second)
                        .max_retry(10)
                        .initial_backoff(1000)
                        .connect()
                        .await
                        .expect("Failed to establish provider to fork url"),
                );

                let fork_block_number = if let Some(fork_block_number) = self.fork_block_number {
                    // auto adjust hardfork if not specified
                    if self.hardfork.is_none() {
                        let hardfork: Hardfork = fork_block_number.into();
                        env.cfg.spec_id = hardfork.into();
                        self.hardfork = Some(hardfork);
                    }

                    fork_block_number
                } else {
                    // pick the last block number but also ensure it's not pending anymore
                    find_latest_fork_block(&provider)
                        .await
                        .expect("Failed to get fork block number")
                };

                let block = provider
                    .get_block(BlockNumber::Number(fork_block_number.into()))
                    .await
                    .expect("Failed to get fork block");

                let block = if let Some(block) = block {
                    block
                } else {
                    if let Ok(latest_block) = provider.get_block_number().await {
                        panic!(
                            "Failed to get block for block number: {}\nlatest block number: {}",
                            fork_block_number, latest_block
                        );
                    }
                    panic!("Failed to get block for block number: {}", fork_block_number)
                };

                // we only use the gas limit value of the block if it is non-zero, since there are networks where this is not used and is always `0x0` which would inevitably result in `OutOfGas` errors as soon as the evm is about to record gas, See also <https://github.com/foundry-rs/foundry/issues/3247>

                let gas_limit =
                    if block.gas_limit.is_zero() { env.block.gas_limit } else { block.gas_limit };

                env.block = BlockEnv {
                    number: fork_block_number.into(),
                    timestamp: block.timestamp,
                    difficulty: block.difficulty,
                    gas_limit,
                    // Keep previous `coinbase` and `basefee` value
                    coinbase: env.block.coinbase,
                    basefee: env.block.basefee,
                };

                // if not set explicitly we use the base fee of the latest block
                if self.base_fee.is_none() {
                    if let Some(base_fee) = block.base_fee_per_gas {
                        self.base_fee = Some(base_fee);
                        env.block.basefee = base_fee;
                        // this is the base fee of the current block, but we need the base fee of
                        // the next block
                        let next_block_base_fee = fees.get_next_block_base_fee_per_gas(
                            block.gas_used,
                            block.gas_limit,
                            block.base_fee_per_gas.unwrap_or_default(),
                        );
                        // update next base fee
                        fees.set_base_fee(next_block_base_fee.into());
                    }
                }

                // use remote gas price
                if self.gas_price.is_none() {
                    if let Ok(gas_price) = provider.get_gas_price().await {
                        self.gas_price = Some(gas_price);
                        fees.set_gas_price(gas_price);
                    }
                }

                let block_hash = block.hash.unwrap();

                let chain_id = if let Some(chain_id) = self.chain_id {
                    chain_id
                } else {
                    let chain_id = provider.get_chainid().await.unwrap().as_u64();
                    // need to update the dev signers and env with the chain id
                    self.set_chain_id(Some(chain_id));
                    env.cfg.chain_id = chain_id.into();
                    env.tx.chain_id = chain_id.into();
                    chain_id
                };
                let override_chain_id = self.chain_id;

                let meta = BlockchainDbMeta::new(env.clone(), eth_rpc_url.clone());

                let block_chain_db = BlockchainDb::new(meta, self.block_cache_path());

                // This will spawn the background thread that will use the provider to fetch
                // blockchain data from the other client
                let backend = SharedBackend::spawn_backend_thread(
                    Arc::clone(&provider),
                    block_chain_db.clone(),
                    Some(fork_block_number.into()),
                );

                let db = Arc::new(tokio::sync::RwLock::new(ForkedDatabase::new(
                    backend,
                    block_chain_db,
                )));
                let fork = ClientFork::new(
                    ClientForkConfig {
                        eth_rpc_url,
                        block_number: fork_block_number,
                        block_hash,
                        provider,
                        chain_id,
                        override_chain_id,
                        timestamp: block.timestamp.as_u64(),
                        base_fee: block.base_fee_per_gas,
                        timeout: self.fork_request_timeout,
                        retries: self.fork_request_retries,
                        backoff: self.fork_retry_backoff,
                        compute_units_per_second: self.compute_units_per_second,
                        total_difficulty: block.total_difficulty.unwrap_or_default(),
                    },
                    Arc::clone(&db),
                );

                (db, Some(fork))
            } else {
                (Arc::new(tokio::sync::RwLock::new(MemDb::default())), None)
            };

        // if provided use all settings of `genesis.json`
        if let Some(ref genesis) = self.genesis {
            genesis.apply(&mut env);
        }

        let genesis = GenesisConfig {
            timestamp: self.get_genesis_timestamp(),
            balance: self.genesis_balance,
            accounts: self.genesis_accounts.iter().map(|acc| acc.address()).collect(),
            fork_genesis_account_infos: Arc::new(Default::default()),
            genesis_init: self.genesis.clone(),
        };

        // only memory based backend for now
        mem::Backend::with_genesis(
            db,
            Arc::new(RwLock::new(env)),
            genesis,
            fees,
            fork,
            self.enable_steps_tracing,
        )
        .await
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

    fn get_phrase(&self) -> &str {
        &self.phrase
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

/// Finds the latest appropriate block to fork
///
/// This fetches the "latest" block and checks whether the `Block` is fully populated (`hash` field
/// is present). This prevents edge cases where anvil forks the "latest" block but `eth_getBlockByNumber` still returns a pending block, <https://github.com/foundry-rs/foundry/issues/2036>
async fn find_latest_fork_block<M: Middleware>(provider: M) -> Result<u64, M::Error> {
    let mut num = provider.get_block_number().await?.as_u64();

    // walk back from the head of the chain, but at most 2 blocks, which should be more than enough
    // leeway
    for _ in 0..2 {
        if let Some(block) = provider.get_block(num).await? {
            if block.hash.is_some() {
                break
            }
        }
        // block not actually finalized, so we try the block before
        num = num.saturating_sub(1)
    }

    Ok(num)
}
