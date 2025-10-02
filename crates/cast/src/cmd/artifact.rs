use super::{
    creation_code::{fetch_creation_code_from_etherscan, parse_code_output},
    interface::load_abi_from_file,
};
use alloy_primitives::Address;
use alloy_provider::Provider;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig, fetch_abi_from_etherscan},
};
use foundry_common::fs;
use serde_json::json;
use std::path::PathBuf;

foundry_config::impl_figment_convert!(ArtifactArgs, etherscan, rpc);

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
        let mut config = self.load_config()?;

        let Self { contract, output: output_location, abi_path, etherscan: _, rpc: _ } = self;

        let provider = utils::get_provider(&config)?;
        let chain = provider.get_chain_id().await?;
        config.chain = Some(chain.into());

        let abi = if let Some(ref abi_path) = abi_path {
            load_abi_from_file(abi_path, None)?
        } else {
            fetch_abi_from_etherscan(contract, &config).await?
        };

        let (abi, _) = abi.first().ok_or_else(|| eyre::eyre!("No ABI found"))?;

        let bytecode = fetch_creation_code_from_etherscan(contract, &config, provider).await?;
        let bytecode =
            parse_code_output(bytecode, contract, &config, abi_path.as_deref(), true, false)
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
