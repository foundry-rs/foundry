use anvil_server::ServerConfig;
use clap::Parser;
use ethers::utils::WEI_IN_ETHER;
use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tracing::log::trace;

use crate::{
    config::{Hardfork, DEFAULT_MNEMONIC},
    eth::pool::transactions::TransactionOrder,
    AccountGenerator, NodeConfig, CHAIN_ID,
};
use forge::executor::opts::EvmOpts;
use foundry_common::evm::EvmArgs;

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

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

    #[clap(
        long,
        help = "The EVM hardfork to use.",
        default_value = "latest",
        value_name = "HARDFORK"
    )]
    pub hardfork: Hardfork,

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
        visible_alias = "no-mine",
        help = "Disable auto and interval mining, and mine on demand instead.",
        conflicts_with = "block-time"
    )]
    pub no_mining: bool,

    #[cfg_attr(
        feature = "clap",
        clap(long, help = "The host the server will listen on", value_name = "IP_ADDR")
    )]
    pub host: Option<IpAddr>,

    #[cfg_attr(
        feature = "clap",
        clap(
            long,
            help = "How transactions are sorted in the mempool",
            default_value = "fees",
            value_name = "ORDER"
        )
    )]
    pub order: TransactionOrder,
}

impl NodeArgs {
    pub fn into_node_config(self) -> NodeConfig {
        let figment = foundry_config::Config::figment_with_root(
            foundry_config::find_project_root_path().unwrap(),
        )
        .merge(&self.evm_opts);
        let evm_opts = figment.extract::<EvmOpts>().expect("EvmOpts are subset");
        let genesis_balance = WEI_IN_ETHER.saturating_mul(self.balance.into());

        NodeConfig::default()
            .with_gas_limit(self.evm_opts.env.gas_limit)
            .with_gas_price(self.evm_opts.env.gas_price)
            .with_hardfork(self.hardfork)
            .with_blocktime(self.block_time.map(std::time::Duration::from_secs))
            .with_no_mining(self.no_mining)
            .with_account_generator(self.account_generator())
            .with_genesis_balance(genesis_balance)
            .with_port(self.port)
            .with_eth_rpc_url(evm_opts.fork_url)
            .with_base_fee(self.evm_opts.env.block_base_fee_per_gas)
            .with_fork_block_number(evm_opts.fork_block_number)
            .with_storage_caching(evm_opts.no_storage_caching)
            .with_server_config(self.server_config)
            .with_host(self.host)
            .set_silent(self.silent)
            .with_chain_id(self.evm_opts.env.chain_id.unwrap_or(CHAIN_ID))
            .with_transaction_order(self.order)
    }

    fn account_generator(&self) -> AccountGenerator {
        let mut gen = AccountGenerator::new(self.accounts as usize)
            .phrase(DEFAULT_MNEMONIC)
            .chain_id(self.evm_opts.env.chain_id.unwrap_or(CHAIN_ID));
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
        let fork = api.get_fork().cloned();
        let running = Arc::new(AtomicUsize::new(0));

        ctrlc::set_handler(move || {
            let prev = running.fetch_add(1, Ordering::SeqCst);
            if prev == 0 {
                // cleaning up and shutting down
                // this will make sure that the fork RPC cache is flushed if caching is configured
                trace!("received shutdown signal, shutting down");
                if let Some(ref fork) = fork {
                    fork.database.read().flush_cache();
                }
                std::process::exit(0);
            }
        })
        .expect("Error setting Ctrl-C handler");

        Ok(handle.await??)
    }
}
