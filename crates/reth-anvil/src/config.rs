use crate::{
    fork::{ForkGenesisAccount, ForkSettings, NodeInfoProbe},
    state_dump::{CheckpointForks, SerializableState},
    types::{ForkChoice, ForkUrl, TransactionOrder},
};
use alloy_eips::{
    eip2935, eip4788, eip7002, eip7251, eip7840::BlobParams, eip7892::BlobScheduleBlobParams,
};
use alloy_genesis::{Genesis, GenesisAccount};
use alloy_primitives::{Address, B256, Bytes, U256, hex, map::HashMap, utils::Unit};
use alloy_rpc_types::anvil::NodeInfo;
use alloy_signer::Signer;
use alloy_signer_local::{MnemonicBuilder, PrivateKeySigner, coins_bip39::English};
use eyre::{Result, WrapErr};
use foundry_common::{ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT};
use foundry_evm_core::{
    constants::{DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE},
    utils::get_blob_params_by_hardfork,
};
use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
use parking_lot::RwLock;
use rand_08::thread_rng;
use reth_ethereum::{
    chainspec::{Chain, ChainSpec, ChainSpecBuilder, ForkCondition},
    primitives::SealedHeader,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fmt::Write,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use yansi::Paint;

#[cfg(feature = "monad")]
use foundry_evm_hardforks::MonadHardfork;
#[cfg(feature = "optimism")]
use reth_ethereum::chainspec::NamedChain;
#[cfg(feature = "monad")]
use revm::primitives::hardfork::SpecId;

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
    /// Where to fork from. Overrides the block in `fork_urls`.
    pub fork_choice: Option<ForkChoice>,
    /// Extra HTTP headers for the fork endpoint.
    pub fork_headers: Vec<String>,
    /// Timeout of fork requests.
    pub fork_request_timeout: Duration,
    /// Number of retries of fork requests.
    pub fork_request_retries: u32,
    /// Initial backoff of fork request retries.
    pub fork_retry_backoff: Duration,
    /// Assumed compute units per second of the fork endpoint.
    pub compute_units_per_second: u64,
    /// Chain id of the fork endpoint, to skip the `eth_chainId` request.
    pub fork_chain_id: Option<u64>,
    /// Skip the on-disk fork cache.
    pub no_storage_caching: bool,
    /// Fetch fork state by block number instead of block hash.
    pub fork_state_by_number: bool,
    /// Skip the node info request to the fork endpoint.
    pub no_fork_node_info: bool,
    /// Skip block access list prefetching when forking.
    pub no_bal: bool,
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
    /// The network the node runs.
    pub networks: NetworkConfigs,
    /// Whether a fork fixed `networks`. Resets to another endpoint keep it, as on anvil.
    pub adopted_fork_network: bool,
    /// Whether a fork endpoint chose the chain id, so a reset to another endpoint adopts its
    /// chain id again.
    pub adopted_chain_id: bool,
    /// Whether a fork endpoint chose the hardfork, so a reset to another endpoint adopts its
    /// hardfork again.
    pub adopted_hardfork: bool,
    /// Whether a fork endpoint chose the base fee, so the next block keeps the fork block's fee
    /// schedule instead of an explicit base fee.
    pub adopted_base_fee: bool,
    /// Whether the user selected the network: such a node may fork and reset to an endpoint of
    /// another network family, as on anvil, where an inferred network may not.
    pub explicit_network: bool,
    /// The fork endpoints that answered `anvil_nodeInfo` once, after which a failing probe is an
    /// error instead of an unsupported method. Shared by every config of a node, so resets and
    /// relaunches keep it, as anvil keeps an endpoint's identity.
    pub anvil_endpoints: Arc<RwLock<HashSet<String>>>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        let genesis_accounts =
            AccountGenerator::new(10).phrase(DEFAULT_MNEMONIC).generate().expect("valid mnemonic");
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
            fork_choice: None,
            fork_headers: Vec::new(),
            fork_request_timeout: REQUEST_TIMEOUT,
            fork_request_retries: 5,
            fork_retry_backoff: Duration::from_millis(1_000),
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
            fork_chain_id: None,
            no_storage_caching: false,
            fork_state_by_number: false,
            no_fork_node_info: false,
            no_bal: false,
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
            networks: NetworkConfigs::default(),
            adopted_fork_network: false,
            adopted_chain_id: false,
            adopted_hardfork: false,
            adopted_base_fee: false,
            explicit_network: false,
            anvil_endpoints: Default::default(),
        }
    }
}

impl NodeConfig {
    /// Returns a test config without funded accounts, signers, or the default CREATE2 deployer.
    #[doc(hidden)]
    pub fn empty_state() -> Self {
        Self {
            genesis_accounts: vec![],
            signer_accounts: vec![],
            disable_default_create2_deployer: true,
            ..Self::test()
        }
    }

    /// Returns a config for tests: silent, on a free port.
    #[doc(hidden)]
    pub fn test() -> Self {
        Self { port: 0, silent: true, ..Default::default() }
    }

    /// Returns a test config for the Tempo network.
    #[doc(hidden)]
    pub fn test_tempo() -> Self {
        Self { networks: NetworkConfigs::with_tempo(), ..Self::test() }
    }

    /// Returns a test config for the Monad network.
    #[cfg(feature = "monad")]
    #[doc(hidden)]
    pub fn test_monad() -> Self {
        Self { networks: NetworkConfigs::with_monad(), ..Self::test() }
    }

    /// Returns a test config for the Base network.
    #[cfg(feature = "base")]
    #[doc(hidden)]
    pub fn test_base() -> Self {
        Self { networks: NetworkConfigs::with_base(), ..Self::test() }
    }

    /// Runs the Tempo network.
    pub fn with_tempo(mut self) -> Self {
        self.networks = NetworkConfigs::with_tempo();
        self
    }

    /// Runs the Monad network.
    #[cfg(feature = "monad")]
    pub fn with_monad(mut self) -> Self {
        self.networks = NetworkConfigs::with_monad();
        self
    }

    /// Runs the Optimism network.
    #[cfg(feature = "optimism")]
    pub fn with_optimism(mut self) -> Self {
        self.networks = NetworkConfigs::default()
            .try_with_chain_id(NamedChain::Optimism as u64)
            .expect("optimism is a known chain");
        self
    }

    /// Sets the network the node runs.
    pub const fn with_networks(mut self, networks: NetworkConfigs) -> Self {
        self.networks = networks;
        self
    }

    /// Sets the chain id.
    pub fn with_chain_id<U: Into<u64>>(mut self, chain_id: Option<U>) -> Self {
        self.set_chain_id(chain_id);
        self
    }

    /// Sets the chain id, and the chain id the dev wallets sign for.
    pub fn set_chain_id<U: Into<u64>>(&mut self, chain_id: Option<U>) {
        self.chain_id = chain_id.map(Into::into);
        let chain_id = self.get_chain_id();
        // The network a chain id implies is resolved at spawn, see `resolve_networks`: a fork
        // takes its endpoint's network instead.
        for wallet in self.genesis_accounts.iter_mut().chain(self.signer_accounts.iter_mut()) {
            wallet.set_chain_id(Some(chain_id));
        }
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
        if self.disable_block_gas_limit {
            return u64::MAX;
        }
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

    /// Returns the Ethereum hardfork active from genesis: the configured one, the one active on
    /// the chain at the genesis timestamp for a known chain, or the latest otherwise.
    pub fn get_hardfork(&self) -> Result<EthereumHardfork> {
        self.ethereum_hardfork_at(Chain::from_id(self.get_chain_id()), self.get_genesis_timestamp())
    }

    /// Returns the configured Ethereum hardfork, or the one active on `chain` at `timestamp`.
    ///
    /// On Monad, this is the Ethereum hardfork the Monad hardfork is based on.
    fn ethereum_hardfork_at(&self, chain: Chain, timestamp: u64) -> Result<EthereumHardfork> {
        #[cfg(feature = "monad")]
        if self.networks.is_monad() {
            return Ok(ethereum_hardfork_of_monad(self.monad_hardfork_at(timestamp)?));
        }
        match self.hardfork {
            None => {
                Ok(EthereumHardfork::from_chain_and_timestamp(chain, timestamp).unwrap_or_default())
            }
            Some(FoundryHardfork::Ethereum(hardfork)) => Ok(hardfork),
            Some(hardfork) => eyre::bail!("hardfork {hardfork:?} is not supported yet"),
        }
    }

    /// Returns the Monad hardfork active from genesis.
    #[cfg(feature = "monad")]
    pub fn get_monad_hardfork(&self) -> Result<MonadHardfork> {
        self.monad_hardfork_at(self.get_genesis_timestamp())
    }

    /// Returns the configured Monad hardfork, or the one active on the chain at `timestamp`.
    #[cfg(feature = "monad")]
    fn monad_hardfork_at(&self, timestamp: u64) -> Result<MonadHardfork> {
        match self.hardfork {
            None => Ok(MonadHardfork::from_chain_and_timestamp(self.get_chain_id(), timestamp)
                .unwrap_or_default()),
            Some(FoundryHardfork::Monad(hardfork)) => Ok(hardfork),
            Some(hardfork) => eyre::bail!("hardfork {hardfork:?} is not a Monad hardfork"),
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

    /// Loads the state to load at startup from a file. A file that does not load is ignored,
    /// as on anvil.
    pub fn with_init_state_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.init_state = SerializableState::load(path).ok();
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

    /// Sets where to fork from.
    pub const fn with_fork_choice(mut self, fork_choice: Option<ForkChoice>) -> Self {
        self.fork_choice = fork_choice;
        self
    }

    /// Sets the block to fork from.
    pub fn with_fork_block_number<U: Into<u64>>(self, fork_block_number: Option<U>) -> Self {
        self.with_fork_choice(
            fork_block_number.map(|number| ForkChoice::Block(number.into() as i128)),
        )
    }

    /// Sets the transaction to fork from.
    pub fn with_fork_transaction_hash<U: Into<B256>>(
        self,
        fork_transaction_hash: Option<U>,
    ) -> Self {
        self.with_fork_choice(
            fork_transaction_hash.map(|hash| ForkChoice::Transaction(hash.into())),
        )
    }

    /// Sets the extra HTTP headers of the fork endpoint.
    pub fn with_fork_headers(mut self, headers: Vec<String>) -> Self {
        self.fork_headers = headers;
        self
    }

    /// Sets the chain id of the fork endpoint, to skip the `eth_chainId` request.
    pub fn with_fork_chain_id(mut self, fork_chain_id: Option<U256>) -> Self {
        self.fork_chain_id = fork_chain_id.map(|chain_id| chain_id.to());
        self
    }

    /// Sets the timeout of fork requests.
    pub const fn fork_request_timeout(mut self, timeout: Option<Duration>) -> Self {
        if let Some(timeout) = timeout {
            self.fork_request_timeout = timeout;
        }
        self
    }

    /// Sets the number of retries of fork requests.
    pub const fn fork_request_retries(mut self, retries: Option<u32>) -> Self {
        if let Some(retries) = retries {
            self.fork_request_retries = retries;
        }
        self
    }

    /// Sets the initial backoff of fork request retries.
    pub const fn fork_retry_backoff(mut self, backoff: Option<Duration>) -> Self {
        if let Some(backoff) = backoff {
            self.fork_retry_backoff = backoff;
        }
        self
    }

    /// Sets the assumed compute units per second of the fork endpoint.
    pub const fn fork_compute_units_per_second(mut self, cups: Option<u64>) -> Self {
        if let Some(cups) = cups {
            self.compute_units_per_second = cups;
        }
        self
    }

    /// Skips the on-disk fork cache.
    pub const fn with_no_storage_caching(mut self, no_storage_caching: bool) -> Self {
        self.no_storage_caching = no_storage_caching;
        self
    }

    /// Fetches fork state by block number instead of block hash.
    pub const fn with_fork_state_by_number(mut self, by_number: bool) -> Self {
        self.fork_state_by_number = by_number;
        self
    }

    /// Skips the node info request to the fork endpoint.
    pub const fn with_no_fork_node_info(mut self, no_fork_node_info: bool) -> Self {
        self.no_fork_node_info = no_fork_node_info;
        self
    }

    /// Skips block access list prefetching when forking.
    pub const fn with_no_bal(mut self, no_bal: bool) -> Self {
        self.no_bal = no_bal;
        self
    }

    /// Returns whether the node forks a remote chain.
    pub const fn is_fork(&self) -> bool {
        !self.fork_urls.is_empty()
    }

    /// Returns the fork connection settings.
    pub fn fork_settings(&self) -> ForkSettings {
        ForkSettings {
            urls: self.fork_urls.iter().map(|fork| fork.url.clone()).collect(),
            headers: self.fork_headers.clone(),
            timeout: self.fork_request_timeout,
            retries: self.fork_request_retries,
            backoff: self.fork_retry_backoff,
            compute_units_per_second: self.compute_units_per_second,
            no_storage_caching: self.no_storage_caching,
            state_by_number: self.fork_state_by_number,
            instance_id: None,
        }
    }

    /// Adopts the chain id, gas limit, and timestamp of the fork block, unless configured
    /// explicitly, and re-keys the dev wallets for the chain id.
    pub fn apply_fork(&mut self, chain_id: u64, header: &SealedHeader, gas_price: u128) {
        if self.chain_id.is_none() {
            self.set_chain_id(Some(chain_id));
            self.adopted_chain_id = true;
        }
        if self.gas_limit.is_none() {
            self.gas_limit = Some(header.gas_limit);
        }
        if self.gas_price.is_none() {
            self.gas_price = Some(gas_price);
        }
        if self.base_fee.is_none() {
            self.base_fee = header.base_fee_per_gas;
            self.adopted_base_fee = true;
        }
        self.genesis_timestamp = Some(header.timestamp);
        self.genesis_block_number = Some(header.number);
    }

    /// Adopts what an anvil endpoint reports about itself: its network, unless one is selected
    /// or a fork adopted one before, and its hardfork, unless one is set explicitly. Anvil keeps
    /// the network of the first fork across resets and re-resolves the hardfork.
    pub fn adopt_fork_identity(&mut self, info: &NodeInfo) {
        if !self.adopted_fork_network && !self.networks.has_network_selection() {
            match NetworkConfigs::from_rpc_identity_profile_with_fallback(
                self.get_chain_id(),
                Some(info.network.as_deref()),
                None,
            ) {
                Ok(Some(networks)) => self.networks = networks,
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(target: "node", %error, "ignoring the network the fork endpoint reports");
                }
            }
        }
        // The first fork fixes the network, whatever it reported.
        self.adopted_fork_network = true;
        if self.hardfork.is_some() && !self.adopted_hardfork {
            return;
        }
        match self.networks.execution_network().parse_hardfork(&info.hard_fork) {
            Ok(hardfork) => {
                self.hardfork = Some(hardfork);
                self.adopted_hardfork = true;
            }
            Err(error) => {
                tracing::warn!(target: "node", %error, "ignoring the hardfork the fork endpoint reports");
            }
        }
    }

    /// Forgets what a fork endpoint chose, so a reset to another endpoint adopts that endpoint's
    /// chain id, hardfork, and identity. The network stays, as on anvil.
    pub const fn forget_fork_adoption(&mut self) {
        if self.adopted_chain_id {
            self.chain_id = None;
            self.adopted_chain_id = false;
        }
        if self.adopted_hardfork {
            self.hardfork = None;
            self.adopted_hardfork = false;
        }
    }

    /// Resolves the network of a node without an explicit network selection, as anvil does: a
    /// fork takes its endpoint's network, from the endpoint's node info or its chain id, and
    /// other nodes the network their chain id implies.
    pub async fn resolve_networks(&mut self) -> Result<()> {
        if self.adopted_fork_network {
            return Ok(());
        }
        if self.networks.has_network_selection() {
            self.explicit_network = true;
            return Ok(());
        }
        match self.fork_urls.first().map(|fork| fork.url.clone()) {
            Some(url) => {
                // A network this node cannot run yet leaves the fork on Ethereum, which serves
                // the remote chain without its own transaction types.
                if let Ok(Some(networks)) = self.fork_networks(&url).await
                    && runs_network(networks.execution_network())
                {
                    self.networks = networks;
                }
                self.adopted_fork_network = true;
            }
            None => self.networks = self.networks.with_chain_id(self.get_chain_id()),
        }
        Ok(())
    }

    /// Returns the network a fork endpoint runs: the one its anvil node info reports, or the one
    /// its chain id implies.
    pub async fn fork_networks(&self, url: &str) -> Result<Option<NetworkConfigs>> {
        let provider =
            foundry_common::provider::ProviderBuilder::<alloy_network::AnyNetwork>::new(url)
                .build()
                .wrap_err("failed to establish provider to fork url")?;
        let mut probe = NodeInfoProbe::new(self.is_anvil_endpoint(url), self.no_fork_node_info);
        let info = probe.request(&provider).await?;
        if info.is_some() {
            self.mark_anvil_endpoint(url);
        }
        let chain_id = match self.fork_chain_id {
            Some(chain_id) => chain_id,
            None => alloy_provider::Provider::get_chain_id(&provider)
                .await
                .wrap_err("failed to fetch network chain ID")?,
        };
        // A network family this build does not include is unknown here, not an error.
        Ok(NetworkConfigs::from_rpc_identity_profile_with_fallback(
            chain_id,
            info.as_ref().map(|info| info.network.as_deref()),
            None,
        )
        .ok()
        .flatten())
    }

    /// Returns whether the endpoint answered `anvil_nodeInfo` before.
    pub fn is_anvil_endpoint(&self, url: &str) -> bool {
        self.anvil_endpoints.read().contains(url)
    }

    /// Records that the endpoint answered `anvil_nodeInfo`.
    pub fn mark_anvil_endpoint(&self, url: &str) {
        self.anvil_endpoints.write().insert(url.to_string());
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

    /// Builds the chain spec of a fork: the fork block header stands in for genesis, and the
    /// genesis accounts keep their remote nonce and code.
    pub fn fork_chain_spec(
        &self,
        header: &SealedHeader,
        accounts: &[(Address, ForkGenesisAccount)],
        source: ForkSource<'_>,
    ) -> Result<Arc<ChainSpec>> {
        let hardfork =
            self.ethereum_hardfork_at(Chain::from_id(self.get_chain_id()), header.timestamp)?;
        // The replayed block of a fork at a transaction hash runs under the hardfork of the block
        // it came from, as on anvil; a newer configured hardfork activates right after it.
        let deferred = source.replay_timestamp.and_then(|timestamp| {
            let replay = EthereumHardfork::from_chain_and_timestamp(
                Chain::from_id(source.chain_id),
                timestamp,
            )?;
            (replay >= EthereumHardfork::Paris && replay < hardfork)
                .then_some((replay, timestamp + 1))
        });
        let genesis_hardfork = deferred.map_or(hardfork, |(replay, _)| replay);
        let mut genesis = self
            .genesis
            .clone()
            .unwrap_or_default()
            .with_timestamp(header.timestamp)
            .with_gas_limit(header.gas_limit)
            .with_difficulty(header.difficulty)
            .with_base_fee(header.base_fee_per_gas.map(u128::from));
        genesis.number = Some(header.number);
        genesis.config.chain_id = self.get_chain_id();

        let remote = |address: &Address| {
            accounts.iter().find(|(remote, _)| remote == address).map(|(_, account)| account)
        };
        // A funded account keeps what the genesis allocation gives it besides the balance, and
        // the remote nonce and code otherwise.
        let configured = genesis.alloc.clone();
        let fork_account = |address: &Address, balance: U256| {
            let remote = remote(address).cloned().unwrap_or_default();
            let mut account = configured.get(address).cloned().unwrap_or_default();
            account.balance = balance;
            account.nonce = account.nonce.or(Some(remote.nonce));
            account.code = account.code.or(remote.code);
            account
        };
        let mut alloc: Vec<(Address, GenesisAccount)> = self
            .genesis_accounts
            .iter()
            .map(|account| {
                (account.address(), fork_account(&account.address(), self.genesis_balance))
            })
            .collect();
        alloc.extend(
            self.funded_accounts
                .iter()
                .map(|(address, balance)| (*address, fork_account(address, *balance))),
        );
        genesis = genesis.extend_accounts(alloc);

        let builder =
            ChainSpecBuilder::default().chain(Chain::from_id(self.get_chain_id())).genesis(genesis);
        let mut spec = build_chain_spec(builder, hardfork, deferred);
        // A fork block without blob fields starts the local blob schedule at zero excess when
        // the source omits them legitimately: a block from before Cancun, or a chain whose
        // headers carry none, as anvil decides. A Cancun block that lost them stays without, so
        // blob execution fails as on anvil. The hash stays the remote one.
        let mut genesis_header = header.clone_header();
        if genesis_hardfork >= EthereumHardfork::Cancun && genesis_header.excess_blob_gas.is_none()
        {
            let explicit_hardfork = self.hardfork.is_some() && !self.adopted_hardfork;
            let source_hardfork = source
                .node_info
                .and_then(|info| {
                    self.networks.execution_network().parse_hardfork(&info.hard_fork).ok()
                })
                .and_then(|hardfork| match hardfork {
                    FoundryHardfork::Ethereum(hardfork) => Some(hardfork),
                    _ => None,
                })
                .or_else(|| {
                    EthereumHardfork::from_chain_and_timestamp(
                        Chain::from_id(source.chain_id),
                        header.timestamp,
                    )
                });
            let may_omit = source_hardfork
                .map_or(explicit_hardfork, |hardfork| hardfork < EthereumHardfork::Cancun);
            let source_chain = Chain::from_id(source.chain_id);
            if (may_omit && genesis_header.blob_gas_used.is_none())
                || source_chain.is_polygon()
                || source_chain.is_arbitrum()
            {
                genesis_header.excess_blob_gas = Some(0);
                genesis_header.blob_gas_used.get_or_insert(0);
            }
        }
        spec.genesis_header = SealedHeader::new(genesis_header, header.hash());
        Ok(Arc::new(spec))
    }

    /// Gives a state dump with a block environment but no block at its head a checkpoint block
    /// to continue from, as anvil does, on top of the dump's block before it, if any.
    pub fn ensure_dump_head(&self, state: &mut SerializableState) -> Result<()> {
        let Some(number) = state.head_number() else { return Ok(()) };
        if state.blocks.iter().any(|block| block.header.number == number) {
            return Ok(());
        }
        if !state.blocks.is_empty() {
            eyre::bail!("Best hash not found for best number {number}");
        }
        let hardfork = self.get_hardfork()?;
        let parent_hash =
            number.checked_sub(1).and_then(|parent| state.block_hash(parent)).unwrap_or_default();
        state.synthesize_head(
            parent_hash,
            CheckpointForks {
                london: hardfork >= EthereumHardfork::London,
                shanghai: hardfork >= EthereumHardfork::Shanghai,
                cancun: hardfork >= EthereumHardfork::Cancun,
                prague: hardfork >= EthereumHardfork::Prague,
            },
        );
        Ok(())
    }

    /// Builds the chain spec of a chain loaded from a state dump: the dump's head block is the
    /// genesis block, with the dump's accounts as the genesis allocation.
    pub fn dump_chain_spec(
        &self,
        header: &SealedHeader,
        state: &SerializableState,
    ) -> Result<Arc<ChainSpec>> {
        let hardfork =
            self.ethereum_hardfork_at(Chain::from_id(self.get_chain_id()), header.timestamp)?;
        let mut genesis = self
            .genesis
            .clone()
            .unwrap_or_default()
            .with_timestamp(header.timestamp)
            .with_gas_limit(header.gas_limit)
            .with_difficulty(header.difficulty)
            .with_base_fee(header.base_fee_per_gas.map(u128::from));
        genesis.number = Some(header.number);
        genesis.config.chain_id = self.get_chain_id();
        let alloc = state.accounts.iter().map(|(address, record)| {
            let storage = (!record.storage.is_empty()).then(|| record.storage.clone());
            (
                *address,
                GenesisAccount::default()
                    .with_nonce(Some(record.nonce))
                    .with_balance(record.balance)
                    .with_code((!record.code.is_empty()).then(|| record.code.clone()))
                    .with_storage(storage),
            )
        });
        genesis = genesis.extend_accounts(alloc);
        let builder =
            ChainSpecBuilder::default().chain(Chain::from_id(self.get_chain_id())).genesis(genesis);
        let mut spec = build_chain_spec(builder, hardfork, None);
        spec.genesis_header = SealedHeader::new(header.clone_header(), header.hash());
        Ok(Arc::new(spec))
    }

    /// Builds the chain spec: the configured hardfork active from genesis, with the dev accounts
    /// and the create2 deployer in the genesis allocation.
    pub fn chain_spec(&self) -> Result<Arc<ChainSpec>> {
        let hardfork = self.get_hardfork()?;
        // A loaded state continues at the block it was dumped at, with its block environment.
        let init_block = self.init_state.as_ref().and_then(|state| state.block_env());
        let init_number = self.init_state.as_ref().and_then(|state| state.head_number());
        let timestamp = match (&init_block, self.genesis_timestamp) {
            (Some(block), None) => block.timestamp.saturating_to::<u64>(),
            _ => self.get_genesis_timestamp(),
        };
        let gas_limit = match (&init_block, self.gas_limit) {
            (Some(block), None) => block.gas_limit,
            _ => self.get_gas_limit(),
        };
        let base_fee = match (&init_block, self.base_fee) {
            (Some(block), None) => block.basefee,
            _ => self.get_base_fee(),
        };
        let mut genesis = self
            .genesis
            .clone()
            .unwrap_or_default()
            .with_timestamp(timestamp)
            .with_gas_limit(gas_limit);
        genesis.config.chain_id = self.get_chain_id();
        let number = init_number.unwrap_or_else(|| self.get_genesis_number());
        if number > 0 {
            genesis.number = Some(number);
        }
        if hardfork >= EthereumHardfork::London {
            genesis = genesis.with_base_fee(Some(base_fee.into()));
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
        // The system contracts anvil deploys at genesis, so the block executor's system calls
        // record beacon roots, block hashes, and requests.
        if hardfork >= EthereumHardfork::Cancun {
            alloc.push((
                eip4788::BEACON_ROOTS_ADDRESS,
                GenesisAccount::default().with_code(Some(eip4788::BEACON_ROOTS_CODE.clone())),
            ));
        }
        if hardfork >= EthereumHardfork::Prague {
            alloc.extend(
                [
                    (eip2935::HISTORY_STORAGE_ADDRESS, eip2935::HISTORY_STORAGE_CODE.clone()),
                    (
                        eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
                        eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_CODE.clone(),
                    ),
                    (
                        eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
                        eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_CODE.clone(),
                    ),
                ]
                .map(|(address, code)| (address, GenesisAccount::default().with_code(Some(code)))),
            );
        }
        if !self.disable_default_create2_deployer {
            alloc.push((
                DEFAULT_CREATE2_DEPLOYER,
                GenesisAccount::default()
                    .with_code(Some(Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE))),
            ));
        }
        if let Some(state) = &self.init_state {
            alloc.extend(state.accounts.iter().map(|(address, record)| {
                let storage = (!record.storage.is_empty()).then(|| record.storage.clone());
                (
                    *address,
                    GenesisAccount::default()
                        .with_nonce(Some(record.nonce))
                        .with_balance(record.balance)
                        .with_code((!record.code.is_empty()).then(|| record.code.clone()))
                        .with_storage(storage),
                )
            }));
        }
        genesis = genesis.extend_accounts(alloc);

        let builder =
            ChainSpecBuilder::default().chain(Chain::from_id(self.get_chain_id())).genesis(genesis);
        Ok(Arc::new(build_chain_spec(builder, hardfork, None)))
    }
}

/// Returns whether this build runs the network.
const fn runs_network(network: NetworkVariant) -> bool {
    match network {
        NetworkVariant::Ethereum => true,
        #[cfg(feature = "monad")]
        NetworkVariant::Monad => true,
        _ => false,
    }
}

/// What the fork endpoint reported about its chain, for decisions about the fork block.
#[derive(Clone, Copy, Debug)]
pub struct ForkSource<'a> {
    /// The chain id of the endpoint.
    pub chain_id: u64,
    /// The endpoint's node info, when it is an anvil node.
    pub node_info: Option<&'a NodeInfo>,
    /// The timestamp of the block a fork at a transaction hash replays, if any.
    pub replay_timestamp: Option<u64>,
}

/// Returns the Ethereum hardfork a Monad hardfork is based on.
#[cfg(feature = "monad")]
const fn ethereum_hardfork_of_monad(hardfork: MonadHardfork) -> EthereumHardfork {
    match hardfork.into_eth_spec() {
        SpecId::PRAGUE => EthereumHardfork::Prague,
        _ => EthereumHardfork::Osaka,
    }
}

/// Builds a chain spec with every hardfork up to and including `hardfork` active at genesis, and
/// anvil's blob schedule for that hardfork.
/// With `deferred`, only the hardforks up to the given one are active at genesis, and the rest up
/// to `hardfork` activate at the given timestamp.
fn build_chain_spec(
    builder: ChainSpecBuilder,
    hardfork: EthereumHardfork,
    deferred: Option<(EthereumHardfork, u64)>,
) -> ChainSpec {
    let (builder, switch) = match deferred {
        Some((at_genesis, timestamp)) => (
            EthereumHardfork::VARIANTS
                .iter()
                .filter(|variant| **variant > at_genesis && **variant <= hardfork)
                .fold(activate_hardfork(builder, at_genesis), |builder, variant| {
                    builder.with_fork(*variant, ForkCondition::Timestamp(timestamp))
                }),
            timestamp,
        ),
        None => (activate_hardfork(builder, hardfork), 0),
    };
    let mut spec = builder.build();
    let mut blob_params = BlobScheduleBlobParams {
        cancun: BlobParams::cancun(),
        prague: BlobParams::prague(),
        osaka: BlobParams::osaka(),
        scheduled: Vec::new(),
    };
    if hardfork > EthereumHardfork::Osaka {
        blob_params.scheduled.push((switch, get_blob_params_by_hardfork(hardfork.into())));
    }
    spec.blob_params = blob_params;
    spec
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
