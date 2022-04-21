use clap::Parser;
use ethers::utils::WEI_IN_ETHER;

use crate::{AccountGenerator, NodeConfig, CHAIN_ID};
use forge::executor::opts::EvmOpts;
use foundry_common::evm::EvmArgs;

#[derive(Clone, Debug, Parser)]
pub struct NodeArgs {
    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

    #[clap(long, short, help = "Port number to listen on", default_value = "8545")]
    pub port: u16,

    #[clap(
        long,
        short,
        help = "Number of genesis dev accounts to generate and configure",
        default_value = "10"
    )]
    pub accounts: u64,

    #[clap(
        long,
        help = "the balance of every genesis account in Ether, defaults to 100ETH",
        default_value = "100"
    )]
    pub balance: u64,

    #[clap(long, short, help = "bip39 mnemonic phrase used for generating accounts")]
    pub mnemonic: Option<String>,

    #[clap(
        long,
        help = "Sets the derivation path of the child key to be derived [default: m/44'/60'/0'/0/]"
    )]
    pub derivation_path: Option<String>,
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
            .chain_id(evm_opts.env.chain_id.unwrap_or(CHAIN_ID))
            .gas_limit(self.evm_opts.env.gas_limit)
            .gas_price(self.evm_opts.env.gas_price)
            .account_generator(self.account_generator())
            .genesis_balance(genesis_balance)
            .port(self.port)
            .eth_rpc_url(evm_opts.fork_url)
            .base_fee(self.evm_opts.env.block_base_fee_per_gas)
            .fork_block_number(evm_opts.fork_block_number)
    }

    fn account_generator(&self) -> AccountGenerator {
        let mut gen = AccountGenerator::new(self.accounts as usize);
        if let Some(ref mnemonic) = self.mnemonic {
            gen = gen.phrase(mnemonic);
        }
        if let Some(ref derivation) = self.derivation_path {
            gen = gen.derivation_path(derivation);
        }
        gen
    }

    /// Starts the node
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let (_api, handle) = crate::spawn(self.into_node_config()).await;

        Ok(handle.await??)
    }
}
