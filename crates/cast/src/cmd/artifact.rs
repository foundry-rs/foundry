use super::creation_code::{fetch_creation_code, load_abi, parse_code_output};
use alloy_primitives::Address;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::LoadConfig,
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

    /// Find the creation code using only the RPC, without Etherscan.
    ///
    /// Requires `--abi-path` and an archive node. Contracts deployed by other contracts also
    /// require `trace_block` support.
    #[arg(long, requires = "abi_path")]
    rpc_only: bool,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl ArtifactArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;
        let Self { contract, output, abi_path, rpc_only, .. } = self;

        let bytecode = fetch_creation_code(&mut config, contract, rpc_only).await?;
        let abi_path = abi_path.as_deref();
        let abi = load_abi(contract, &config, abi_path).await?;
        let bytecode =
            parse_code_output(bytecode, contract, &config, abi_path, true, false).await?;

        let artifact = json!({ "abi": abi, "bytecode": { "object": bytecode } });
        let artifact = serde_json::to_string_pretty(&artifact)?;

        if let Some(loc) = output {
            if let Some(parent) = loc.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&loc, artifact)?;
            sh_status!("Saved artifact at {}", loc.display())?;
        } else {
            sh_println!("{artifact}")?;
        }
        Ok(())
    }
}
