use crate::{
    AccountGenerator, CHAIN_ID, DEFAULT_MNEMONIC, DEFAULT_SLOTS_IN_AN_EPOCH, NodeConfig,
    state_dump::{SerializableState, StateFile},
    types::{ForkChoice, ForkUrl, TransactionOrder},
};
use alloy_genesis::Genesis;
use alloy_primitives::{Address, B256, U256, map::HashMap, utils::Unit};
use alloy_signer_local::coins_bip39::{English, Mnemonic};
use clap::Parser;
use eyre::Result;
use foundry_common::shell;
use foundry_config::Chain;
use foundry_evm_hardforks::FoundryHardfork;
use foundry_evm_networks::NetworkConfigs;
use rand_08::{SeedableRng, rngs::StdRng};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

/// The `anvil` node options.
#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    /// Port number to listen on.
    #[arg(long, short, default_value = "8545", value_name = "NUM")]
    pub port: u16,

    /// Number of dev accounts to generate and configure.
    #[arg(long, short, default_value = "10", value_name = "NUM")]
    pub accounts: u64,

    /// The balance of every dev account in Ether.
    #[arg(long, default_value = "10000", value_name = "NUM")]
    pub balance: u64,

    /// The timestamp of the genesis block.
    #[arg(long, value_name = "NUM")]
    pub timestamp: Option<u64>,

    /// The number of the genesis block.
    #[arg(long, value_name = "NUM")]
    pub number: Option<u64>,

    /// BIP39 mnemonic phrase used for generating accounts.
    /// Cannot be used if `mnemonic_random` or `mnemonic_seed` are used.
    #[arg(long, short, conflicts_with_all = &["mnemonic_seed", "mnemonic_random"])]
    pub mnemonic: Option<String>,

    /// Automatically generates a BIP39 mnemonic phrase, and derives accounts from it.
    /// Cannot be used with other `mnemonic` options.
    /// You can specify the number of words you want in the mnemonic.
    /// [default: 12]
    #[arg(long, conflicts_with_all = &["mnemonic", "mnemonic_seed"], default_missing_value = "12", num_args(0..=1))]
    pub mnemonic_random: Option<usize>,

    /// Generates a BIP39 mnemonic phrase from a given seed
    /// Cannot be used with other `mnemonic` options.
    ///
    /// CAREFUL: This is NOT SAFE and should only be used for testing.
    /// Never use the private keys generated in production.
    #[arg(long = "mnemonic-seed-unsafe", conflicts_with_all = &["mnemonic", "mnemonic_random"])]
    pub mnemonic_seed: Option<u64>,

    /// Sets the derivation path of the child key to be derived.
    ///
    /// [default: m/44'/60'/0'/0/]
    #[arg(long)]
    pub derivation_path: Option<String>,

    /// The account used to sponsor Tempo fee-payer requests (`eth_signRawTransaction` and raw
    /// transactions carrying the sponsorship placeholder).
    ///
    /// Must be an unlocked account. Only used on Tempo networks; defaults to the last dev
    /// account.
    #[arg(long = "tempo.fee-payer", value_name = "ADDRESS")]
    pub tempo_fee_payer: Option<Address>,

    /// Override the Base activation-registry administrator.
    #[cfg(feature = "base")]
    #[arg(long, value_name = "ADDRESS")]
    pub base_activation_admin: Option<Address>,

    /// The EVM hardfork to use.
    ///
    /// Choose the hardfork by name, e.g. `prague`, `cancun`, `shanghai`, `paris`, `london`, etc...
    /// [default: latest]
    #[arg(long)]
    pub hardfork: Option<String>,

    /// Block time in seconds for interval mining.
    #[arg(short, long, visible_alias = "blockTime", value_name = "SECONDS", value_parser = duration_from_secs_f64)]
    pub block_time: Option<Duration>,

    /// Slots in an epoch
    #[arg(long, value_name = "SLOTS_IN_AN_EPOCH", default_value_t = DEFAULT_SLOTS_IN_AN_EPOCH)]
    pub slots_in_an_epoch: u64,

    /// Writes output of `anvil` as json to user-specified file.
    #[arg(long, value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    pub config_out: Option<PathBuf>,

    /// Disable auto and interval mining, and mine on demand instead.
    #[arg(long, visible_alias = "no-mine", conflicts_with = "block_time")]
    pub no_mining: bool,

    /// Enable mixed mining mode. Blocks are mined on a timer (set by `--block-time`),
    /// but also whenever a transaction is submitted. Requires `--block-time` to be set.
    #[arg(long, requires = "block_time")]
    pub mixed_mining: bool,

    /// The hosts the server will listen on.
    #[arg(
        long,
        value_name = "IP_ADDR",
        env = "ANVIL_IP_ADDR",
        default_value = "127.0.0.1",
        help_heading = "Server options",
        value_delimiter = ','
    )]
    pub host: Vec<IpAddr>,

    /// How transactions are sorted in the mempool.
    #[arg(long, default_value = "fees")]
    pub order: TransactionOrder,

    /// Initialize the genesis block with the given `genesis.json` file.
    #[arg(long, value_name = "PATH", value_parser = read_genesis_file)]
    pub init: Option<Genesis>,

    /// This is an alias for both --load-state and --dump-state.
    ///
    /// It initializes the chain with the state and block environment stored at the file, if it
    /// exists, and dumps the chain's state on exit.
    #[arg(
        long,
        value_name = "PATH",
        value_parser = StateFile::parse,
        conflicts_with_all = &[
            "init",
            "dump_state",
            "load_state"
        ]
    )]
    pub state: Option<StateFile>,

    /// Interval in seconds at which the state and block environment is to be dumped to disk.
    ///
    /// See --state and --dump-state
    #[arg(short, long, value_name = "SECONDS")]
    pub state_interval: Option<u64>,

    /// Dump the state and block environment of chain on exit to the given file.
    ///
    /// If the value is a directory, the state will be written to `<VALUE>/state.json`.
    #[arg(long, value_name = "PATH", conflicts_with = "init")]
    pub dump_state: Option<PathBuf>,

    /// Preserve historical state snapshots when dumping the state.
    ///
    /// This will save the in-memory states of the chain at particular block hashes.
    ///
    /// These historical states will be loaded into the memory when `--load-state` / `--state`,
    /// and aids in RPC calls beyond the block at which state was dumped.
    #[arg(long, conflicts_with = "init", default_value = "false")]
    pub preserve_historical_states: bool,

    /// Initialize the chain from a previously saved state snapshot.
    #[arg(
        long,
        value_name = "PATH",
        value_parser = SerializableState::parse,
        conflicts_with = "init"
    )]
    pub load_state: Option<SerializableState>,

    /// Fund specified accounts with a given amount in Ether at genesis (without affecting
    /// signers).
    #[arg(long, value_name = "ADDRESS:AMOUNT", value_delimiter = ' ', num_args = 1..)]
    pub fund_accounts: Vec<String>,

    #[arg(long, help = IPC_HELP, value_name = "PATH", visible_alias = "ipcpath")]
    pub ipc: Option<Option<String>>,

    /// Don't keep full chain history.
    /// If a number argument is specified, at most this number of states is kept in memory.
    ///
    /// If enabled, no state will be persisted on disk, so `max_persisted_states` will be 0.
    #[arg(long)]
    pub prune_history: Option<Option<usize>>,

    /// Max number of states to persist on disk.
    ///
    /// Note that `prune_history` will overwrite `max_persisted_states` to 0.
    #[arg(long, conflicts_with = "prune_history")]
    pub max_persisted_states: Option<usize>,

    /// Number of blocks with transactions to keep in memory.
    #[arg(long)]
    pub transaction_block_keeper: Option<usize>,

    /// Maximum number of transactions per block.
    #[arg(long)]
    pub max_transactions: Option<usize>,

    #[command(flatten)]
    pub evm: AnvilEvmArgs,

    #[command(flatten)]
    pub server_config: ServerArgs,

    /// Path to the cache directory where states are stored.
    #[arg(long, value_name = "PATH")]
    pub cache_path: Option<PathBuf>,
}

#[cfg(windows)]
const IPC_HELP: &str =
    "Launch an ipc server at the given path or default path = `\\\\.\\pipe\\anvil.ipc`";

#[cfg(not(windows))]
const IPC_HELP: &str = "Launch an ipc server at the given path or default path = `/tmp/anvil.ipc`";

impl NodeArgs {
    /// Builds the node config from the arguments.
    pub fn into_node_config(self) -> Result<NodeConfig> {
        let genesis_balance = Unit::ETHER.wei().saturating_mul(U256::from(self.balance));
        let funded_accounts = self.parse_funded_accounts()?;
        // A chain id of a known network selects it, unless a fork endpoint will tell.
        let local_chain_id = self
            .evm
            .chain_id
            .map(u64::from)
            .or_else(|| self.init.as_ref().map(|genesis| genesis.config.chain_id));
        let inferred_chain_id = self
            .evm
            .fork_chain_id
            .map(u64::from)
            .or(if self.evm.fork_url.is_empty() { local_chain_id } else { None });
        let networks = match inferred_chain_id {
            Some(chain_id) => {
                self.evm.networks.try_with_chain_id(chain_id).map_err(eyre::Report::msg)?
            }
            None => self.evm.networks,
        };
        let hardfork = self
            .hardfork
            .as_deref()
            .map(|hardfork| parse_hardfork(hardfork, &networks))
            .transpose()?;
        let networks = match hardfork {
            Some(hardfork) => {
                networks.normalize_for_hardfork(hardfork).map_err(eyre::Report::msg)?
            }
            None => networks,
        };
        let compute_units_per_second =
            if self.evm.no_rate_limit { Some(u64::MAX) } else { self.evm.compute_units_per_second };
        let fork_choice = match (self.evm.fork_block_number, self.evm.fork_transaction_hash) {
            (Some(number), _) => Some(ForkChoice::Block(number)),
            (None, Some(hash)) => Some(ForkChoice::Transaction(hash)),
            (None, None) => None,
        };

        let config = NodeConfig::default()
            .with_gas_limit(self.evm.gas_limit)
            .disable_block_gas_limit(self.evm.disable_block_gas_limit)
            .enable_tx_gas_limit(self.evm.enable_tx_gas_limit)
            .with_gas_price(self.evm.gas_price)
            .with_hardfork(hardfork)
            .with_networks(networks)
            .with_blocktime(self.block_time)
            .with_no_mining(self.no_mining)
            .with_mixed_mining(self.mixed_mining, self.block_time)
            .with_account_generator(self.account_generator())?
            .with_genesis_balance(genesis_balance)
            .with_genesis_timestamp(self.timestamp)
            .with_genesis_block_number(self.number)
            .with_port(self.port)
            .with_fork_urls(self.evm.fork_url)
            .with_fork_choice(fork_choice)
            .with_fork_headers(self.evm.fork_headers)
            .with_fork_chain_id(self.evm.fork_chain_id.map(u64::from).map(U256::from))
            .with_no_fork_node_info(self.evm.no_fork_node_info)
            .with_no_bal(self.evm.no_bal)
            .with_fork_state_by_number(self.evm.fork_state_by_number)
            .fork_request_timeout(self.evm.fork_request_timeout.map(Duration::from_millis))
            .fork_request_retries(self.evm.fork_request_retries)
            .fork_retry_backoff(self.evm.fork_retry_backoff.map(Duration::from_millis))
            .fork_compute_units_per_second(compute_units_per_second)
            .with_no_storage_caching(self.evm.no_storage_caching)
            .with_base_fee(self.evm.block_base_fee_per_gas)
            .disable_min_priority_fee(self.evm.disable_min_priority_fee)
            .with_server_config(
                self.server_config.allow_origin,
                self.server_config.no_cors,
                self.server_config.no_request_size_limit,
            )
            .with_host(self.host)
            .set_silent(shell::is_quiet())
            .set_config_out(self.config_out)
            .with_transaction_order(self.order)
            .with_genesis(self.init)
            .with_steps_tracing(self.evm.steps_tracing)
            .with_print_logs(!self.evm.disable_console_log)
            .with_print_traces(self.evm.print_traces)
            .with_auto_impersonate(self.evm.auto_impersonate)
            .with_ipc(self.ipc)
            .with_code_size_limit(self.evm.code_size_limit)
            .disable_code_size_limit(self.evm.disable_code_size_limit)
            .set_pruned_history(self.prune_history)
            .with_init_state(self.load_state.or_else(|| self.state.and_then(|s| s.state)))
            .with_transaction_block_keeper(self.transaction_block_keeper)
            .with_max_transactions(self.max_transactions)
            .with_max_persisted_states(self.max_persisted_states)
            .with_chain_id(self.evm.chain_id.map(u64::from))
            .with_disable_default_create2_deployer(self.evm.disable_default_create2_deployer)
            .with_disable_pool_balance_checks(self.evm.disable_pool_balance_checks)
            .with_tempo_fee_payer(self.tempo_fee_payer)
            .with_slots_in_an_epoch(self.slots_in_an_epoch)
            .with_memory_limit(self.evm.memory_limit)
            .with_cache_path(self.cache_path)
            .with_funded_accounts(funded_accounts);
        #[cfg(feature = "base")]
        let config = config.with_base_activation_admin(self.base_activation_admin);
        Ok(config)
    }

    fn parse_funded_accounts(&self) -> Result<HashMap<Address, U256>> {
        let mut accounts = HashMap::default();
        for entry in &self.fund_accounts {
            let Some((address, amount)) = entry.split_once(':') else {
                eyre::bail!(
                    "Invalid fund-accounts entry '{entry}'. Expected format: ADDRESS:AMOUNT"
                );
            };
            let address = address
                .parse::<Address>()
                .map_err(|e| eyre::eyre!("Invalid address '{address}': {e}"))?;
            let amount: u64 =
                amount.parse().map_err(|e| eyre::eyre!("Invalid amount '{amount}': {e}"))?;
            accounts.insert(address, Unit::ETHER.wei().saturating_mul(U256::from(amount)));
        }
        Ok(accounts)
    }

    fn account_generator(&self) -> AccountGenerator {
        let mut generator = AccountGenerator::new(self.accounts as usize)
            .phrase(DEFAULT_MNEMONIC)
            .chain_id(self.evm.chain_id.map(u64::from).unwrap_or(CHAIN_ID));
        if let Some(mnemonic) = &self.mnemonic {
            generator = generator.phrase(mnemonic);
        } else if let Some(count) = self.mnemonic_random {
            let mut rng = rand_08::thread_rng();
            let mnemonic = match Mnemonic::<English>::new_with_count(&mut rng, count) {
                Ok(mnemonic) => mnemonic.to_phrase(),
                Err(err) => {
                    tracing::warn!(target: "node", ?count, %err, "failed to generate mnemonic, falling back to 12-word random mnemonic");
                    Mnemonic::<English>::new_with_count(&mut rng, 12)
                        .expect("valid default word count")
                        .to_phrase()
                }
            };
            generator = generator.phrase(mnemonic);
        } else if let Some(seed) = self.mnemonic_seed {
            let mut seed = StdRng::seed_from_u64(seed);
            generator = generator.phrase(Mnemonic::<English>::new(&mut seed).to_phrase());
        }
        if let Some(derivation) = &self.derivation_path {
            generator = generator.derivation_path(derivation);
        }
        generator
    }

    /// Runs the node until it exits.
    pub async fn run(self) -> Result<()> {
        let dump_state = self
            .dump_state
            .as_ref()
            .or_else(|| self.state.as_ref().map(|s| &s.path))
            .cloned()
            .map(|path| if path.is_dir() { path.join("state.json") } else { path });
        let preserve_historical_states = self.preserve_historical_states;
        let dump_interval =
            self.state_interval.map(Duration::from_secs).unwrap_or(DEFAULT_DUMP_INTERVAL);
        let config = self.into_node_config()?;
        let (api, handle) = crate::try_spawn(config).await?;
        handle.print()?;

        if let Some(path) = dump_state.clone() {
            let api = api.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval_at(
                    tokio::time::Instant::now() + dump_interval,
                    dump_interval,
                );
                loop {
                    interval.tick().await;
                    dump_state_to(&api, &path, preserve_historical_states).await;
                }
            });
        }

        let shutdown = async {
            #[cfg(unix)]
            let sigterm = async {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut stream) => {
                        stream.recv().await;
                    }
                    Err(_) => futures::future::pending::<()>().await,
                }
            };
            #[cfg(not(unix))]
            let sigterm = futures::future::pending::<()>();
            tokio::select! {
                _ = sigterm => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        };
        let result = tokio::select! {
            result = handle.node_exit_future => result,
            _ = shutdown => Ok(()),
        };
        if let Some(path) = &dump_state {
            dump_state_to(&api, path, preserve_historical_states).await;
        }
        result
    }
}

/// Default interval of `--state-interval`.
const DEFAULT_DUMP_INTERVAL: Duration = Duration::from_secs(60);

/// Writes the state dump to `path` as JSON. Failures are logged, as the node keeps running.
async fn dump_state_to(api: &crate::EthApi, path: &Path, preserve_historical_states: bool) {
    let state = match api.anvil_dump_state(Some(preserve_historical_states)).await {
        Ok(bytes) => SerializableState::decode(&bytes),
        Err(error) => Err(error),
    };
    match state {
        Ok(state) => {
            if let Err(error) = foundry_common::fs::write_json_file(path, &state) {
                tracing::error!(target: "node", ?path, %error, "failed to write state dump");
            }
        }
        Err(error) => tracing::error!(target: "node", %error, "failed to dump state"),
    }
}

/// The EVM options.
#[derive(Clone, Debug, Parser)]
#[command(next_help_heading = "EVM options")]
pub struct AnvilEvmArgs {
    /// Fetch state over a remote endpoint instead of starting from an empty state.
    ///
    /// If you want to fetch state from a specific block number, add a block number like
    /// `http://localhost:8545@1400000` or use the `--fork-block-number` argument.
    #[arg(
        long,
        short,
        visible_alias = "rpc-url",
        value_name = "URL",
        help_heading = "Fork config"
    )]
    pub fork_url: Vec<ForkUrl>,

    /// Headers to use for the rpc client, e.g. "User-Agent: test-agent"
    ///
    /// See --fork-url.
    #[arg(
        long = "fork-header",
        value_name = "HEADERS",
        help_heading = "Fork config",
        requires = "fork_url"
    )]
    pub fork_headers: Vec<String>,

    /// Timeout in ms for requests sent to remote JSON-RPC server in forking mode.
    ///
    /// Default value 45000
    #[arg(id = "timeout", long = "timeout", help_heading = "Fork config", requires = "fork_url")]
    pub fork_request_timeout: Option<u64>,

    /// Number of retry requests for spurious networks (timed out requests)
    ///
    /// Default value 5
    #[arg(id = "retries", long = "retries", help_heading = "Fork config", requires = "fork_url")]
    pub fork_request_retries: Option<u32>,

    /// Fetch state from a specific block number over a remote endpoint.
    ///
    /// If negative, the given value is subtracted from the `latest` block number.
    ///
    /// See --fork-url.
    #[arg(
        long,
        requires = "fork_url",
        value_name = "BLOCK",
        help_heading = "Fork config",
        allow_hyphen_values = true
    )]
    pub fork_block_number: Option<i128>,

    /// Fetch state from a specific transaction hash over a remote endpoint.
    ///
    /// See --fork-url.
    #[arg(
        long,
        requires = "fork_url",
        value_name = "TRANSACTION",
        help_heading = "Fork config",
        conflicts_with = "fork_block_number"
    )]
    pub fork_transaction_hash: Option<B256>,

    /// Initial retry backoff on encountering errors.
    ///
    /// See --fork-url.
    #[arg(long, requires = "fork_url", value_name = "BACKOFF", help_heading = "Fork config")]
    pub fork_retry_backoff: Option<u64>,

    /// Specify chain id to skip fetching it from remote endpoint. This enables offline-start mode.
    ///
    /// You still must pass both `--fork-url` and `--fork-block-number`, and already have your
    /// required state cached on disk, anything missing locally would be fetched from the
    /// remote.
    #[arg(
        long,
        help_heading = "Fork config",
        value_name = "CHAIN",
        requires = "fork_block_number"
    )]
    pub fork_chain_id: Option<Chain>,

    /// Disable fetching node info from the fork endpoint.
    #[arg(long, requires = "fork_url", help_heading = "Fork config")]
    pub no_fork_node_info: bool,

    /// Disable block access list prefetching when forking.
    #[arg(long, help_heading = "Fork config")]
    pub no_bal: bool,

    /// Fetch fork state by block number instead of block hash.
    #[arg(long, requires = "fork_url", help_heading = "Fork config")]
    pub fork_state_by_number: bool,

    /// Sets the number of assumed available compute units per second for this provider
    ///
    /// default value: 330
    ///
    /// See also --fork-url and <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[arg(
        long,
        requires = "fork_url",
        alias = "cups",
        value_name = "CUPS",
        help_heading = "Fork config"
    )]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also --fork-url and <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[arg(
        long,
        requires = "fork_url",
        value_name = "NO_RATE_LIMITS",
        help_heading = "Fork config",
        visible_alias = "no-rpc-rate-limit"
    )]
    pub no_rate_limit: bool,

    /// Explicitly disables the use of RPC caching.
    ///
    /// All storage slots are read entirely from the endpoint.
    ///
    /// This flag overrides the project's configuration file.
    ///
    /// See --fork-url.
    #[arg(long, requires = "fork_url", help_heading = "Fork config")]
    pub no_storage_caching: bool,

    /// The block gas limit.
    #[arg(long, alias = "block-gas-limit", help_heading = "Environment config")]
    pub gas_limit: Option<u64>,

    /// Disable the `call.gas_limit <= block.gas_limit` constraint.
    #[arg(
        long,
        value_name = "DISABLE_GAS_LIMIT",
        help_heading = "Environment config",
        alias = "disable-gas-limit",
        conflicts_with = "gas_limit"
    )]
    pub disable_block_gas_limit: bool,

    /// Enable the transaction gas limit cap (EIP-7825).
    #[arg(long, visible_alias = "tx-gas-limit", help_heading = "Environment config")]
    pub enable_tx_gas_limit: bool,

    /// EIP-170: Contract code size limit in bytes. Useful to increase this because some
    /// contracts, for example Solidity ones, can be larger than the default. By default,
    /// it is 0x6000 (~25kb).
    #[arg(long, value_name = "CODE_SIZE", help_heading = "Environment config")]
    pub code_size_limit: Option<usize>,

    /// Disable EIP-170: Contract code size limit.
    #[arg(
        long,
        value_name = "DISABLE_CODE_SIZE_LIMIT",
        conflicts_with = "code_size_limit",
        help_heading = "Environment config"
    )]
    pub disable_code_size_limit: bool,

    /// The gas price.
    #[arg(long, help_heading = "Environment config")]
    pub gas_price: Option<u128>,

    /// The base fee in a block.
    #[arg(
        long,
        visible_alias = "base-fee",
        value_name = "FEE",
        help_heading = "Environment config"
    )]
    pub block_base_fee_per_gas: Option<u64>,

    /// Disable the enforcement of a minimum suggested priority fee.
    #[arg(long, visible_alias = "no-priority-fee", help_heading = "Environment config")]
    pub disable_min_priority_fee: bool,

    /// The chain ID.
    #[arg(long, alias = "chain", help_heading = "Environment config")]
    pub chain_id: Option<Chain>,

    /// Enable steps tracing used for debug calls returning geth-style traces
    #[arg(long, visible_alias = "tracing")]
    pub steps_tracing: bool,

    /// Disable printing of `console.log` invocations to stdout.
    #[arg(long, visible_alias = "no-console-log")]
    pub disable_console_log: bool,

    /// Enable printing of traces for executed transactions and `eth_call` to stdout.
    #[arg(long, visible_alias = "enable-trace-printing")]
    pub print_traces: bool,

    /// Enables automatic impersonation on startup. This allows any transaction sender to be
    /// simulated as different accounts, which is useful for testing contract behavior.
    #[arg(long, visible_alias = "auto-unlock")]
    pub auto_impersonate: bool,

    /// Disable the default create2 deployer
    #[arg(long, visible_alias = "no-create2")]
    pub disable_default_create2_deployer: bool,

    /// Disable balance checks for transactions in the pool.
    #[arg(long)]
    pub disable_pool_balance_checks: bool,

    /// The memory limit per EVM execution in bytes.
    #[arg(long)]
    pub memory_limit: Option<u64>,

    #[command(flatten)]
    pub networks: NetworkConfigs,
}

/// The server options.
#[derive(Clone, Debug, Parser)]
#[command(next_help_heading = "Server options")]
pub struct ServerArgs {
    /// The cors `allow_origin` header
    #[arg(long, default_value = "*")]
    pub allow_origin: String,

    /// Disable CORS.
    #[arg(long, conflicts_with = "allow_origin")]
    pub no_cors: bool,

    /// Disable the default request body size limit. At time of writing the default limit is 2MB.
    #[arg(long)]
    pub no_request_size_limit: bool,
}

/// Parses a hardfork name, in the namespace of the selected network when it carries none.
fn parse_hardfork(hardfork: &str, networks: &NetworkConfigs) -> Result<FoundryHardfork> {
    if let Ok(hardfork) = FoundryHardfork::from_str(hardfork) {
        networks.normalize_for_hardfork(hardfork).map_err(eyre::Report::msg)?;
        return Ok(hardfork);
    }
    networks.execution_network().parse_hardfork(hardfork).map_err(eyre::Report::msg)
}

/// Clap's value parser for genesis. Loads a genesis.json file.
fn read_genesis_file(path: &str) -> Result<Genesis, String> {
    foundry_common::fs::read_json_file(path.as_ref()).map_err(|err| err.to_string())
}

fn duration_from_secs_f64(s: &str) -> Result<Duration, String> {
    let s = s.parse::<f64>().map_err(|e| e.to_string())?;
    if s == 0.0 {
        return Err("Duration must be greater than 0".to_string());
    }
    Duration::try_from_secs_f64(s).map_err(|e| e.to_string())
}
