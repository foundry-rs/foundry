use alloy_genesis::Genesis;
use alloy_primitives::{U256, hex, map::HashMap, utils::Unit};
use alloy_signer::Signer;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{English, Mnemonic},
};
use anvil_server::ServerConfig;
use eyre::{Context, Result};
use foundry_common::{duration_since_unix_epoch, sh_println};
use polkadot_sdk::{
    sc_cli::{
        self, CliConfiguration as SubstrateCliConfiguration, Cors, DEFAULT_WASM_EXECUTION_METHOD,
        DEFAULT_WASMTIME_INSTANTIATION_STRATEGY, RPC_DEFAULT_MAX_CONNECTIONS,
        RPC_DEFAULT_MAX_REQUEST_SIZE_MB, RPC_DEFAULT_MAX_RESPONSE_SIZE_MB,
        RPC_DEFAULT_MAX_SUBS_PER_CONN, RPC_DEFAULT_MESSAGE_CAPACITY_PER_CONN,
    },
    sc_service,
};
use rand_08::thread_rng;
use serde_json::{Value, json};
use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io,
    net::{IpAddr, Ipv4Addr},
    num::NonZeroU32,
    path::PathBuf,
    time::Duration,
};
use yansi::Paint;

pub use foundry_common::version::SHORT_VERSION as VERSION_MESSAGE;

/// Default port the rpc will open
pub const NODE_PORT: u16 = 8545;
/// Default chain id of the node
pub const CHAIN_ID: u64 = 31337;
/// The default gas limit for all transactions
pub const DEFAULT_GAS_LIMIT: u128 = 30_000_000;
/// Default mnemonic for dev accounts
pub const DEFAULT_MNEMONIC: &str = "test test test test test test test test test test test junk";

/// The default IPC endpoint
pub const DEFAULT_IPC_ENDPOINT: &str =
    if cfg!(unix) { "/tmp/anvil.ipc" } else { r"\\.\pipe\anvil.ipc" };

/// Initial base fee for EIP-1559 blocks.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Initial default gas price for the first block
pub const INITIAL_GAS_PRICE: u128 = 1_875_000_000;

const BANNER: &str = r"
                             _   _
                            (_) | |
      __ _   _ __   __   __  _  | |
     / _` | | '_ \  \ \ / / | | | |
    | (_| | | | | |  \ V /  | | | |
     \__,_| |_| |_|   \_/   |_| |_|
";

#[derive(Clone, Debug)]
pub struct SubstrateNodeConfig {
    shared_params: sc_cli::SharedParams,
    rpc_params: sc_cli::RpcParams,
    import_params: sc_cli::ImportParams,
}

impl SubstrateNodeConfig {
    pub fn new(_anvil_config: &AnvilNodeConfig) -> Self {
        let shared_params = sc_cli::SharedParams {
            chain: None,
            dev: true,
            base_path: None,
            log: vec![],
            detailed_log_output: true,
            disable_log_color: false,
            enable_log_reloading: false,
            tracing_targets: None,
            tracing_receiver: sc_cli::TracingReceiver::Log,
        };

        let rpc_params = sc_cli::RpcParams {
            rpc_external: false,
            unsafe_rpc_external: false,
            rpc_methods: sc_cli::RpcMethods::Auto,
            rpc_rate_limit: None,
            rpc_rate_limit_whitelisted_ips: vec![],
            rpc_rate_limit_trust_proxy_headers: false,
            rpc_max_request_size: RPC_DEFAULT_MAX_REQUEST_SIZE_MB,
            rpc_max_response_size: RPC_DEFAULT_MAX_RESPONSE_SIZE_MB,
            rpc_max_subscriptions_per_connection: RPC_DEFAULT_MAX_SUBS_PER_CONN,
            rpc_port: None,
            experimental_rpc_endpoint: vec![],
            rpc_max_connections: RPC_DEFAULT_MAX_CONNECTIONS,
            rpc_message_buffer_capacity_per_connection: RPC_DEFAULT_MESSAGE_CAPACITY_PER_CONN,
            rpc_disable_batch_requests: false,
            rpc_max_batch_request_len: None,
            rpc_cors: None,
        };

        // Anvil node requires these cli params configured by default except the state_pruning and
        // block_pruning params. They must be set to `DatabasePruningMode::Archive` because
        // chain reversion RPCs must revert the state db for finalized blocks, which is a no
        // operation when pruning is not configured as an archive for both blocks & state.
        let import_params = sc_cli::ImportParams {
            pruning_params: sc_cli::PruningParams {
                state_pruning: Some(sc_cli::DatabasePruningMode::Archive),
                blocks_pruning: sc_cli::DatabasePruningMode::Archive,
            },
            database_params: sc_cli::DatabaseParams { database: None, database_cache_size: None },
            wasm_method: DEFAULT_WASM_EXECUTION_METHOD,
            wasmtime_instantiation_strategy: DEFAULT_WASMTIME_INSTANTIATION_STRATEGY,
            wasm_runtime_overrides: None,
            execution_strategies: sc_cli::ExecutionStrategiesParams {
                execution_syncing: None,
                execution_import_block: None,
                execution_block_construction: None,
                execution_offchain_worker: None,
                execution_other: None,
                execution: None,
            },
            trie_cache_size: 1024 * 1024 * 1024,
            warm_up_trie_cache: None,
        };

        Self { shared_params, rpc_params, import_params }
    }

    pub fn set_base_path(&mut self, base_path: Option<PathBuf>) {
        self.shared_params.base_path = base_path;
    }
}

impl SubstrateCliConfiguration for SubstrateNodeConfig {
    fn shared_params(&self) -> &sc_cli::SharedParams {
        &self.shared_params
    }

    fn import_params(&self) -> Option<&sc_cli::ImportParams> {
        Some(&self.import_params)
    }

    fn network_params(&self) -> Option<&sc_cli::NetworkParams> {
        None
    }

    fn keystore_params(&self) -> Option<&sc_cli::KeystoreParams> {
        None
    }

    fn offchain_worker_params(&self) -> Option<&sc_cli::OffchainWorkerParams> {
        None
    }

    fn node_name(&self) -> sc_cli::Result<String> {
        Ok("anvil-substrate".to_string())
    }

    fn dev_key_seed(&self, _is_dev: bool) -> sc_cli::Result<Option<String>> {
        Ok(Some("//Alice".into()))
    }

    fn telemetry_endpoints(
        &self,
        _chain_spec: &Box<dyn sc_service::ChainSpec>,
    ) -> sc_cli::Result<Option<sc_service::config::TelemetryEndpoints>> {
        Ok(None)
    }

    fn role(&self, _is_dev: bool) -> sc_cli::Result<sc_service::Role> {
        Ok(sc_service::Role::Authority)
    }

    fn force_authoring(&self) -> sc_cli::Result<bool> {
        Ok(true)
    }

    fn prometheus_config(
        &self,
        _default_listen_port: u16,
        _chain_spec: &Box<dyn sc_service::ChainSpec>,
    ) -> sc_cli::Result<Option<sc_service::config::PrometheusConfig>> {
        Ok(None)
    }

    fn disable_grandpa(&self) -> sc_cli::Result<bool> {
        Ok(true)
    }

    fn rpc_max_connections(&self) -> sc_cli::Result<u32> {
        Ok(self.rpc_params.rpc_max_connections)
    }

    fn rpc_cors(&self, _is_dev: bool) -> sc_cli::Result<Option<Vec<String>>> {
        Ok(self.rpc_params.rpc_cors.clone().unwrap_or(Cors::All).into())
    }

    fn rpc_addr(
        &self,
        default_listen_port: u16,
    ) -> sc_cli::Result<Option<Vec<sc_cli::RpcEndpoint>>> {
        self.rpc_params.rpc_addr(true, true, default_listen_port)
    }

    fn rpc_methods(&self) -> sc_cli::Result<sc_service::RpcMethods> {
        Ok(self.rpc_params.rpc_methods.into())
    }

    fn rpc_max_request_size(&self) -> sc_cli::Result<u32> {
        Ok(self.rpc_params.rpc_max_request_size)
    }

    fn rpc_max_response_size(&self) -> sc_cli::Result<u32> {
        Ok(self.rpc_params.rpc_max_response_size)
    }

    fn rpc_max_subscriptions_per_connection(&self) -> sc_cli::Result<u32> {
        Ok(self.rpc_params.rpc_max_subscriptions_per_connection)
    }

    fn rpc_buffer_capacity_per_connection(&self) -> sc_cli::Result<u32> {
        Ok(self.rpc_params.rpc_message_buffer_capacity_per_connection)
    }

    fn rpc_batch_config(&self) -> sc_cli::Result<sc_service::config::RpcBatchRequestConfig> {
        self.rpc_params.rpc_batch_config()
    }

    fn rpc_rate_limit(&self) -> sc_cli::Result<Option<NonZeroU32>> {
        Ok(self.rpc_params.rpc_rate_limit)
    }

    fn rpc_rate_limit_whitelisted_ips(&self) -> sc_cli::Result<Vec<sc_service::config::IpNetwork>> {
        Ok(self.rpc_params.rpc_rate_limit_whitelisted_ips.clone())
    }

    fn rpc_rate_limit_trust_proxy_headers(&self) -> sc_cli::Result<bool> {
        Ok(self.rpc_params.rpc_rate_limit_trust_proxy_headers)
    }

    fn transaction_pool(
        &self,
        _is_dev: bool,
    ) -> sc_cli::Result<sc_service::TransactionPoolOptions> {
        Ok(sc_service::TransactionPoolOptions::new_with_params(
            8192,
            20480 * 1024,
            None,
            // Replace this back with TransactionPoolType::ForkAware
            // when we start using polkadot-sdk::master
            sc_cli::TransactionPoolType::SingleState.into(),
            true,
        ))
    }

    fn base_path(&self) -> sc_cli::Result<Option<sc_service::BasePath>> {
        self.shared_params().base_path()
    }
}

/// Configurations of the EVM node
#[derive(Clone, Debug)]
pub struct AnvilNodeConfig {
    /// Chain ID of the EVM chain
    pub chain_id: Option<u64>,
    /// Default gas limit for all txs
    pub gas_limit: Option<u128>,
    /// If set to `true`, disables the block gas limit
    pub disable_block_gas_limit: bool,
    /// Default gas price for all txs
    pub gas_price: Option<u128>,
    /// Default base fee
    pub base_fee: Option<u64>,
    /// If set to `true`, disables the enforcement of a minimum suggested priority fee
    pub disable_min_priority_fee: bool,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub genesis_accounts: Vec<PrivateKeySigner>,
    /// Native token balance of every genesis account in the genesis block
    pub genesis_balance: U256,
    /// Genesis block timestamp
    pub genesis_timestamp: Option<u64>,
    /// Genesis block number
    pub genesis_block_number: Option<u64>,
    /// Signer accounts that can sign messages/transactions from the EVM node
    pub signer_accounts: Vec<PrivateKeySigner>,
    /// Configured block time for the EVM chain. Use `None` to mine a new block for every tx
    pub block_time: Option<Duration>,
    /// Disable auto, interval mining mode uns use `MiningMode::None` instead
    pub no_mining: bool,
    /// Enables auto and interval mining mode
    pub mixed_mining: bool,
    /// port to use for the server
    pub port: u16,
    /// maximum number of transactions in a block
    pub max_transactions: usize,
    /// The generator used to generate the dev accounts
    pub account_generator: Option<AccountGenerator>,
    /// whether to enable tracing
    pub enable_tracing: bool,
    /// How to configure the server
    pub server_config: ServerConfig,
    /// The host the server will listen on
    pub host: Vec<IpAddr>,
    /// Filename to write anvil output as json
    pub config_out: Option<PathBuf>,
    /// The genesis to use to initialize the node
    pub genesis: Option<Genesis>,
    /// The ipc path
    pub ipc_path: Option<Option<String>>,
    /// Enable transaction/call steps tracing for debug calls returning geth-style traces
    pub enable_steps_tracing: bool,
    /// Enable printing of `console.log` invocations.
    pub print_logs: bool,
    /// Enable printing of traces.
    pub print_traces: bool,
    /// Enable auto impersonation of accounts on startup
    pub enable_auto_impersonate: bool,
    /// Configure the code size limit
    pub code_size_limit: Option<usize>,
    /// Disable the default CREATE2 deployer
    pub disable_default_create2_deployer: bool,
    /// The memory limit per EVM execution in bytes.
    pub memory_limit: Option<u64>,
    /// Do not print log messages.
    pub silent: bool,
}

impl AnvilNodeConfig {
    fn as_string(&self) -> String {
        let mut s: String = String::new();
        let _ = write!(s, "\n{}", BANNER.green());
        let _ = write!(s, "\n    {VERSION_MESSAGE}");
        let _ = write!(s, "\n    {}", "https://github.com/paritytech/foundry-polkadot".green());

        let _ = write!(
            s,
            r#"

Available Accounts
==================
"#
        );
        let balance = alloy_primitives::utils::format_ether(self.genesis_balance);
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            write!(s, "\n({idx}) {} ({balance} ETH)", wallet.address()).unwrap();
        }

        let _ = write!(
            s,
            r#"

Private Keys
==================
"#
        );

        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let hex = hex::encode(wallet.credential().to_bytes());
            let _ = write!(s, "\n({idx}) 0x{hex}");
        }

        if let Some(ref rng_gen) = self.account_generator {
            let _ = write!(
                s,
                r#"

Wallet
==================
Mnemonic:          {}
Derivation path:   {}
"#,
                rng_gen.phrase,
                rng_gen.get_derivation_path()
            );
        }

        let _ = write!(
            s,
            r#"

Chain ID
==================

{}
"#,
            self.get_chain_id().green()
        );

        let _ = write!(
            s,
            r#"
Base Fee
==================

{}
"#,
            self.get_base_fee().green()
        );

        let _ = write!(
            s,
            r#"
Gas Limit
==================

{}
"#,
            {
                if self.disable_block_gas_limit {
                    "Disabled".to_string()
                } else {
                    self.gas_limit
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| DEFAULT_GAS_LIMIT.to_string())
                }
            }
            .green()
        );

        let _ = write!(
            s,
            r#"
Genesis Timestamp
==================

{}
"#,
            self.get_genesis_timestamp().green()
        );

        let _ = write!(
            s,
            r#"
Genesis Number
==================

{}
"#,
            self.get_genesis_number().green()
        );

        s
    }

    fn as_json(&self) -> Value {
        let mut wallet_description = HashMap::new();
        let mut available_accounts = Vec::with_capacity(self.genesis_accounts.len());
        let mut private_keys = Vec::with_capacity(self.genesis_accounts.len());

        for wallet in &self.genesis_accounts {
            available_accounts.push(format!("{:?}", wallet.address()));
            private_keys.push(format!("0x{}", hex::encode(wallet.credential().to_bytes())));
        }

        if let Some(ref rng_gen) = self.account_generator {
            let phrase = rng_gen.get_phrase().to_string();
            let derivation_path = rng_gen.get_derivation_path().to_string();

            wallet_description.insert("derivation_path".to_string(), derivation_path);
            wallet_description.insert("mnemonic".to_string(), phrase);
        };

        let gas_limit = match self.gas_limit {
            // if we have a disabled flag we should max out the limit
            Some(_) | None if self.disable_block_gas_limit => Some(u64::MAX.to_string()),
            Some(limit) => Some(limit.to_string()),
            _ => None,
        };

        json!({
          "available_accounts": available_accounts,
          "private_keys": private_keys,
          "wallet": wallet_description,
          "base_fee": format!("{}", self.get_base_fee()),
          "gas_price": format!("{}", self.get_gas_price()),
          "gas_limit": gas_limit,
          "genesis_timestamp": format!("{}", self.get_genesis_timestamp()),
        })
    }

    pub fn test_config() -> Self {
        Self {
            port: 0,
            no_mining: true,
            mixed_mining: false,
            enable_tracing: false,
            silent: true,
            ..Default::default()
        }
    }
}

impl Default for AnvilNodeConfig {
    fn default() -> Self {
        // generate some random wallets
        let genesis_accounts = AccountGenerator::new(10)
            .phrase(DEFAULT_MNEMONIC)
            .rng_gen()
            .expect("Invalid mnemonic.");
        Self {
            chain_id: None,
            gas_limit: None,
            disable_block_gas_limit: false,
            gas_price: None,
            signer_accounts: genesis_accounts.clone(),
            genesis_timestamp: None,
            genesis_block_number: None,
            genesis_accounts,
            // 100ETH default balance
            genesis_balance: Unit::ETHER.wei().saturating_mul(U256::from(100u64)),
            block_time: None,
            no_mining: false,
            mixed_mining: false,
            port: NODE_PORT,
            // TODO make this something dependent on block capacity
            max_transactions: 1_000,
            account_generator: None,
            base_fee: None,
            disable_min_priority_fee: false,
            enable_tracing: true,
            enable_steps_tracing: false,
            print_logs: true,
            print_traces: false,
            enable_auto_impersonate: false,
            server_config: Default::default(),
            host: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            config_out: None,
            genesis: None,
            ipc_path: None,
            code_size_limit: None,
            disable_default_create2_deployer: false,
            memory_limit: None,
            silent: false,
        }
    }
}

impl AnvilNodeConfig {
    /// Returns the memory limit of the node
    #[must_use]
    pub fn with_memory_limit(mut self, mems_value: Option<u64>) -> Self {
        self.memory_limit = mems_value;
        self
    }
    /// Returns the base fee to use
    pub fn get_base_fee(&self) -> u64 {
        self.base_fee
            .or_else(|| self.genesis.as_ref().and_then(|g| g.base_fee_per_gas.map(|g| g as u64)))
            .unwrap_or(INITIAL_BASE_FEE)
    }

    /// Returns the base fee to use
    pub fn get_gas_price(&self) -> u128 {
        self.gas_price.unwrap_or(INITIAL_GAS_PRICE)
    }

    /// Sets a custom code size limit
    #[must_use]
    pub fn with_code_size_limit(mut self, code_size_limit: Option<usize>) -> Self {
        self.code_size_limit = code_size_limit;
        self
    }
    /// Disables  code size limit
    #[must_use]
    pub fn disable_code_size_limit(mut self, disable_code_size_limit: bool) -> Self {
        if disable_code_size_limit {
            self.code_size_limit = Some(usize::MAX);
        }
        self
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
            .or_else(|| self.genesis.as_ref().map(|g| g.config.chain_id))
            .unwrap_or(CHAIN_ID)
    }

    /// Sets the chain id and updates all wallets
    pub fn set_chain_id(&mut self, chain_id: Option<impl Into<u64>>) {
        self.chain_id = chain_id.map(Into::into);
        let chain_id = self.get_chain_id();
        self.genesis_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(Some(chain_id));
        });
        self.signer_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(Some(chain_id));
        })
    }

    /// Sets the gas limit
    #[must_use]
    pub fn with_gas_limit(mut self, gas_limit: Option<u128>) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Disable block gas limit check
    ///
    /// If set to `true` block gas limit will not be enforced
    #[must_use]
    pub fn disable_block_gas_limit(mut self, disable_block_gas_limit: bool) -> Self {
        self.disable_block_gas_limit = disable_block_gas_limit;
        self
    }

    /// Sets the gas price
    #[must_use]
    pub fn with_gas_price(mut self, gas_price: Option<u128>) -> Self {
        self.gas_price = gas_price;
        self
    }

    /// Sets the base fee
    #[must_use]
    pub fn with_base_fee(mut self, base_fee: Option<u64>) -> Self {
        self.base_fee = base_fee;
        self
    }

    /// Disable the enforcement of a minimum suggested priority fee
    #[must_use]
    pub fn disable_min_priority_fee(mut self, disable_min_priority_fee: bool) -> Self {
        self.disable_min_priority_fee = disable_min_priority_fee;
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
            .or_else(|| self.genesis.as_ref().map(|g| g.timestamp))
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

    /// Sets the genesis number
    #[must_use]
    pub fn with_genesis_block_number<U: Into<u64>>(mut self, number: Option<U>) -> Self {
        if let Some(number) = number {
            self.genesis_block_number = Some(number.into());
        }
        self
    }

    /// Returns the genesis number
    pub fn get_genesis_number(&self) -> u64 {
        self.genesis_block_number
            .or_else(|| self.genesis.as_ref().and_then(|g| g.number))
            .unwrap_or(0)
    }

    /// Sets the genesis accounts
    #[must_use]
    pub fn with_genesis_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the signer accounts
    #[must_use]
    pub fn with_signer_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.signer_accounts = accounts;
        self
    }

    /// Sets both the genesis accounts and the signer accounts
    /// so that `genesis_accounts == accounts`
    pub fn with_account_generator(mut self, generator: AccountGenerator) -> eyre::Result<Self> {
        let accounts = generator.rng_gen()?;
        self.account_generator = Some(generator);
        Ok(self.with_signer_accounts(accounts.clone()).with_genesis_accounts(accounts))
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

    #[must_use]
    pub fn with_mixed_mining<D: Into<Duration>>(
        mut self,
        mixed_mining: bool,
        block_time: Option<D>,
    ) -> Self {
        self.block_time = block_time.map(Into::into);
        self.mixed_mining = mixed_mining;
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
    pub fn set_config_out(mut self, config_out: Option<PathBuf>) -> Self {
        self.config_out = config_out;
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

    /// Sets whether to print `console.log` invocations to stdout.
    #[must_use]
    pub fn with_print_logs(mut self, print_logs: bool) -> Self {
        self.print_logs = print_logs;
        self
    }

    /// Sets whether to print traces to stdout.
    #[must_use]
    pub fn with_print_traces(mut self, print_traces: bool) -> Self {
        self.print_traces = print_traces;
        self
    }

    /// Sets whether to enable autoImpersonate
    #[must_use]
    pub fn with_auto_impersonate(mut self, enable_auto_impersonate: bool) -> Self {
        self.enable_auto_impersonate = enable_auto_impersonate;
        self
    }

    #[must_use]
    pub fn with_server_config(mut self, config: ServerConfig) -> Self {
        self.server_config = config;
        self
    }

    /// Sets the host the server will listen on
    #[must_use]
    pub fn with_host(mut self, host: Vec<IpAddr>) -> Self {
        self.host = if host.is_empty() { vec![IpAddr::V4(Ipv4Addr::LOCALHOST)] } else { host };
        self
    }

    /// Returns the ipc path for the ipc endpoint if any
    pub fn get_ipc_path(&self) -> Option<String> {
        match &self.ipc_path {
            Some(path) => path.clone().or_else(|| Some(DEFAULT_IPC_ENDPOINT.to_string())),
            None => None,
        }
    }

    /// Prints the config info
    pub fn print(&self) -> Result<()> {
        if let Some(path) = &self.config_out {
            let file = io::BufWriter::new(
                File::create(path).wrap_err("unable to create anvil config description file")?,
            );
            let value = self.as_json();
            serde_json::to_writer(file, &value).wrap_err("failed writing JSON")?;
        }
        if !self.silent {
            sh_println!("{}", self.as_string())?;
        }
        Ok(())
    }

    /// Sets whether to disable the default create2 deployer
    #[must_use]
    pub fn with_disable_default_create2_deployer(mut self, yes: bool) -> Self {
        self.disable_default_create2_deployer = yes;
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
}

/// Can create dev accounts
#[derive(Clone, Debug)]
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
            phrase: Mnemonic::<English>::new(&mut thread_rng()).to_phrase(),
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
    pub fn rng_gen(&self) -> eyre::Result<Vec<PrivateKeySigner>> {
        let builder = MnemonicBuilder::<English>::default().phrase(self.phrase.as_str());

        // use the derivation path
        let derivation_path = self.get_derivation_path();

        let mut wallets = Vec::with_capacity(self.amount);
        for idx in 0..self.amount {
            let builder =
                builder.clone().derivation_path(format!("{derivation_path}{idx}")).unwrap();
            let wallet = builder.build()?.with_chain_id(Some(self.chain_id));
            wallets.push(wallet)
        }
        Ok(wallets)
    }
}
