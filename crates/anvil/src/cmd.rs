use crate::{
    config::DEFAULT_MNEMONIC,
    eth::{backend::db::SerializableState, pool::transactions::TransactionOrder, EthApi},
    genesis::Genesis,
    AccountGenerator, Hardfork, NodeConfig, CHAIN_ID,
};
use anvil_server::ServerConfig;
use clap::Parser;
use core::fmt;
use ethers::{
    signers::coins_bip39::{English, Mnemonic},
    utils::WEI_IN_ETHER,
};
use foundry_config::{Chain, Config};
use futures::FutureExt;
use rand::{rngs::StdRng, SeedableRng};
use std::{
    future::Future,
    net::IpAddr,
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Instant, Interval};
use tracing::{error, trace};

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    /// Port number to listen on.
    #[clap(long, short, default_value = "8545", value_name = "NUM")]
    pub port: u16,

    /// Number of dev accounts to generate and configure.
    #[clap(long, short, default_value = "10", value_name = "NUM")]
    pub accounts: u64,

    /// The balance of every dev account in Ether.
    #[clap(long, default_value = "10000", value_name = "NUM")]
    pub balance: u64,

    /// The timestamp of the genesis block.
    #[clap(long, value_name = "NUM")]
    pub timestamp: Option<u64>,

    /// BIP39 mnemonic phrase used for generating accounts.
    /// Cannot be used if `mnemonic_random` or `mnemonic_seed` are used
    #[clap(long, short, conflicts_with_all = &["mnemonic_seed", "mnemonic_random"])]
    pub mnemonic: Option<String>,

    /// Automatically generates a BIP39 mnemonic phrase, and derives accounts from it.
    /// Cannot be used with other `mnemonic` options
    /// You can specify the number of words you want in the mnemonic.
    /// [default: 12]
    #[clap(long, conflicts_with_all = &["mnemonic", "mnemonic_seed"], default_missing_value = "12", num_args(0..=1))]
    pub mnemonic_random: Option<usize>,

    /// Generates a BIP39 mnemonic phrase from a given seed
    /// Cannot be used with other `mnemonic` options
    ///
    /// CAREFUL: this is NOT SAFE and should only be used for testing.
    /// Never use the private keys generated in production.
    #[clap(long = "mnemonic-seed-unsafe", conflicts_with_all = &["mnemonic", "mnemonic_random"])]
    pub mnemonic_seed: Option<u64>,

    /// Sets the derivation path of the child key to be derived.
    ///
    /// [default: m/44'/60'/0'/0/]
    #[clap(long)]
    pub derivation_path: Option<String>,

    /// Don't print anything on startup and don't print logs
    #[clap(long)]
    pub silent: bool,

    /// The EVM hardfork to use.
    ///
    /// Choose the hardfork by name, e.g. `shanghai`, `paris`, `london`, etc...
    /// [default: latest]
    #[clap(long, value_parser = Hardfork::from_str)]
    pub hardfork: Option<Hardfork>,

    /// Block time in seconds for interval mining.
    #[clap(short, long, visible_alias = "blockTime", name = "block-time", value_name = "SECONDS")]
    pub block_time: Option<u64>,

    /// Writes output of `anvil` as json to user-specified file.
    #[clap(long, value_name = "OUT_FILE")]
    pub config_out: Option<String>,

    /// Disable auto and interval mining, and mine on demand instead.
    #[clap(long, visible_alias = "no-mine", conflicts_with = "block-time")]
    pub no_mining: bool,

    /// The hosts the server will listen on.
    #[clap(
        long,
        value_name = "IP_ADDR",
        env = "ANVIL_IP_ADDR",
        default_value = "127.0.0.1",
        help_heading = "Server options",
        value_delimiter = ','
    )]
    pub host: Vec<IpAddr>,

    /// How transactions are sorted in the mempool.
    #[clap(long, default_value = "fees")]
    pub order: TransactionOrder,

    /// Initialize the genesis block with the given `genesis.json` file.
    #[clap(long, value_name = "PATH", value_parser = Genesis::parse)]
    pub init: Option<Genesis>,

    /// This is an alias for both --load-state and --dump-state.
    ///
    /// It initializes the chain with the state stored at the file, if it exists, and dumps the
    /// chain's state on exit.
    #[clap(
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

    /// Interval in seconds at which the status is to be dumped to disk.
    ///
    /// See --state and --dump-state
    #[clap(short, long, value_name = "SECONDS")]
    pub state_interval: Option<u64>,

    /// Dump the state of chain on exit to the given file.
    ///
    /// If the value is a directory, the state will be written to `<VALUE>/state.json`.
    #[clap(long, value_name = "PATH", conflicts_with = "init")]
    pub dump_state: Option<PathBuf>,

    /// Initialize the chain from a previously saved state snapshot.
    #[clap(
        long,
        value_name = "PATH",
        value_parser = SerializableState::parse,
        conflicts_with = "init"
    )]
    pub load_state: Option<SerializableState>,

    #[clap(long, help = IPC_HELP, value_name = "PATH", visible_alias = "ipcpath")]
    pub ipc: Option<Option<String>>,

    /// Don't keep full chain history.
    /// If a number argument is specified, at most this number of states is kept in memory.
    #[clap(long)]
    pub prune_history: Option<Option<usize>>,

    /// Number of blocks with transactions to keep in memory.
    #[clap(long)]
    pub transaction_block_keeper: Option<usize>,

    #[clap(flatten)]
    pub evm_opts: AnvilEvmArgs,

    #[clap(flatten)]
    pub server_config: ServerConfig,
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
    pub fn into_node_config(self) -> NodeConfig {
        let genesis_balance = WEI_IN_ETHER.saturating_mul(self.balance.into());
        let compute_units_per_second = if self.evm_opts.no_rate_limit {
            Some(u64::MAX)
        } else {
            self.evm_opts.compute_units_per_second
        };

        NodeConfig::default()
            .with_gas_limit(self.evm_opts.gas_limit)
            .disable_block_gas_limit(self.evm_opts.disable_block_gas_limit)
            .with_gas_price(self.evm_opts.gas_price)
            .with_hardfork(self.hardfork)
            .with_blocktime(self.block_time.map(Duration::from_secs))
            .with_no_mining(self.no_mining)
            .with_account_generator(self.account_generator())
            .with_genesis_balance(genesis_balance)
            .with_genesis_timestamp(self.timestamp)
            .with_port(self.port)
            .with_fork_block_number(
                self.evm_opts
                    .fork_block_number
                    .or_else(|| self.evm_opts.fork_url.as_ref().and_then(|f| f.block)),
            )
            .with_fork_headers(self.evm_opts.fork_headers)
            .with_fork_chain_id(self.evm_opts.fork_chain_id.map(u64::from))
            .fork_request_timeout(self.evm_opts.fork_request_timeout.map(Duration::from_millis))
            .fork_request_retries(self.evm_opts.fork_request_retries)
            .fork_retry_backoff(self.evm_opts.fork_retry_backoff.map(Duration::from_millis))
            .fork_compute_units_per_second(compute_units_per_second)
            .with_eth_rpc_url(self.evm_opts.fork_url.map(|fork| fork.url))
            .with_base_fee(self.evm_opts.block_base_fee_per_gas)
            .with_storage_caching(self.evm_opts.no_storage_caching)
            .with_server_config(self.server_config)
            .with_host(self.host)
            .set_silent(self.silent)
            .set_config_out(self.config_out)
            .with_chain_id(self.evm_opts.chain_id)
            .with_transaction_order(self.order)
            .with_genesis(self.init)
            .with_steps_tracing(self.evm_opts.steps_tracing)
            .with_auto_impersonate(self.evm_opts.auto_impersonate)
            .with_ipc(self.ipc)
            .with_code_size_limit(self.evm_opts.code_size_limit)
            .set_pruned_history(self.prune_history)
            .with_init_state(self.load_state.or_else(|| self.state.and_then(|s| s.state)))
            .with_transaction_block_keeper(self.transaction_block_keeper)
    }

    fn account_generator(&self) -> AccountGenerator {
        let mut gen = AccountGenerator::new(self.accounts as usize)
            .phrase(DEFAULT_MNEMONIC)
            .chain_id(self.evm_opts.chain_id.unwrap_or_else(|| CHAIN_ID.into()));
        if let Some(ref mnemonic) = self.mnemonic {
            gen = gen.phrase(mnemonic);
        } else if let Some(count) = self.mnemonic_random {
            let mut rng = rand::thread_rng();
            let mnemonic = match Mnemonic::<English>::new_with_count(&mut rng, count) {
                Ok(mnemonic) => mnemonic.to_phrase(),
                Err(_) => DEFAULT_MNEMONIC.to_string(),
            };
            gen = gen.phrase(mnemonic);
        } else if let Some(seed) = self.mnemonic_seed {
            let mut seed = StdRng::seed_from_u64(seed);
            let mnemonic = Mnemonic::<English>::new(&mut seed).to_phrase();
            gen = gen.phrase(mnemonic);
        }
        if let Some(ref derivation) = self.derivation_path {
            gen = gen.derivation_path(derivation);
        }
        gen
    }

    /// Returns the location where to dump the state to.
    fn dump_state_path(&self) -> Option<PathBuf> {
        self.dump_state.as_ref().or_else(|| self.state.as_ref().map(|s| &s.path)).cloned()
    }

    /// Starts the node
    ///
    /// See also [crate::spawn()]
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let dump_state = self.dump_state_path();
        let dump_interval =
            self.state_interval.map(Duration::from_secs).unwrap_or(DEFAULT_DUMP_INTERVAL);

        let (api, mut handle) = crate::spawn(self.into_node_config()).await;

        // sets the signal handler to gracefully shutdown.
        let mut fork = api.get_fork();
        let running = Arc::new(AtomicUsize::new(0));

        // handle for the currently running rt, this must be obtained before setting the crtlc
        // handler, See [Handle::current]
        let mut signal = handle.shutdown_signal_mut().take();

        let task_manager = handle.task_manager();
        let mut on_shutdown = task_manager.on_shutdown();

        let mut state_dumper = PeriodicStateDumper::new(api, dump_state, dump_interval);

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

            // on windows, this will never fires
            #[cfg(not(unix))]
            let mut sigterm = Box::pin(futures::future::pending::<()>());

            // await shutdown signal but also periodically flush state
            tokio::select! {
                 _ = &mut sigterm => {
                    trace!("received sigterm signal, shutting down");
                },
                _ = &mut on_shutdown =>{

                }
                _ = &mut state_dumper =>{}
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
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "EVM options")]
pub struct AnvilEvmArgs {
    /// Fetch state over a remote endpoint instead of starting from an empty state.
    ///
    /// If you want to fetch state from a specific block number, add a block number like `http://localhost:8545@1400000` or use the `--fork-block-number` argument.
    #[clap(
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
    #[clap(
        long = "fork-header",
        value_name = "HEADERS",
        help_heading = "Fork config",
        requires = "fork_url"
    )]
    pub fork_headers: Vec<String>,

    /// Timeout in ms for requests sent to remote JSON-RPC server in forking mode.
    ///
    /// Default value 45000
    #[clap(
        long = "timeout",
        name = "timeout",
        help_heading = "Fork config",
        requires = "fork_url"
    )]
    pub fork_request_timeout: Option<u64>,

    /// Number of retry requests for spurious networks (timed out requests)
    ///
    /// Default value 5
    #[clap(
        long = "retries",
        name = "retries",
        help_heading = "Fork config",
        requires = "fork_url"
    )]
    pub fork_request_retries: Option<u32>,

    /// Fetch state from a specific block number over a remote endpoint.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork_url", value_name = "BLOCK", help_heading = "Fork config")]
    pub fork_block_number: Option<u64>,

    /// Initial retry backoff on encountering errors.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork_url", value_name = "BACKOFF", help_heading = "Fork config")]
    pub fork_retry_backoff: Option<u64>,

    /// Specify chain id to skip fetching it from remote endpoint. This enables offline-start mode.
    ///
    /// You still must pass both `--fork-url` and `--fork-block-number`, and already have your
    /// required state cached on disk, anything missing locally would be fetched from the
    /// remote.
    #[clap(
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
    /// See --fork-url.
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(
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
    /// See --fork-url.
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(
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
    #[clap(long, requires = "fork_url", help_heading = "Fork config")]
    pub no_storage_caching: bool,

    /// The block gas limit.
    #[clap(long, alias = "block-gas-limit", help_heading = "Environment config")]
    pub gas_limit: Option<u64>,

    /// Disable the `call.gas_limit <= block.gas_limit` constraint.
    #[clap(
        long,
        value_name = "DISABLE_GAS_LIMIT",
        help_heading = "Environment config",
        alias = "disable-gas-limit",
        conflicts_with = "gas_limit"
    )]
    pub disable_block_gas_limit: bool,

    /// EIP-170: Contract code size limit in bytes. Useful to increase this because of tests. By
    /// default, it is 0x6000 (~25kb).
    #[clap(long, value_name = "CODE_SIZE", help_heading = "Environment config")]
    pub code_size_limit: Option<usize>,

    /// The gas price.
    #[clap(long, help_heading = "Environment config")]
    pub gas_price: Option<u64>,

    /// The base fee in a block.
    #[clap(
        long,
        visible_alias = "base-fee",
        value_name = "FEE",
        help_heading = "Environment config"
    )]
    pub block_base_fee_per_gas: Option<u64>,

    /// The chain ID.
    #[clap(long, alias = "chain", help_heading = "Environment config")]
    pub chain_id: Option<Chain>,

    /// Enable steps tracing used for debug calls returning geth-style traces
    #[clap(long, visible_alias = "tracing")]
    pub steps_tracing: bool,

    /// Enable autoImpersonate on startup
    #[clap(long, visible_alias = "auto-impersonate")]
    pub auto_impersonate: bool,
}

/// Resolves an alias passed as fork-url to the matching url defined in the rpc_endpoints section
/// of the project configuration file.
/// Does nothing if the fork-url is not a configured alias.
impl AnvilEvmArgs {
    pub fn resolve_rpc_alias(&mut self) {
        if let Some(fork_url) = &self.fork_url {
            let config = Config::load();
            if let Some(Ok(url)) = config.get_rpc_url_with_alias(&fork_url.url) {
                self.fork_url = Some(ForkUrl { url: url.to_string(), block: fork_url.block });
            }
        }
    }
}

/// Helper type to periodically dump the state of the chain to disk
struct PeriodicStateDumper {
    in_progress_dump: Option<Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>>,
    api: EthApi,
    dump_state: Option<PathBuf>,
    interval: Interval,
}

impl PeriodicStateDumper {
    fn new(api: EthApi, dump_state: Option<PathBuf>, interval: Duration) -> Self {
        let dump_state = dump_state.map(|mut dump_state| {
            if dump_state.is_dir() {
                dump_state = dump_state.join("state.json");
            }
            dump_state
        });

        // periodically flush the state
        let interval = tokio::time::interval_at(Instant::now() + interval, interval);
        Self { in_progress_dump: None, api, dump_state, interval }
    }

    async fn dump(&self) {
        if let Some(state) = self.dump_state.clone() {
            Self::dump_state(self.api.clone(), state).await
        }
    }

    /// Infallible state dump
    async fn dump_state(api: EthApi, dump_state: PathBuf) {
        trace!(path=?dump_state, "Dumping state on shutdown");
        match api.serialized_state().await {
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
            return Poll::Pending
        }

        loop {
            if let Some(mut flush) = this.in_progress_dump.take() {
                match flush.poll_unpin(cx) {
                    Poll::Ready(_) => {
                        this.interval.reset();
                    }
                    Poll::Pending => {
                        this.in_progress_dump = Some(flush);
                        return Poll::Pending
                    }
                }
            }

            if this.interval.poll_tick(cx).is_ready() {
                let api = this.api.clone();
                let path = this.dump_state.clone().expect("exists; see above");
                this.in_progress_dump = Some(Box::pin(PeriodicStateDumper::dump_state(api, path)));
            } else {
                break
            }
        }

        Poll::Pending
    }
}

/// Represents the --state flag and where to load from, or dump the state to
#[derive(Debug, Clone)]
pub struct StateFile {
    pub path: PathBuf,
    pub state: Option<SerializableState>,
}

impl StateFile {
    /// This is used as the clap `value_parser` implementation to parse from file but only if it
    /// exists
    fn parse(path: &str) -> Result<Self, String> {
        let mut path = PathBuf::from(path);
        if path.is_dir() {
            path = path.join("state.json");
        }
        let mut state = Self { path, state: None };
        if !state.path.exists() {
            return Ok(state)
        }

        state.state = Some(SerializableState::load(&state.path).map_err(|err| err.to_string())?);

        Ok(state)
    }
}

/// Represents the input URL for a fork with an optional trailing block number:
/// `http://localhost:8545@1000000`
#[derive(Debug, Clone, PartialEq, Eq)]
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
                return Ok(ForkUrl { url: url.to_string(), block: None })
            }
            // this will prevent false positives for auths `user:password@example.com`
            if !block.is_empty() && !block.contains(':') && !block.contains('.') {
                let block: u64 = block
                    .parse()
                    .map_err(|_| format!("Failed to parse block number: `{block}`"))?;
                return Ok(ForkUrl { url: url.to_string(), block: Some(block) })
            }
        }
        Ok(ForkUrl { url: s.to_string(), block: None })
    }
}

#[cfg(test)]
mod tests {
    use std::{env, net::Ipv4Addr};

    use super::*;

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
    fn can_parse_hardfork() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--hardfork", "berlin"]);
        assert_eq!(args.hardfork, Some(Hardfork::Berlin));
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
        assert_eq!(
            args.evm_opts.fork_headers,
            vec!["User-Agent: test-agent", "Referrer: example.com"]
        );
    }

    #[test]
    fn can_parse_prune_config() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--prune-history"]);
        assert!(args.prune_history.is_some());

        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--prune-history", "100"]);
        assert_eq!(args.prune_history, Some(Some(100)));
    }

    #[test]
    fn can_parse_disable_block_gas_limit() {
        let args: NodeArgs = NodeArgs::parse_from(["anvil", "--disable-block-gas-limit"]);
        assert!(args.evm_opts.disable_block_gas_limit);

        let args =
            NodeArgs::try_parse_from(["anvil", "--disable-block-gas-limit", "--gas-limit", "100"]);
        assert!(args.is_err());
    }

    #[test]
    fn can_parse_host() {
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(args.host, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);

        let args = NodeArgs::parse_from([
            "anvil", "--host", "::1", "--host", "1.1.1.1", "--host", "2.2.2.2",
        ]);
        assert_eq!(
            args.host,
            ["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec()
        );

        let args = NodeArgs::parse_from(["anvil", "--host", "::1,1.1.1.1,2.2.2.2"]);
        assert_eq!(
            args.host,
            ["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec()
        );

        env::set_var("ANVIL_IP_ADDR", "1.1.1.1");
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(args.host, vec!["1.1.1.1".parse::<IpAddr>().unwrap()]);

        env::set_var("ANVIL_IP_ADDR", "::1,1.1.1.1,2.2.2.2");
        let args = NodeArgs::parse_from(["anvil"]);
        assert_eq!(
            args.host,
            ["::1", "1.1.1.1", "2.2.2.2"].map(|ip| ip.parse::<IpAddr>().unwrap()).to_vec()
        );
    }
}
