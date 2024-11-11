use alloy_primitives::Address;
use alloy_provider::Provider;
use clap::{command, Parser};
use eyre::Result;
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils,
};
use foundry_common::fs;
use foundry_config::Config;
use serde_json::json;
use std::path::PathBuf;

use super::{
    creation_code::{fetch_creation_code, parse_code_output},
    interface::{fetch_abi_from_etherscan, load_abi_from_file},
};

/// CLI arguments for `cast artifact`.
#[derive(Parser)]
pub struct ArtifactArgs {
    /// An Ethereum address, for which the artifact will be produced.
    contract: Address,

    /// Path to file containing the contract's JSON ABI. It's necessary if the target contract is
    /// not verified on Etherscan.
    #[arg(long)]
    abi_path: Option<String>,

    /// The path to the output file.
    ///
    /// If not specified, the artifact will be output to stdout.
    #[arg(
        short,
        long,
        value_hint = clap::ValueHint::FilePath,
        value_name = "PATH",
    )]
    output: Option<PathBuf>,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl ArtifactArgs {
    pub async fn run(self) -> Result<()> {
        let Self { contract, etherscan, rpc, output: output_location, abi_path } = self;

        let mut etherscan = etherscan;
        let config = Config::from(&rpc);
        let provider = utils::get_provider(&config)?;
        let api_key = etherscan.key().unwrap_or_default();
        let chain = provider.get_chain_id().await?;
        etherscan.chain = Some(chain.into());
        let client = Client::new(chain.into(), api_key)?;

        let abi = if let Some(ref abi_path) = abi_path {
            load_abi_from_file(abi_path, None)?
        } else {
            fetch_abi_from_etherscan(contract, &etherscan).await?
        };

        let (abi, _) = abi.first().ok_or_else(|| eyre::eyre!("No ABI found"))?;

        let bytecode = fetch_creation_code(contract, client, provider).await?;
        let bytecode =
            parse_code_output(bytecode, contract, &etherscan, abi_path.as_deref(), true, false)
                .await?;

        let artifact = json!({
            "abi": abi,
            "bytecode": {
              "object": bytecode
            }
        });

        let artifact = serde_json::to_string_pretty(&artifact)?;

        if let Some(loc) = output_location {
            if let Some(parent) = loc.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&loc, artifact)?;
            sh_println!("Saved artifact at {}", loc.display())?;
        } else {
            sh_println!("{artifact}")?;
        }

        Ok(())
    }
}
