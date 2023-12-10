use cast::{AbiPath, SimpleCast};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_config::Config;
use itertools::Itertools;
use std::path::{Path, PathBuf};

/// CLI arguments for `cast interface`.
#[derive(Debug, Clone, Parser)]
pub struct InterfaceArgs {
    /// The contract address, or the path to an ABI file.
    ///
    /// If an address is specified, then the ABI is fetched from Etherscan.
    path_or_address: String,

    /// The name to use for the generated interface.
    #[clap(long, short)]
    name: Option<String>,

    /// Solidity pragma version.
    #[clap(long, short, default_value = "^0.8.4", value_name = "VERSION")]
    pragma: String,

    /// The path to the output file.
    ///
    /// If not specified, the interface will be output to stdout.
    #[clap(
        short,
        long,
        value_hint = clap::ValueHint::FilePath,
        value_name = "PATH",
    )]
    output: Option<PathBuf>,

    /// If specified, the interface will be output as JSON rather than Solidity.
    #[clap(long, short)]
    json: bool,

    #[clap(flatten)]
    etherscan: EtherscanOpts,
}

impl InterfaceArgs {
    pub async fn run(self) -> Result<()> {
        let InterfaceArgs {
            path_or_address,
            name,
            pragma,
            output: output_location,
            etherscan,
            json,
        } = self;
        let source = if Path::new(&path_or_address).exists() {
            AbiPath::Local { path: path_or_address, name }
        } else {
            let config = Config::from(&etherscan);
            let chain = config.chain.unwrap_or_default();
            let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
            AbiPath::Etherscan {
                chain,
                api_key,
                address: path_or_address.parse().wrap_err("invalid path or address")?,
            }
        };

        let interfaces = SimpleCast::generate_interface(source).await?;

        // put it all together
        let res = if json {
            interfaces.iter().map(|iface| &iface.json_abi).format("\n").to_string()
        } else {
            format!(
                "// SPDX-License-Identifier: UNLICENSED\n\
                 pragma solidity {pragma};\n\n\
                 {}",
                interfaces.iter().map(|iface| &iface.source).format("\n")
            )
        };

        // print or write to file
        if let Some(loc) = output_location {
            if let Some(parent) = loc.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&loc, res)?;
            println!("Saved interface at {}", loc.display());
        } else {
            print!("{res}");
        }
        Ok(())
    }
}
