use crate::{
    config::{Hardfork, DEFAULT_MNEMONIC},
    eth::pool::transactions::TransactionOrder,
    AccountGenerator, NodeConfig, CHAIN_ID,
};
use anvil_server::ServerConfig;
use clap::Parser;
use core::fmt;
use ethers::utils::WEI_IN_ETHER;
use foundry_config::Chain;
use std::{
    net::IpAddr,
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::runtime::Handle;
use tracing::log::trace;

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
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

    #[clap(flatten, next_help_heading = "SERVER OPTIONS")]
    pub server_config: ServerConfig,

    #[clap(long, help = "Don't print anything on startup.")]
    pub silent: bool,

    #[clap(long, help = "The EVM hardfork to use.", value_name = "HARDFORK")]
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
        help_heading = "SERVER OPTIONS"
    )]
    pub host: Option<IpAddr>,

    #[clap(
        long,
        help = "How transactions are sorted in the mempool",
        default_value = "fees",
        value_name = "ORDER"
    )]
    pub order: TransactionOrder,
}

impl NodeArgs {
    pub fn into_node_config(self) -> NodeConfig {
        let genesis_balance = WEI_IN_ETHER.saturating_mul(self.balance.into());

        NodeConfig::default()
            .with_gas_limit(self.evm_opts.gas_limit)
            .with_gas_price(self.evm_opts.gas_price)
            .with_hardfork(self.hardfork)
            .with_blocktime(self.block_time.map(std::time::Duration::from_secs))
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
            .with_eth_rpc_url(self.evm_opts.fork_url.map(|fork| fork.url))
            .with_base_fee(self.evm_opts.block_base_fee_per_gas)
            .with_storage_caching(self.evm_opts.no_storage_caching)
            .with_server_config(self.server_config)
            .with_host(self.host)
            .set_silent(self.silent)
            .set_config_out(self.config_out)
            .with_chain_id(self.evm_opts.chain_id)
            .with_transaction_order(self.order)
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

    /// Starts the node
    ///
    /// See also [crate::spawn()]
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let (api, handle) = crate::spawn(self.into_node_config()).await;

        // sets the signal handler to gracefully shutdown.
        let mut fork = api.get_fork().cloned();
        let running = Arc::new(AtomicUsize::new(0));

        // handle for the currently running rt, this must be obtained before setting the crtlc
        // handler, See [Handle::current]
        let rt_handle = Handle::current();

        ctrlc::set_handler(move || {
            let prev = running.fetch_add(1, Ordering::SeqCst);
            if prev == 0 {
                // cleaning up and shutting down
                // this will make sure that the fork RPC cache is flushed if caching is configured
                trace!("received shutdown signal, shutting down");
                if let Some(fork) = fork.take() {
                    rt_handle.block_on(async move {
                        fork.database.read().await.flush_cache();
                    });
                }
                std::process::exit(0);
            }
        })
        .expect("Error setting Ctrl-C handler");

        Ok(handle.await??)
    }
}

// Anvil's evm related arguments
#[derive(Debug, Clone, Parser)]
pub struct AnvilEvmArgs {
    /// Fetch state over a remote endpoint instead of starting from an empty state.
    ///
    /// If you want to fetch state from a specific block number, add a block number like `http://localhost:8545@1400000` or use the `--fork-block-number` argument.
    #[clap(
        long,
        short,
        visible_alias = "rpc-url",
        value_name = "URL",
        help_heading = "FORK CONFIG"
    )]
    pub fork_url: Option<ForkUrl>,

    /// Fetch state from a specific block number over a remote endpoint.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork-url", value_name = "BLOCK", help_heading = "FORK CONFIG")]
    pub fork_block_number: Option<u64>,

    /// Initial retry backoff on encountering errors.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork-url", value_name = "BACKOFF", help_heading = "FORK CONFIG")]
    pub fork_retry_backoff: Option<u64>,

    /// Explicitly disables the use of RPC caching.
    ///
    /// All storage slots are read entirely from the endpoint.
    ///
    /// This flag overrides the project's configuration file.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork-url", help_heading = "FORK CONFIG")]
    pub no_storage_caching: bool,

    /// The block gas limit.
    #[clap(long, value_name = "GAS_LIMIT", help_heading = "ENVIRONMENT CONFIG")]
    pub gas_limit: Option<u64>,

    /// The gas price.
    #[clap(long, value_name = "GAS_PRICE", help_heading = "ENVIRONMENT CONFIG")]
    pub gas_price: Option<u64>,

    /// The base fee in a block.
    #[clap(
        long,
        visible_alias = "base-fee",
        value_name = "FEE",
        help_heading = "ENVIRONMENT CONFIG"
    )]
    pub block_base_fee_per_gas: Option<u64>,

    /// The chain ID.
    #[clap(long, alias = "chain", value_name = "CHAIN_ID", help_heading = "ENVIRONMENT CONFIG")]
    pub chain_id: Option<Chain>,
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
}
