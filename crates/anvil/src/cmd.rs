use crate::{
    AccountGenerator, CHAIN_ID, NodeConfig,
    config::{DEFAULT_MNEMONIC, ForkChoice},
    eth::{EthApi, backend::db::SerializableState, pool::transactions::TransactionOrder},
};
use alloy_genesis::Genesis;
use alloy_primitives::{B256, U256, utils::Unit};
use alloy_signer_local::coins_bip39::{English, Mnemonic};
use anvil_server::ServerConfig;
use clap::Parser;
use core::fmt;
use foundry_common::shell;
use foundry_config::{Chain, Config, FigmentProviders};
use foundry_evm::hardfork::{EthereumHardfork, OpHardfork};
use foundry_evm_networks::NetworkConfigs;
use futures::FutureExt;
use rand_08::{SeedableRng, rngs::StdRng};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    pin::Pin,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Instant, Interval};

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    /// Port number to listen on.
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(long, short, value_name = "NUM")]
    pub port: Option<u16>,

    /// Number of dev accounts to generate and configure.
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(long, short, value_name = "NUM")]
    pub accounts: Option<u64>,

    /// The balance of every dev account in Ether.
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(long, value_name = "NUM")]
    pub balance: Option<u64>,

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
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(long, value_name = "SLOTS_IN_AN_EPOCH")]
    pub slots_in_an_epoch: Option<u64>,

    /// Writes output of `anvil` as json to user-specified file.
    #[arg(long, value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    pub config_out: Option<PathBuf>,

    /// Disable auto and interval mining, and mine on demand instead.
    #[arg(long, visible_alias = "no-mine", conflicts_with = "block_time")]
    pub no_mining: bool,

    #[arg(long, requires = "block_time")]
    pub mixed_mining: bool,

    /// The hosts the server will listen on.
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(
        long,
        value_name = "IP_ADDR",
        env = "ANVIL_IP_ADDR",
        help_heading = "Server options",
        value_delimiter = ','
    )]
    pub host: Option<Vec<IpAddr>>,

    /// How transactions are sorted in the mempool.
    ///
    /// Can also be set in `foundry.toml` under `[anvil]`. CLI flags take precedence.
    #[arg(long)]
    pub order: Option<TransactionOrder>,

    /// Initialize the genesis block with the given `genesis.json` file.
    #[arg(long, value_name = "PATH", value_parser= read_genesis_file)]
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
    /// These historical states will be loaded into the memory when `--load-state` / `--state`, and
    /// aids in RPC calls beyond the block at which state was dumped.
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

    /// Maximum number of transactions in a block.
    #[arg(long)]
    pub max_transactions: Option<usize>,

    #[command(flatten)]
    pub evm: AnvilEvmArgs,

    #[command(flatten)]
    pub server_config: ServerConfig,

    /// Path to the cache directory where persisted states are stored (see
    /// `--max-persisted-states`).
    ///
    /// Note: This does not affect the fork RPC cache location (`storage.json`), which is stored in
    /// `~/.foundry/cache/rpc/<chain>/<block>/`.
    #[arg(long, value_name = "PATH")]
    pub cache_path: Option<PathBuf>,
}

#[cfg(windows)]
const IPC_HELP: &str =
    "Launch an ipc server at the given path or default path = `\\.\\pipe\\anvil.ipc`";

/// The default IPC endpoint
#[cfg(not(windows))]
const IPC_HELP: &str = "Launch an ipc server at the given path or default path = `/tmp/anvil.ipc`";

/// Default interval for periodically dumping the state.
const DEFAULT_DUMP_INTERVAL: Duration = Duration::from_secs(60);

impl NodeArgs {
    pub fn into_node_config(self) -> eyre::Result<NodeConfig> {
        // Load [anvil] from foundry.toml (if in a foundry project)
        let anvil_config = Config::load_with_providers(FigmentProviders::Anvil)?.anvil;

        // Build account generator before destructuring self.
        // CLI mnemonic flags (--mnemonic, --mnemonic-random, --mnemonic-seed-unsafe) suppress
        // config mnemonic to avoid surprising override behavior.
        let has_cli_mnemonic = self.mnemonic.is_some()
            || self.mnemonic_random.is_some()
            || self.mnemonic_seed.is_some();
        let config_mnemonic = if has_cli_mnemonic { None } else { anvil_config.mnemonic.clone() };
        let config_derivation_path =
            if has_cli_mnemonic { None } else { anvil_config.derivation_path.clone() };

        let merged_chain_id = self
            .evm
            .chain_id
            .unwrap_or_else(|| anvil_config.chain_id.map(Chain::from).unwrap_or(CHAIN_ID.into()));

        let merged_accounts = self.accounts.unwrap_or(anvil_config.accounts);
        let account_generator = self.account_generator(
            merged_accounts,
            merged_chain_id,
            &config_mnemonic,
            &config_derivation_path,
        );

        // Destructure self to take ownership of all fields.
        let Self {
            port,
            accounts: _,
            balance,
            timestamp,
            number,
            mnemonic: _,
            mnemonic_random: _,
            mnemonic_seed: _,
            derivation_path: _,
            hardfork,
            block_time,
            slots_in_an_epoch,
            config_out,
            no_mining,
            mixed_mining,
            host,
            order,
            init,
            state,
            state_interval: _,
            dump_state: _,
            preserve_historical_states: _,
            load_state,
            ipc,
            prune_history,
            max_persisted_states,
            transaction_block_keeper,
            max_transactions,
            evm,
            server_config,
            cache_path,
        } = self;

        // Merge CLI > config > defaults for required fields
        let port = port.unwrap_or(anvil_config.port);
        let balance = balance.unwrap_or(anvil_config.balance);
        let slots_in_an_epoch = slots_in_an_epoch.unwrap_or(anvil_config.slots_in_an_epoch);

        let genesis_balance = Unit::ETHER.wei().saturating_mul(U256::from(balance));

        // Merge host: CLI > config > default (127.0.0.1)
        let host = host.unwrap_or_else(|| {
            if !anvil_config.host.is_empty() {
                anvil_config
                    .host
                    .iter()
                    .map(|h| {
                        h.parse::<IpAddr>().unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
                    })
                    .collect()
            } else {
                vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]
            }
        });

        // Merge order: CLI > config > default (fees)
        let order = order.unwrap_or_else(|| {
            if let Some(ref order_str) = anvil_config.order {
                order_str.parse::<TransactionOrder>().unwrap_or(TransactionOrder::Fees)
            } else {
                TransactionOrder::Fees
            }
        });

        // Merge hardfork: CLI > config
        let hardfork_str = hardfork.or(anvil_config.hardfork);
        let hardfork = match &hardfork_str {
            Some(hf) => {
                if evm.networks.is_optimism() {
                    Some(OpHardfork::from_str(hf)?.into())
                } else {
                    Some(EthereumHardfork::from_str(hf)?.into())
                }
            }
            None => None,
        };

        // Merge block_time: CLI > config
        let block_time = block_time.or_else(|| anvil_config.block_time.map(Duration::from_secs));

        // Merge bool flags: CLI || config
        let no_mining = no_mining || anvil_config.no_mining;
        let mixed_mining = mixed_mining || anvil_config.mixed_mining;

        // Merge EVM options: CLI > config
        let gas_limit = evm.gas_limit.or(anvil_config.gas_limit.map(|g| g.0));
        let disable_block_gas_limit =
            evm.disable_block_gas_limit || anvil_config.disable_block_gas_limit;
        let enable_tx_gas_limit = evm.enable_tx_gas_limit || anvil_config.enable_tx_gas_limit;
        let gas_price = evm.gas_price.or(anvil_config.gas_price);
        let block_base_fee_per_gas =
            evm.block_base_fee_per_gas.or(anvil_config.block_base_fee_per_gas);
        let disable_min_priority_fee =
            evm.disable_min_priority_fee || anvil_config.disable_min_priority_fee;
        let no_storage_caching = evm.no_storage_caching || anvil_config.no_storage_caching;
        let steps_tracing = evm.steps_tracing || anvil_config.steps_tracing;
        let disable_console_log = evm.disable_console_log || anvil_config.disable_console_log;
        let print_traces = evm.print_traces || anvil_config.print_traces;
        let auto_impersonate = evm.auto_impersonate || anvil_config.auto_impersonate;
        let disable_default_create2_deployer =
            evm.disable_default_create2_deployer || anvil_config.disable_default_create2_deployer;
        let disable_pool_balance_checks =
            evm.disable_pool_balance_checks || anvil_config.disable_pool_balance_checks;
        let no_rate_limit = evm.no_rate_limit || anvil_config.no_rate_limit;

        // Merge Option<T> fields: CLI > config
        let code_size_limit = evm.code_size_limit.or(anvil_config.code_size_limit);
        let disable_code_size_limit =
            evm.disable_code_size_limit || anvil_config.disable_code_size_limit;
        let memory_limit = evm.memory_limit.or(anvil_config.memory_limit);
        let max_transactions = max_transactions.or(anvil_config.max_transactions);
        let prune_history = prune_history.or(anvil_config.prune_history);
        let max_persisted_states = max_persisted_states.or(anvil_config.max_persisted_states);
        let transaction_block_keeper =
            transaction_block_keeper.or(anvil_config.transaction_block_keeper);
        let cache_path = cache_path.or(anvil_config.cache_path);
        let ipc_merged: Option<Option<String>> =
            if ipc.is_some() { ipc } else { anvil_config.ipc.map(Some) };

        // Merge fork settings: CLI > config
        let fork_url_str = evm.fork_url.as_ref().map(|f| f.url.clone()).or(anvil_config.fork_url);
        let fork_block = evm.fork_url.as_ref().and_then(|f| f.block);
        let fork_block_number = evm.fork_block_number.or(anvil_config.fork_block_number);
        let fork_headers =
            if evm.fork_headers.is_empty() { anvil_config.fork_headers } else { evm.fork_headers };
        let fork_chain_id = evm.fork_chain_id.or(anvil_config.fork_chain_id.map(Chain::from));
        let fork_request_timeout = evm.fork_request_timeout.or(anvil_config.fork_request_timeout);
        let fork_request_retries = evm.fork_request_retries.or(anvil_config.fork_request_retries);
        let fork_retry_backoff = evm.fork_retry_backoff.or(anvil_config.fork_retry_backoff);
        let compute_units_per_second = if no_rate_limit {
            Some(u64::MAX)
        } else {
            evm.compute_units_per_second.or(anvil_config.compute_units_per_second)
        };

        Ok(NodeConfig::default()
            .with_gas_limit(gas_limit)
            .disable_block_gas_limit(disable_block_gas_limit)
            .enable_tx_gas_limit(enable_tx_gas_limit)
            .with_gas_price(gas_price)
            .with_hardfork(hardfork)
            .with_blocktime(block_time)
            .with_no_mining(no_mining)
            .with_mixed_mining(mixed_mining, block_time)
            .with_account_generator(account_generator)?
            .with_genesis_balance(genesis_balance)
            .with_genesis_timestamp(timestamp)
            .with_genesis_block_number(number)
            .with_port(port)
            .with_fork_choice(match (fork_block_number, evm.fork_transaction_hash) {
                (Some(block), None) => Some(ForkChoice::Block(block)),
                (None, Some(hash)) => Some(ForkChoice::Transaction(hash)),
                _ => fork_block.map(|num| ForkChoice::Block(num as i128)),
            })
            .with_fork_headers(fork_headers)
            .with_fork_chain_id(fork_chain_id.map(u64::from).map(U256::from))
            .fork_request_timeout(fork_request_timeout.map(Duration::from_millis))
            .fork_request_retries(fork_request_retries)
            .fork_retry_backoff(fork_retry_backoff.map(Duration::from_millis))
            .fork_compute_units_per_second(compute_units_per_second)
            .with_eth_rpc_url(fork_url_str)
            .with_base_fee(block_base_fee_per_gas)
            .disable_min_priority_fee(disable_min_priority_fee)
            .with_no_storage_caching(no_storage_caching)
            .with_server_config(server_config)
            .with_host(host)
            .set_silent(shell::is_quiet())
            .set_config_out(config_out)
            .with_chain_id(Some(merged_chain_id))
            .with_transaction_order(order)
            .with_genesis(init)
            .with_steps_tracing(steps_tracing)
            .with_print_logs(!disable_console_log)
            .with_print_traces(print_traces)
            .with_auto_impersonate(auto_impersonate)
            .with_ipc(ipc_merged)
            .with_code_size_limit(code_size_limit)
            .disable_code_size_limit(disable_code_size_limit)
            .set_pruned_history(prune_history)
            .with_init_state(load_state.or_else(|| state.and_then(|s| s.state)))
            .with_transaction_block_keeper(transaction_block_keeper)
            .with_max_transactions(max_transactions)
            .with_max_persisted_states(max_persisted_states)
            .with_networks(evm.networks)
            .with_disable_default_create2_deployer(disable_default_create2_deployer)
            .with_disable_pool_balance_checks(disable_pool_balance_checks)
            .with_slots_in_an_epoch(slots_in_an_epoch)
            .with_memory_limit(memory_limit)
            .with_cache_path(cache_path))
    }

    fn account_generator(
        &self,
        accounts: u64,
        chain_id: Chain,
        config_mnemonic: &Option<String>,
        config_derivation_path: &Option<String>,
    ) -> AccountGenerator {
        let mut generator =
            AccountGenerator::new(accounts as usize).phrase(DEFAULT_MNEMONIC).chain_id(chain_id);
        if let Some(ref mnemonic) = self.mnemonic {
            generator = generator.phrase(mnemonic);
        } else if let Some(count) = self.mnemonic_random {
            let mut rng = rand_08::thread_rng();
            let mnemonic = match Mnemonic::<English>::new_with_count(&mut rng, count) {
                Ok(mnemonic) => mnemonic.to_phrase(),
                Err(err) => {
                    warn!(target: "node", ?count, %err, "failed to generate mnemonic, falling back to 12-word random mnemonic");
                    Mnemonic::<English>::new_with_count(&mut rng, 12)
                        .expect("valid default word count")
                        .to_phrase()
                }
            };
            generator = generator.phrase(mnemonic);
        } else if let Some(seed) = self.mnemonic_seed {
            let mut seed = StdRng::seed_from_u64(seed);
            let mnemonic = Mnemonic::<English>::new(&mut seed).to_phrase();
            generator = generator.phrase(mnemonic);
        } else if let Some(mnemonic) = config_mnemonic {
            generator = generator.phrase(mnemonic);
        }
        if let Some(ref derivation) = self.derivation_path {
            generator = generator.derivation_path(derivation);
        } else if let Some(derivation) = config_derivation_path {
            generator = generator.derivation_path(derivation);
        }
        generator
    }

    /// Returns the location where to dump the state to.
    fn dump_state_path(&self) -> Option<PathBuf> {
        self.dump_state.as_ref().or_else(|| self.state.as_ref().map(|s| &s.path)).cloned()
    }

    /// Starts the node
    ///
    /// See also [crate::spawn()]
    pub async fn run(self) -> eyre::Result<()> {
        let dump_state = self.dump_state_path();
        let dump_interval =
            self.state_interval.map(Duration::from_secs).unwrap_or(DEFAULT_DUMP_INTERVAL);
        let preserve_historical_states = self.preserve_historical_states;

        let (api, mut handle) = crate::try_spawn(self.into_node_config()?).await?;

        // sets the signal handler to gracefully shutdown.
        let mut fork = api.get_fork();
        let running = Arc::new(AtomicUsize::new(0));

        // handle for the currently running rt, this must be obtained before setting the crtlc
        // handler, See [Handle::current]
        let mut signal = handle.shutdown_signal_mut().take();

        let task_manager = handle.task_manager();
        let mut on_shutdown = task_manager.on_shutdown();

        let mut state_dumper =
            PeriodicStateDumper::new(api, dump_state, dump_interval, preserve_historical_states);

        task_manager.spawn(async move {
            // wait for the SIGTERM signal on unix systems
            #[cfg(unix)]
            let mut sigterm = Box::pin(async {
                if let Ok(mut stream) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    stream.recv().await;
                } else {
                    futures::future::pending::<()>().await;
                }
            });

            // On windows, this will never fire.
            #[cfg(not(unix))]
            let mut sigterm = Box::pin(futures::future::pending::<()>());

            // await shutdown signal but also periodically flush state
            tokio::select! {
                 _ = &mut sigterm => {
                    trace!("received sigterm signal, shutting down");
                }
                _ = &mut on_shutdown => {}
                _ = &mut state_dumper => {}
            }

            // shutdown received
            state_dumper.dump().await;

            // cleaning up and shutting down
            // this will make sure that the fork RPC cache is flushed if caching is configured
            if let Some(fork) = fork.take() {
                trace!("flushing cache on shutdown");
                fork.database
                    .read()
                    .await
                    .maybe_flush_cache()
                    .expect("Could not flush cache on fork DB");
                // cleaning up and shutting down
                // this will make sure that the fork RPC cache is flushed if caching is configured
            }
            std::process::exit(0);
        });

        ctrlc::set_handler(move || {
            let prev = running.fetch_add(1, Ordering::SeqCst);
            if prev == 0 {
                trace!("received shutdown signal, shutting down");
                let _ = signal.take();
            }
        })
        .expect("Error setting Ctrl-C handler");

        Ok(handle.await??)
    }
}

/// Anvil's EVM related arguments.
#[derive(Clone, Debug, Parser)]
#[command(next_help_heading = "EVM options")]
pub struct AnvilEvmArgs {
    /// Fetch state over a remote endpoint instead of starting from an empty state.
    ///
    /// If you want to fetch state from a specific block number, add a block number like `http://localhost:8545@1400000` or use the `--fork-block-number` argument.
    #[arg(
        long,
        short,
        visible_alias = "rpc-url",
        value_name = "URL",
        help_heading = "Fork config"
    )]
    pub fork_url: Option<ForkUrl>,

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

    /// Fetch state from after a specific transaction hash has been applied over a remote endpoint.
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

    /// Enable the transaction gas limit check as imposed by EIP-7825 (Osaka hardfork).
    #[arg(long, visible_alias = "tx-gas-limit", help_heading = "Environment config")]
    pub enable_tx_gas_limit: bool,

    /// EIP-170: Contract code size limit in bytes. Useful to increase this because of tests. To
    /// disable entirely, use `--disable-code-size-limit`. By default, it is 0x6000 (~25kb).
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

    /// Disable pool balance checks
    #[arg(long)]
    pub disable_pool_balance_checks: bool,

    /// The memory limit per EVM execution in bytes.
    #[arg(long)]
    pub memory_limit: Option<u64>,

    #[command(flatten)]
    pub networks: NetworkConfigs,
}

/// Resolves an alias passed as fork-url to the matching url defined in the rpc_endpoints section
/// of the project configuration file.
/// Does nothing if the fork-url is not a configured alias.
impl AnvilEvmArgs {
    pub fn resolve_rpc_alias(&mut self) {
        if let Some(fork_url) = &self.fork_url
            && let Ok(config) = Config::load_with_providers(FigmentProviders::Anvil)
            && let Some(Ok(url)) = config.get_rpc_url_with_alias(&fork_url.url)
        {
            self.fork_url = Some(ForkUrl { url: url.to_string(), block: fork_url.block });
        }
    }
}

/// Helper type to periodically dump the state of the chain to disk
struct PeriodicStateDumper {
    in_progress_dump: Option<Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>>,
    api: EthApi,
    dump_state: Option<PathBuf>,
    preserve_historical_states: bool,
    interval: Interval,
}

impl PeriodicStateDumper {
    fn new(
        api: EthApi,
        dump_state: Option<PathBuf>,
        interval: Duration,
        preserve_historical_states: bool,
    ) -> Self {
        let dump_state = dump_state.map(|mut dump_state| {
            if dump_state.is_dir() {
                dump_state = dump_state.join("state.json");
            }
            dump_state
        });

        // periodically flush the state
        let interval = tokio::time::interval_at(Instant::now() + interval, interval);
        Self { in_progress_dump: None, api, dump_state, preserve_historical_states, interval }
    }

    async fn dump(&self) {
        if let Some(state) = self.dump_state.clone() {
            Self::dump_state(self.api.clone(), state, self.preserve_historical_states).await
        }
    }

    /// Infallible state dump
    async fn dump_state(api: EthApi, dump_state: PathBuf, preserve_historical_states: bool) {
        trace!(path=?dump_state, "Dumping state on shutdown");
        match api.serialized_state(preserve_historical_states).await {
            Ok(state) => {
                if let Err(err) = foundry_common::fs::write_json_file(&dump_state, &state) {
                    error!(?err, "Failed to dump state");
                } else {
                    trace!(path=?dump_state, "Dumped state on shutdown");
                }
            }
            Err(err) => {
                error!(?err, "Failed to extract state");
            }
        }
    }
}

// An endless future that periodically dumps the state to disk if configured.
impl Future for PeriodicStateDumper {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.dump_state.is_none() {
            return Poll::Pending;
        }

        loop {
            if let Some(mut flush) = this.in_progress_dump.take() {
                match flush.poll_unpin(cx) {
                    Poll::Ready(_) => {
                        this.interval.reset();
                    }
                    Poll::Pending => {
                        this.in_progress_dump = Some(flush);
                        return Poll::Pending;
                    }
                }
            }

            if this.interval.poll_tick(cx).is_ready() {
                let api = this.api.clone();
                let path = this.dump_state.clone().expect("exists; see above");
                this.in_progress_dump =
                    Some(Box::pin(Self::dump_state(api, path, this.preserve_historical_states)));
            } else {
                break;
            }
        }

        Poll::Pending
    }
}

/// Represents the --state flag and where to load from, or dump the state to
#[derive(Clone, Debug)]
pub struct StateFile {
    pub path: PathBuf,
    pub state: Option<SerializableState>,
}

impl StateFile {
    /// This is used as the clap `value_parser` implementation to parse from file but only if it
    /// exists
    fn parse(path: &str) -> Result<Self, String> {
        Self::parse_path(path)
    }

    /// Parse from file but only if it exists
    pub fn parse_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut path = path.as_ref().to_path_buf();
        if path.is_dir() {
            path = path.join("state.json");
        }
        let mut state = Self { path, state: None };
        if !state.path.exists() {
            return Ok(state);
        }

        state.state = Some(SerializableState::load(&state.path).map_err(|err| err.to_string())?);

        Ok(state)
    }
}

/// Represents the input URL for a fork with an optional trailing block number:
/// `http://localhost:8545@1000000`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkUrl {
    /// The endpoint url
    pub url: String,
    /// Optional trailing block
    pub block: Option<u64>,
}

impl fmt::Display for ForkUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.url.fmt(f)?;
        if let Some(block) = self.block {
            write!(f, "@{block}")?;
        }
        Ok(())
    }
}

impl FromStr for ForkUrl {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((url, block)) = s.rsplit_once('@') {
            if block == "latest" {
                return Ok(Self { url: url.to_string(), block: None });
            }
            // this will prevent false positives for auths `user:password@example.com`
            if !block.is_empty() && !block.contains(':') && !block.contains('.') {
                let block: u64 = block
                    .parse()
                    .map_err(|_| format!("Failed to parse block number: `{block}`"))?;
                return Ok(Self { url: url.to_string(), block: Some(block) });
            }
        }
        Ok(Self { url: s.to_string(), block: None })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_parse_fork_url() {
        let fork: ForkUrl = "http://localhost:8545@1000000".parse().unwrap();
        assert_eq!(
            fork,
            ForkUrl { url: "http://localhost:8545".to_string(), block: Some(1000000) }
        );

        let fork: ForkUrl = "http://localhost:8545".parse().unwrap();
        assert_eq!(fork, ForkUrl { url: "http://localhost:8545".to_string(), block: None });

        let fork: ForkUrl = "wss://user:password@example.com/".parse().unwrap();
        assert_eq!(
            fork,
            ForkUrl { url: "wss://user:password@example.com/".to_string(), block: None }
        );

        let fork: ForkUrl = "wss://user:password@example.com/@latest".parse().unwrap();
        assert_eq!(
            fork,
            ForkUrl { url: "wss://user:password@example.com/".to_string(), block: None }
        );

        let fork: ForkUrl = "wss://user:password@example.com/@100000".parse().unwrap();
        assert_eq!(
            fork,
            ForkUrl { url: "wss://user:password@example.com/".to_string(), block: Some(100000) }
        );
    }

    #[test]
    fn can_parse_ethereum_hardfork() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--hardfork", "berlin"]);
        let config = args.into_node_config().unwrap();
        assert_eq!(config.hardfork, Some(EthereumHardfork::Berlin.into()));
    }

    #[test]
    fn can_parse_optimism_hardfork() {
        let args: NodeArgs =
            NodeArgs::parse_from(["anvil", "--optimism", "--hardfork", "Regolith"]);
        let config = args.into_node_config().unwrap();
        assert_eq!(config.hardfork, Some(OpHardfork::Regolith.into()));
    }

    #[test]
    fn cant_parse_invalid_hardfork() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--hardfork", "Regolith"]);
        let config = args.into_node_config();
        assert!(config.is_err());
    }

    #[test]
    fn can_parse_fork_headers() {
        let args: NodeArgs = NodeArgs::parse_from([
            "anvil",
            "--fork-url",
            "http,://localhost:8545",
            "--fork-header",
            "User-Agent: test-agent",
            "--fork-header",
            "Referrer: example.com",
        ]);
        assert_eq!(args.evm.fork_headers, vec!["User-Agent: test-agent", "Referrer: example.com"]);
    }

    #[test]
    fn can_parse_prune_config() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--prune-history"]);
        assert!(args.prune_history.is_some());

        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--prune-history", "100"]);
        assert_eq!(args.prune_history, Some(Some(100)));
    }

    #[test]
    fn can_parse_max_persisted_states_config() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--max-persisted-states", "500"]);
        assert_eq!(args.max_persisted_states, (Some(500)));
    }

    #[test]
    fn can_parse_disable_block_gas_limit() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--disable-block-gas-limit"]);
        assert!(args.evm.disable_block_gas_limit);

        let args =
            NodeArgs::try_parse_from(["anvil", "--disable-block-gas-limit", "--gas-limit", "100"]);
        assert!(args.is_err());
    }

    #[test]
    fn can_parse_enable_tx_gas_limit() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--enable-tx-gas-limit"]);
        assert!(args.evm.enable_tx_gas_limit);

        // Also test the alias
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--tx-gas-limit"]);
        assert!(args.evm.enable_tx_gas_limit);
    }

    #[test]
    fn can_parse_disable_code_size_limit() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--disable-code-size-limit"]);
        assert!(args.evm.disable_code_size_limit);

        let args = NodeArgs::try_parse_from([
            "anvil",
            "--disable-code-size-limit",
            "--code-size-limit",
            "100",
        ]);
        // can't be used together
        assert!(args.is_err());
    }

    #[test]
    fn can_parse_host() {
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(args.host, None);

        let args = NodeArgs::parse_from([
            "anvil", "--host", "::1", "--host", "1.1.1.1", "--host", "2.2.2.2",
        ]);
        assert_eq!(
            args.host,
            Some(["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec())
        );

        let args = NodeArgs::parse_from(["anvil", "--host", "::1,1.1.1.1,2.2.2.2"]);
        assert_eq!(
            args.host,
            Some(["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec())
        );

        unsafe { env::set_var("ANVIL_IP_ADDR", "1.1.1.1") };
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(args.host, Some(vec!["1.1.1.1".parse::<IpAddr>().unwrap()]));

        unsafe { env::set_var("ANVIL_IP_ADDR", "::1,1.1.1.1,2.2.2.2") };
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(
            args.host,
            Some(["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec())
        );
    }
}
