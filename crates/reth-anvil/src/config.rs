use crate::{
    state_dump::SerializableState,
    types::{ForkUrl, TransactionOrder},
};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, Bytes, U256, hex, map::HashMap, utils::Unit};
use alloy_signer::Signer;
use alloy_signer_local::{MnemonicBuilder, PrivateKeySigner, coins_bip39::English};
use eyre::{Result, WrapErr};
use foundry_evm_core::constants::{
    DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE,
};
use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
use rand_08::thread_rng;
use reth_ethereum::chainspec::{Chain, ChainSpec, ChainSpecBuilder, ForkCondition};
use serde_json::{Value, json};
use std::{
    fmt::Write,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use yansi::Paint;

const BANNER: &str = r"
                             _   _
                            (_) | |
      __ _   _ __   __   __  _  | |
     / _` | | '_ \  \ \ / / | | | |
    | (_| | | | | |  \ V /  | | | |
     \__,_| |_| |_|   \_/   |_| |_|
";

/// Default port the RPC server listens on.
pub const NODE_PORT: u16 = 8545;

/// Default chain id of the dev chain.
pub const CHAIN_ID: u64 = 31337;

/// Default block gas limit.
pub const DEFAULT_GAS_LIMIT: u64 = 30_000_000;

/// Default base fee of the genesis block, in wei.
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000;

/// Default number of slots in an epoch, which sets the distance of the `safe` and `finalized` tags
/// from the head.
pub const DEFAULT_SLOTS_IN_AN_EPOCH: u64 = 32;

/// Default mnemonic of the dev accounts.
pub const DEFAULT_MNEMONIC: &str = "test test test test test test test test test test test junk";

/// Default IPC endpoint.
#[cfg(windows)]
pub const DEFAULT_IPC_ENDPOINT: &str = r"\\.\pipe\anvil.ipc";

/// Default IPC endpoint.
#[cfg(not(windows))]
pub const DEFAULT_IPC_ENDPOINT: &str = "/tmp/anvil.ipc";

/// Configuration of a reth-anvil node.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// Chain id of the node.
    pub chain_id: Option<u64>,
    /// Block gas limit.
    pub gas_limit: Option<u64>,
    /// Hardfork active from genesis.
    pub hardfork: Option<FoundryHardfork>,
    /// Accounts funded in genesis.
    pub genesis_accounts: Vec<PrivateKeySigner>,
    /// Accounts the node signs for.
    pub signer_accounts: Vec<PrivateKeySigner>,
    /// Timestamp of the genesis block.
    pub genesis_timestamp: Option<u64>,
    /// Balance of every genesis account.
    pub genesis_balance: U256,
    /// Custom genesis to start from.
    pub genesis: Option<Genesis>,
    /// Base fee of the genesis block.
    pub base_fee: Option<u64>,
    /// Interval between blocks. `None` mines a block per transaction.
    pub block_time: Option<Duration>,
    /// Mine only on request.
    pub no_mining: bool,
    /// Mine both per transaction and at the interval.
    pub mixed_mining: bool,
    /// Port of the RPC server. Zero picks a free port.
    pub port: u16,
    /// Hosts the RPC server binds to.
    pub host: Vec<IpAddr>,
    /// Impersonate every account.
    pub enable_auto_impersonate: bool,
    /// Skip the default create2 deployer in genesis.
    pub disable_default_create2_deployer: bool,
    /// Number of slots in an epoch.
    pub slots_in_an_epoch: u64,
    /// Print nothing on startup.
    pub silent: bool,
    /// Accounts and balances to fund in genesis on top of the dev accounts.
    pub funded_accounts: HashMap<Address, U256>,
    /// The generator the dev accounts came from, for the startup banner.
    pub account_generator: Option<AccountGenerator>,
    /// Number of the genesis block.
    pub genesis_block_number: Option<u64>,
    /// Gas price for pre-London chains.
    pub gas_price: Option<u128>,
    /// Disable the block gas limit.
    pub disable_block_gas_limit: bool,
    /// Enforce the transaction gas limit cap.
    pub enable_tx_gas_limit: bool,
    /// Contract code size limit.
    pub code_size_limit: Option<usize>,
    /// Disable the minimum priority fee.
    pub disable_min_priority_fee: bool,
    /// EVM memory limit.
    pub memory_limit: Option<u64>,
    /// How the pool orders transactions.
    pub transaction_order: TransactionOrder,
    /// IPC endpoint, if enabled.
    pub ipc_path: Option<String>,
    /// File to write the node config to as JSON.
    pub config_out: Option<PathBuf>,
    /// State to load at startup.
    pub init_state: Option<SerializableState>,
    /// Fork endpoints.
    pub fork_urls: Vec<ForkUrl>,
    /// Print opcode traces.
    pub enable_steps_tracing: bool,
    /// Print `console.log` output.
    pub print_logs: bool,
    /// Print transaction traces.
    pub print_traces: bool,
    /// Number of blocks to keep in history.
    pub prune_history: Option<Option<usize>>,
    /// Maximum number of states to persist on disk.
    pub max_persisted_states: Option<usize>,
    /// Number of blocks for which to keep transactions.
    pub transaction_block_keeper: Option<usize>,
    /// Maximum number of transactions per block.
    pub max_transactions: usize,
    /// Disable pool balance checks.
    pub disable_pool_balance_checks: bool,
    /// Path of the block cache.
    pub cache_path: Option<PathBuf>,
    /// CORS `allow_origin` header.
    pub allow_origin: String,
    /// Disable CORS.
    pub no_cors: bool,
    /// Disable the request body size limit.
    pub no_request_size_limit: bool,
}

impl Default for NodeConfig {
    fn default() -> Self {
        let genesis_accounts = AccountGenerator::new(10).generate().expect("valid mnemonic");
        Self {
            chain_id: None,
            gas_limit: None,
            hardfork: None,
            signer_accounts: genesis_accounts.clone(),
            genesis_accounts,
            genesis_timestamp: None,
            genesis_balance: Unit::ETHER.wei().saturating_mul(U256::from(100u64)),
            genesis: None,
            base_fee: None,
            block_time: None,
            no_mining: false,
            mixed_mining: false,
            port: NODE_PORT,
            host: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            enable_auto_impersonate: false,
            disable_default_create2_deployer: false,
            slots_in_an_epoch: DEFAULT_SLOTS_IN_AN_EPOCH,
            silent: false,
            funded_accounts: HashMap::default(),
            account_generator: None,
            genesis_block_number: None,
            gas_price: None,
            disable_block_gas_limit: false,
            enable_tx_gas_limit: false,
            code_size_limit: None,
            disable_min_priority_fee: false,
            memory_limit: None,
            transaction_order: TransactionOrder::default(),
            ipc_path: None,
            config_out: None,
            init_state: None,
            fork_urls: Vec::new(),
            enable_steps_tracing: false,
            print_logs: true,
            print_traces: false,
            prune_history: None,
            max_persisted_states: None,
            transaction_block_keeper: None,
            max_transactions: 1_000,
            disable_pool_balance_checks: false,
            cache_path: None,
            allow_origin: "*".to_string(),
            no_cors: false,
            no_request_size_limit: false,
        }
    }
}

impl NodeConfig {
    /// Returns a config for tests: silent, on a free port.
    #[doc(hidden)]
    pub fn test() -> Self {
        Self { port: 0, silent: true, ..Default::default() }
    }

    /// Sets the chain id.
    pub fn with_chain_id<U: Into<u64>>(mut self, chain_id: Option<U>) -> Self {
        self.chain_id = chain_id.map(Into::into);
        self
    }

    /// Returns the chain id.
    pub fn get_chain_id(&self) -> u64 {
        self.chain_id
            .or_else(|| self.genesis.as_ref().map(|genesis| genesis.config.chain_id))
            .unwrap_or(CHAIN_ID)
    }

    /// Sets the block gas limit.
    pub const fn with_gas_limit(mut self, gas_limit: Option<u64>) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Returns the block gas limit.
    pub fn get_gas_limit(&self) -> u64 {
        self.gas_limit
            .or_else(|| self.genesis.as_ref().map(|genesis| genesis.gas_limit))
            .unwrap_or(DEFAULT_GAS_LIMIT)
    }

    /// Sets the base fee of the genesis block.
    pub const fn with_base_fee(mut self, base_fee: Option<u64>) -> Self {
        self.base_fee = base_fee;
        self
    }

    /// Returns the base fee of the genesis block.
    pub fn get_base_fee(&self) -> u64 {
        self.base_fee
            .or_else(|| {
                self.genesis
                    .as_ref()
                    .and_then(|genesis| genesis.base_fee_per_gas.map(|fee| fee as u64))
            })
            .unwrap_or(INITIAL_BASE_FEE)
    }

    /// Sets the hardfork active from genesis.
    pub const fn with_hardfork(mut self, hardfork: Option<FoundryHardfork>) -> Self {
        self.hardfork = hardfork;
        self
    }

    /// Returns the Ethereum hardfork active from genesis.
    pub fn get_hardfork(&self) -> Result<EthereumHardfork> {
        match self.hardfork {
            None => Ok(EthereumHardfork::from_chain_and_timestamp(
                Chain::mainnet(),
                self.get_genesis_timestamp(),
            )
            .unwrap_or(EthereumHardfork::Osaka)),
            Some(FoundryHardfork::Ethereum(hardfork)) => Ok(hardfork),
            Some(hardfork) => eyre::bail!("hardfork {hardfork:?} is not supported yet"),
        }
    }

    /// Sets a custom genesis.
    pub fn with_genesis(mut self, genesis: Option<Genesis>) -> Self {
        self.genesis = genesis;
        self
    }

    /// Sets the genesis timestamp.
    pub fn with_genesis_timestamp<U: Into<u64>>(mut self, timestamp: Option<U>) -> Self {
        if let Some(timestamp) = timestamp {
            self.genesis_timestamp = Some(timestamp.into());
        }
        self
    }

    /// Returns the genesis timestamp.
    pub fn get_genesis_timestamp(&self) -> u64 {
        self.genesis_timestamp
            .or_else(|| self.genesis.as_ref().map(|genesis| genesis.timestamp))
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("current time is after the unix epoch")
                    .as_secs()
            })
    }

    /// Sets the accounts funded in genesis.
    pub fn with_genesis_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the accounts the node signs for.
    pub fn with_signer_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.signer_accounts = accounts;
        self
    }

    /// Generates the genesis and signer accounts with the given generator.
    pub fn with_account_generator(mut self, generator: AccountGenerator) -> Result<Self> {
        let accounts = generator.generate()?;
        self.genesis_accounts = accounts.clone();
        self.signer_accounts = accounts;
        self.account_generator = Some(generator);
        Ok(self)
    }

    /// Sets the genesis block number.
    pub fn with_genesis_block_number<U: Into<u64>>(mut self, number: Option<U>) -> Self {
        if let Some(number) = number {
            self.genesis_block_number = Some(number.into());
        }
        self
    }

    /// Returns the genesis block number.
    pub fn get_genesis_number(&self) -> u64 {
        self.genesis_block_number
            .or_else(|| self.genesis.as_ref().and_then(|genesis| genesis.number))
            .unwrap_or(0)
    }

    /// Sets the gas price for pre-London chains.
    pub const fn with_gas_price(mut self, gas_price: Option<u128>) -> Self {
        self.gas_price = gas_price;
        self
    }

    /// Returns the gas price for pre-London chains.
    pub fn get_gas_price(&self) -> u128 {
        self.gas_price.unwrap_or(INITIAL_BASE_FEE as u128)
    }

    /// Disables the block gas limit.
    pub const fn disable_block_gas_limit(mut self, disable: bool) -> Self {
        self.disable_block_gas_limit = disable;
        self
    }

    /// Enforces the transaction gas limit cap.
    pub const fn enable_tx_gas_limit(mut self, enable: bool) -> Self {
        self.enable_tx_gas_limit = enable;
        self
    }

    /// Sets the contract code size limit.
    pub const fn with_code_size_limit(mut self, code_size_limit: Option<usize>) -> Self {
        self.code_size_limit = code_size_limit;
        self
    }

    /// Disables the contract code size limit.
    pub const fn disable_code_size_limit(mut self, disable: bool) -> Self {
        if disable {
            self.code_size_limit = Some(usize::MAX);
        }
        self
    }

    /// Disables the minimum priority fee.
    pub const fn disable_min_priority_fee(mut self, disable: bool) -> Self {
        self.disable_min_priority_fee = disable;
        self
    }

    /// Sets the EVM memory limit.
    pub const fn with_memory_limit(mut self, memory_limit: Option<u64>) -> Self {
        self.memory_limit = memory_limit;
        self
    }

    /// Sets how the pool orders transactions.
    pub const fn with_transaction_order(mut self, order: TransactionOrder) -> Self {
        self.transaction_order = order;
        self
    }

    /// Enables the IPC endpoint at the given path, or the default path.
    pub fn with_ipc(mut self, ipc_path: Option<Option<String>>) -> Self {
        self.ipc_path =
            ipc_path.map(|path| path.unwrap_or_else(|| DEFAULT_IPC_ENDPOINT.to_string()));
        self
    }

    /// Returns the IPC endpoint, if enabled.
    pub fn get_ipc_path(&self) -> Option<String> {
        self.ipc_path.clone()
    }

    /// Sets the file to write the node config to.
    pub fn set_config_out(mut self, config_out: Option<PathBuf>) -> Self {
        self.config_out = config_out;
        self
    }

    /// Sets whether to print nothing on startup.
    pub const fn set_silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    /// Sets the state to load at startup.
    pub fn with_init_state(mut self, init_state: Option<SerializableState>) -> Self {
        self.init_state = init_state;
        self
    }

    /// Sets the fork endpoints.
    pub fn with_fork_urls(mut self, fork_urls: Vec<ForkUrl>) -> Self {
        self.fork_urls = fork_urls;
        self
    }

    /// Sets the fork endpoint.
    pub fn with_eth_rpc_url<U: Into<String>>(mut self, eth_rpc_url: Option<U>) -> Self {
        self.fork_urls = eth_rpc_url
            .map(|url| vec![ForkUrl { url: url.into(), block: None }])
            .unwrap_or_default();
        self
    }

    /// Enables opcode tracing output.
    pub const fn with_steps_tracing(mut self, enable: bool) -> Self {
        self.enable_steps_tracing = enable;
        self
    }

    /// Enables `console.log` output.
    pub const fn with_print_logs(mut self, print_logs: bool) -> Self {
        self.print_logs = print_logs;
        self
    }

    /// Enables transaction trace output.
    pub const fn with_print_traces(mut self, print_traces: bool) -> Self {
        self.print_traces = print_traces;
        self
    }

    /// Sets the number of blocks to keep in history.
    pub const fn set_pruned_history(mut self, prune_history: Option<Option<usize>>) -> Self {
        self.prune_history = prune_history;
        self
    }

    /// Sets the maximum number of states to persist on disk.
    pub const fn with_max_persisted_states(mut self, max: Option<usize>) -> Self {
        self.max_persisted_states = max;
        self
    }

    /// Sets the number of blocks for which to keep transactions.
    pub const fn with_transaction_block_keeper(mut self, keeper: Option<usize>) -> Self {
        self.transaction_block_keeper = keeper;
        self
    }

    /// Sets the maximum number of transactions per block.
    pub const fn with_max_transactions(mut self, max: Option<usize>) -> Self {
        if let Some(max) = max {
            self.max_transactions = max;
        }
        self
    }

    /// Disables pool balance checks.
    pub const fn with_disable_pool_balance_checks(mut self, yes: bool) -> Self {
        self.disable_pool_balance_checks = yes;
        self
    }

    /// Sets the block cache path.
    pub fn with_cache_path(mut self, cache_path: Option<PathBuf>) -> Self {
        self.cache_path = cache_path;
        self
    }

    /// Sets the server options.
    pub fn with_server_config(
        mut self,
        allow_origin: String,
        no_cors: bool,
        no_request_size_limit: bool,
    ) -> Self {
        self.allow_origin = allow_origin;
        self.no_cors = no_cors;
        self.no_request_size_limit = no_request_size_limit;
        self
    }

    /// Prints the startup banner and writes the config file, if configured.
    pub fn print(&self) -> Result<()> {
        if let Some(path) = &self.config_out {
            foundry_common::fs::write_sensitive_json_file(path, &self.as_json())
                .wrap_err("failed writing JSON")?;
        }
        if !self.silent {
            foundry_common::sh_println!("{}", self.as_string())?;
        }
        Ok(())
    }

    /// Returns the startup banner.
    fn as_string(&self) -> String {
        let mut s = String::new();
        let _ = write!(s, "\n{}", BANNER.green());
        let _ = write!(s, "\n    {}", foundry_common::version::SHORT_VERSION);
        let _ = write!(s, "\n    {}", "https://github.com/foundry-rs/foundry".green());
        let _ = write!(s, "\n\nAvailable Accounts\n==================\n");
        let balance = alloy_primitives::utils::format_ether(self.genesis_balance);
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let _ = write!(s, "\n({idx}) {} ({balance} ETH)", wallet.address());
        }
        let _ = write!(s, "\n\nPrivate Keys\n==================\n");
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let _ = write!(s, "\n({idx}) {}", hex::encode_prefixed(wallet.credential().to_bytes()));
        }
        if let Some(generator) = &self.account_generator {
            let _ = write!(
                s,
                "\n\nWallet\n==================\nMnemonic:          {}\nDerivation path:   {}\n",
                generator.get_phrase(),
                generator.get_derivation_path()
            );
        }
        let _ = write!(s, "\n\nChain ID\n==================\n{}\n", self.get_chain_id().green());
        if self.get_hardfork().is_ok_and(|hardfork| hardfork < EthereumHardfork::London) {
            let _ =
                write!(s, "\nGas Price\n==================\n{}\n", self.get_gas_price().green());
        } else {
            let _ = write!(s, "\nBase Fee\n==================\n{}\n", self.get_base_fee().green());
        }
        let gas_limit = if self.disable_block_gas_limit {
            "Disabled".to_string()
        } else {
            self.get_gas_limit().to_string()
        };
        let _ = write!(s, "\nGas Limit\n==================\n{}\n", gas_limit.green());
        let _ = write!(
            s,
            "\nGenesis Timestamp\n==================\n{}\n",
            self.get_genesis_timestamp().green()
        );
        let _ = write!(
            s,
            "\nGenesis Number\n==================\n{}\n",
            self.get_genesis_number().green()
        );
        s
    }

    /// Returns the config as JSON, for `--config-out`.
    fn as_json(&self) -> Value {
        let available_accounts: Vec<_> =
            self.genesis_accounts.iter().map(|wallet| format!("{:?}", wallet.address())).collect();
        let private_keys: Vec<_> = self
            .genesis_accounts
            .iter()
            .map(|wallet| hex::encode_prefixed(wallet.credential().to_bytes()))
            .collect();
        let mut wallet_description = HashMap::new();
        if let Some(generator) = &self.account_generator {
            wallet_description
                .insert("derivation_path".to_string(), generator.get_derivation_path().to_string());
            wallet_description.insert("mnemonic".to_string(), generator.get_phrase().to_string());
        }
        let gas_limit = match self.gas_limit {
            Some(_) | None if self.disable_block_gas_limit => Some(u64::MAX.to_string()),
            Some(limit) => Some(limit.to_string()),
            None => None,
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

    /// Sets the balance of every genesis account.
    pub fn with_genesis_balance<U: Into<U256>>(mut self, balance: U) -> Self {
        self.genesis_balance = balance.into();
        self
    }

    /// Sets the interval between blocks.
    pub fn with_blocktime<D: Into<Duration>>(mut self, block_time: Option<D>) -> Self {
        self.block_time = block_time.map(Into::into);
        self
    }

    /// Sets mixed mining: a block per transaction and at the interval.
    pub fn with_mixed_mining<D: Into<Duration>>(
        mut self,
        mixed_mining: bool,
        block_time: Option<D>,
    ) -> Self {
        self.block_time = block_time.map(Into::into);
        self.mixed_mining = mixed_mining;
        self
    }

    /// Disables automatic mining.
    pub const fn with_no_mining(mut self, no_mining: bool) -> Self {
        self.no_mining = no_mining;
        self
    }

    /// Sets the number of slots in an epoch.
    pub const fn with_slots_in_an_epoch(mut self, slots_in_an_epoch: u64) -> Self {
        self.slots_in_an_epoch = slots_in_an_epoch;
        self
    }

    /// Sets the RPC port.
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the RPC hosts.
    pub fn with_host(mut self, host: Vec<IpAddr>) -> Self {
        self.host = if host.is_empty() { vec![IpAddr::V4(Ipv4Addr::LOCALHOST)] } else { host };
        self
    }

    /// Enables impersonation of every account.
    pub const fn with_auto_impersonate(mut self, enable_auto_impersonate: bool) -> Self {
        self.enable_auto_impersonate = enable_auto_impersonate;
        self
    }

    /// Skips the default create2 deployer in genesis.
    pub const fn with_disable_default_create2_deployer(mut self, yes: bool) -> Self {
        self.disable_default_create2_deployer = yes;
        self
    }

    /// Prints nothing on startup.
    pub const fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Funds the given accounts in genesis.
    pub fn with_funded_accounts(mut self, accounts: HashMap<Address, U256>) -> Self {
        self.funded_accounts = accounts;
        self
    }

    /// Builds the chain spec: the configured hardfork active from genesis, with the dev accounts
    /// and the create2 deployer in the genesis allocation.
    pub fn chain_spec(&self) -> Result<Arc<ChainSpec>> {
        let hardfork = self.get_hardfork()?;
        let mut genesis = self
            .genesis
            .clone()
            .unwrap_or_default()
            .with_timestamp(self.get_genesis_timestamp())
            .with_gas_limit(self.get_gas_limit())
            .with_difficulty(U256::ZERO);
        genesis.config.chain_id = self.get_chain_id();
        if hardfork >= EthereumHardfork::London {
            genesis = genesis.with_base_fee(Some(self.get_base_fee().into()));
        }
        if hardfork >= EthereumHardfork::Cancun {
            let excess_blob_gas = genesis.excess_blob_gas.unwrap_or_default();
            let blob_gas_used = genesis.blob_gas_used.unwrap_or_default();
            genesis = genesis
                .with_excess_blob_gas(Some(excess_blob_gas))
                .with_blob_gas_used(Some(blob_gas_used));
        }

        let mut alloc: Vec<(Address, GenesisAccount)> = self
            .genesis_accounts
            .iter()
            .map(|account| {
                (account.address(), GenesisAccount::default().with_balance(self.genesis_balance))
            })
            .collect();
        alloc.extend(self.funded_accounts.iter().map(|(address, balance)| {
            (*address, GenesisAccount::default().with_balance(*balance))
        }));
        if !self.disable_default_create2_deployer {
            alloc.push((
                DEFAULT_CREATE2_DEPLOYER,
                GenesisAccount::default()
                    .with_code(Some(Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE))),
            ));
        }
        genesis = genesis.extend_accounts(alloc);

        let builder =
            ChainSpecBuilder::default().chain(Chain::from_id(self.get_chain_id())).genesis(genesis);
        Ok(Arc::new(activate_hardfork(builder, hardfork).build()))
    }
}

/// Activates every hardfork up to and including `hardfork` at genesis.
fn activate_hardfork(builder: ChainSpecBuilder, hardfork: EthereumHardfork) -> ChainSpecBuilder {
    match hardfork {
        EthereumHardfork::Frontier => builder.frontier_activated(),
        EthereumHardfork::Homestead => builder.homestead_activated(),
        EthereumHardfork::Dao => builder.dao_activated(),
        EthereumHardfork::Tangerine => builder.tangerine_whistle_activated(),
        EthereumHardfork::SpuriousDragon => builder.spurious_dragon_activated(),
        EthereumHardfork::Byzantium => builder.byzantium_activated(),
        EthereumHardfork::Constantinople => builder.constantinople_activated(),
        EthereumHardfork::Petersburg => builder.petersburg_activated(),
        EthereumHardfork::Istanbul => builder.istanbul_activated(),
        EthereumHardfork::MuirGlacier => builder.muirglacier_activated(),
        EthereumHardfork::Berlin => builder.berlin_activated(),
        EthereumHardfork::London => builder.london_activated(),
        EthereumHardfork::ArrowGlacier => builder.arrowglacier_activated(),
        EthereumHardfork::GrayGlacier => builder.grayglacier_activated(),
        EthereumHardfork::Paris => builder.paris_activated(),
        EthereumHardfork::Shanghai => builder.shanghai_activated(),
        EthereumHardfork::Cancun => builder.cancun_activated(),
        EthereumHardfork::Prague => builder.prague_activated(),
        EthereumHardfork::Osaka => builder.osaka_activated(),
        EthereumHardfork::Amsterdam => builder.amsterdam_activated(),
        EthereumHardfork::Bogota => builder.bogota_activated(),
        bpo @ (EthereumHardfork::Bpo1
        | EthereumHardfork::Bpo2
        | EthereumHardfork::Bpo3
        | EthereumHardfork::Bpo4
        | EthereumHardfork::Bpo5) => EthereumHardfork::bpo_variants()
            .iter()
            .take_while(|variant| **variant <= bpo)
            .fold(builder.osaka_activated(), |builder, variant| {
                builder.with_fork(*variant, ForkCondition::Timestamp(0))
            }),
        // The hardfork enum is non-exhaustive. A fork newer than the ones listed here gets the
        // latest known schedule.
        _ => builder.bogota_activated(),
    }
}

/// Derives dev accounts from a mnemonic.
#[derive(Clone, Debug)]
pub struct AccountGenerator {
    chain_id: u64,
    amount: usize,
    phrase: String,
    derivation_path: Option<String>,
}

impl AccountGenerator {
    /// Creates a generator for `amount` accounts from a random mnemonic.
    pub fn new(amount: usize) -> Self {
        Self {
            chain_id: CHAIN_ID,
            amount,
            phrase: alloy_signer_local::coins_bip39::Mnemonic::<English>::new(&mut thread_rng())
                .to_phrase(),
            derivation_path: None,
        }
    }

    /// Sets the mnemonic.
    pub fn phrase(mut self, phrase: impl Into<String>) -> Self {
        self.phrase = phrase.into();
        self
    }

    /// Returns the mnemonic.
    pub fn get_phrase(&self) -> &str {
        &self.phrase
    }

    /// Sets the chain id the accounts sign for.
    pub fn chain_id(mut self, chain_id: impl Into<u64>) -> Self {
        self.chain_id = chain_id.into();
        self
    }

    /// Sets the derivation path.
    pub fn derivation_path(mut self, derivation_path: impl Into<String>) -> Self {
        let mut derivation_path = derivation_path.into();
        if !derivation_path.ends_with('/') {
            derivation_path.push('/');
        }
        self.derivation_path = Some(derivation_path);
        self
    }

    /// Returns the derivation path.
    pub fn get_derivation_path(&self) -> &str {
        self.derivation_path.as_deref().unwrap_or("m/44'/60'/0'/0/")
    }

    /// Derives the accounts.
    pub fn generate(&self) -> Result<Vec<PrivateKeySigner>> {
        let builder = MnemonicBuilder::<English>::default().phrase(self.phrase.as_str());
        let derivation_path = self.get_derivation_path();
        foundry_common::wallet::validate_bip32_path(derivation_path).map_err(|e| eyre::eyre!(e))?;

        let mut wallets = Vec::with_capacity(self.amount);
        for idx in 0..self.amount {
            let idx = u32::try_from(idx).map_err(|_| eyre::eyre!("account index overflows u32"))?;
            let full_path = foundry_common::wallet::derive_key_path_checked(derivation_path, idx)
                .map_err(|e| eyre::eyre!(e))?;
            let wallet = builder
                .clone()
                .derivation_path(full_path)?
                .build()?
                .with_chain_id(Some(self.chain_id));
            wallets.push(wallet);
        }
        Ok(wallets)
    }
}
