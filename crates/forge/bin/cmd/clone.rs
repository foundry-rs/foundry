use std::path::PathBuf;

use alloy_primitives::Address;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_config::Config;

use super::init::InitArgs;

/// CLI arguments for `forge clone`.
#[derive(Clone, Debug, Parser)]
pub struct CloneArgs {
    /// The contract address to clone.
    address: String,

    /// The root directory of the cloned project.
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    #[command(flatten)]
    etherscan: EtherscanOpts,
}

impl CloneArgs {
    pub async fn run(self) -> Result<()> {
        let CloneArgs { address, root, etherscan } = self;

        // parse the contract address
        let contract_address: Address = address.parse()?;

        // get the chain and api key from the config
        let config = Config::from(&etherscan);
        let chain = config.chain.unwrap_or_default();
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();

        // get the contract code
        let client = Client::new(chain, etherscan_api_key)?;
        let mut meta = client.contract_source_code(contract_address).await?;
        if meta.items.len() != 1 {
            return Err(eyre::eyre!("contract not found or ill-formed"));
        }
        let meta = meta.items.remove(0);

        // let's try to init the project with default init args
        let init_args = InitArgs { root: root.clone(), vscode: true, ..Default::default() };
        init_args.run().map_err(|_| eyre::eyre!("Cannot run `clone` on a non-empty directory."))?;

        // canonicalize the root path
        // note that at this point, root must be created
        let root = dunce::canonicalize(root)?;

        // remove the unnecessary example contracts
        // XXX (ZZ): this is a temporary solution until we have a proper way to remove contracts,
        // e.g., add a field in the config to control the example contract generatoin
        fs::remove_file(root.join("src/Counter.sol"))?;
        fs::remove_file(root.join("test/Counter.t.sol"))?;
        fs::remove_file(root.join("script/Counter.s.sol"))?;

        // update configuration
        Config::update_at(root, |config, doc| {
            true
        });

        Ok(())
    }
}
