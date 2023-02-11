use crate::{
    config::DEFAULT_MNEMONIC,
    eth::{backend::db::SerializableState, pool::transactions::TransactionOrder, EthApi},
    genesis::Genesis,
    AccountGenerator, Hardfork, NodeConfig, CHAIN_ID,
};
use anvil_server::ServerConfig;
use clap::Parser;
use core::fmt;
use ethers::utils::WEI_IN_ETHER;
use foundry_config::Chain;
use futures::FutureExt;
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
    #[clap(flatten)]
    pub evm_opts: AnvilEvmArgs,

    #[clap(
        long,
        short,
        help = "Port number to listen on.",
        default_value = "8545",
        value_name = "NUM"
    )]
    pub port: u16,

    #[clap(
        long,
        short,
        help = "Number of dev accounts to generate and configure.",
        default_value = "10",
        value_name = "NUM"
    )]
    pub accounts: u64,

    #[clap(
        long,
        help = "The balance of every dev account in Ether.",
        default_value = "10000",
        value_name = "NUM"
    )]
    pub balance: u64,

    #[clap(long, help = "The timestamp of the genesis block", value_name = "NUM")]
    pub timestamp: Option<u64>,

    #[clap(
        long,
        short,
        help = "BIP39 mnemonic phrase used for generating accounts",
        value_name = "MNEMONIC"
    )]
    pub mnemonic: Option<String>,

    #[clap(
        long,
        help = "Sets the derivation path of the child key to be derived. [default: m/44'/60'/0'/0/]",
        value_name = "DERIVATION_PATH"
    )]
    pub derivation_path: Option<String>,

    #[clap(flatten)]
    pub server_config: ServerConfig,

    #[clap(long, help = "Don't print anything on startup.")]
    pub silent: bool,

    #[clap(long, help = "The EVM hardfork to use.", value_name = "HARDFORK", value_parser = Hardfork::from_str)]
    pub hardfork: Option<Hardfork>,

    #[clap(
        short,
        long,
        visible_alias = "blockTime",
        help = "Block time in seconds for interval mining.",
        name = "block-time",
        value_name = "SECONDS"
    )]
    pub block_time: Option<u64>,

    #[clap(
        long,
        help = "Writes output of `anvil` as json to user-specified file",
        value_name = "OUT_FILE"
    )]
    pub config_out: Option<String>,

    #[clap(
        long,
        visible_alias = "no-mine",
        help = "Disable auto and interval mining, and mine on demand instead.",
        conflicts_with = "block-time"
    )]
    pub no_mining: bool,

    #[clap(
        long,
        help = "The host the server will listen on",
        value_name = "IP_ADDR",
        env = "ANVIL_IP_ADDR",
        help_heading = "Server options"
    )]
    pub host: Option<IpAddr>,

    #[clap(
        long,
        help = "How transactions are sorted in the mempool",
        default_value = "fees",
        value_name = "ORDER"
    )]
    pub order: TransactionOrder,

    #[clap(
        long,
        help = "Initialize the genesis block with the given `genesis.json` file.",
        value_name = "PATH",
        value_parser = Genesis::parse
    )]
    pub init: Option<Genesis>,

    #[clap(
        long,
        help = "This is an alias for bot --load-state and --dump-state. It initializes the chain with the state stored at the file, if it exists, and dumps the chain's state on exit",
        value_name = "PATH",
        value_parser = StateFile::parse,
        conflicts_with_all = &["init", "dump_state", "load_state"]
    )]
    pub state: Option<StateFile>,

    #[clap(
        short,
        long,
        help = "Interval in seconds at which the status is to be dumped to disk. See --state and --dump-state",
        value_name = "SECONDS"
    )]
    pub state_interval: Option<u64>,

    #[clap(
        long,
        help = "Dump the state of chain on exit to the given file. If the value is a directory, the state will be written to `<VALUE>/state.json`.",
        value_name = "PATH",
        conflicts_with = "init"
    )]
    pub dump_state: Option<PathBuf>,

    #[clap(
        long,
        help = "Initialize the chain from a previously saved state snapshot.",
        value_name = "PATH",
        value_parser = SerializableState::parse,
        conflicts_with = "init"
    )]
    pub load_state: Option<SerializableState>,

    #[clap(
        long,
        help = IPC_HELP,
        value_name = "PATH",
        visible_alias = "ipcpath"
    )]
    pub ipc: Option<Option<String>>,

    #[clap(
        long,
        help = "Don't keep full chain history. If a number argument is specified, at most this number of states is kept in memory."
    )]
    pub prune_history: Option<Option<usize>>,

    #[clap(long, help = "Number of blocks with transactions to keep in memory.")]
    pub transaction_block_keeper: Option<usize>,
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
            .with_fork_chain_id(self.evm_opts.fork_chain_id)
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
        let mut fork = api.get_fork().cloned();
        let running = Arc::new(AtomicUsize::new(0));

        // handle for the currently running rt, this must be obtained before setting the crtlc
        // handler, See [Handle::current]
        let mut signal = handle.shutdown_signal_mut().take();

        let task_manager = handle.task_manager();
        let mut on_shutdown = task_manager.on_shutdown();

        let mut state_dumper = PeriodicStateDumper::new(api, dump_state, dump_interval);

        task_manager.spawn(async move {
            // await shutdown signal but also periodically flush state
            tokio::select! {
                _ = &mut on_shutdown =>{}
                _ = &mut state_dumper =>{}
            }

            // shutdown received
            state_dumper.dump().await;

            // cleaning up and shutting down
            // this will make sure that the fork RPC cache is flushed if caching is configured
            if let Some(fork) = fork.take() {
                trace!("flushing cache on shutdown");
                fork.database.read().await.flush_cache();
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
        help = "Disables rate limiting for this node provider.",
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
    #[clap(long, value_name = "GAS_LIMIT", help_heading = "Environment config")]
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
    #[clap(long, value_name = "GAS_PRICE", help_heading = "Environment config")]
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
    #[clap(long, alias = "chain", value_name = "CHAIN_ID", help_heading = "Environment config")]
    pub chain_id: Option<Chain>,

    #[clap(
        long,
        help = "Enable steps tracing used for debug calls returning geth-style traces",
        visible_alias = "tracing"
    )]
    pub steps_tracing: bool,
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
                this.in_progress_dump =
                    Some(Box::pin(async move { PeriodicStateDumper::dump_state(api, path).await }));
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
}
