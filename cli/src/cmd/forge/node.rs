use clap::Parser;
use ethers::core::types::U256;

use crate::opts::evm::EvmArgs;
use anvil::{AccountGenerator, NodeConfig};

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

    #[clap(long, help = "the balance of every genesis account, defaults to 100ETH")]
    pub balance: Option<U256>,

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
        let NodeConfig { chain_id, gas_limit, gas_price, genesis_balance, .. } =
            NodeConfig::default();

        NodeConfig::default()
            .chain_id(self.evm_opts.env.chain_id.unwrap_or(chain_id))
            .gas_limit(self.evm_opts.env.gas_limit.unwrap_or(gas_limit.as_u64()))
            .gas_price(self.evm_opts.env.gas_price.unwrap_or(gas_price.as_u64()))
            .account_generator(self.account_generator())
            .genesis_balance(self.balance.unwrap_or(genesis_balance))
            .port(self.port)
            .eth_rpc_url(self.evm_opts.fork_url)
            .fork_block_number(self.evm_opts.fork_block_number)
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
    pub async fn run(self) -> eyre::Result<()> {
        let (_api, handle) = anvil::spawn(self.into_node_config()).await;

        Ok(handle.await??)
    }
}
