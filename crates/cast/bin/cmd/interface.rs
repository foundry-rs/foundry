use cast::{AbiPath, SimpleCast};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_config::Config;
use itertools::Itertools;
use std::path::{Path, PathBuf};

fn find_file_in_folders(artifact_path: &Path, filename: &str) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(artifact_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    // Recursively search inside the subdirectory
                    if let Some(found_path) = find_file_in_folders(&path, filename) {
                        return Some(found_path);
                    }
                } else if path.is_file() {
                    // Check if the file name matches the given filename
                    if let Some(file_name) = path.file_stem() {
                        if file_name == filename {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    None
}

/// CLI arguments for `cast interface`.
#[derive(Clone, Debug, Parser)]
pub struct InterfaceArgs {
    /// The contract address, or the path to an ABI file.
    ///
    /// If an address is specified, then the ABI is fetched from Etherscan.
    path_or_address: String,

    /// The name to use for the generated interface.
    #[arg(long, short)]
    name: Option<String>,

    /// Solidity pragma version.
    #[arg(long, short, default_value = "^0.8.4", value_name = "VERSION")]
    pragma: String,

    /// The path to the output file.
    ///
    /// If not specified, the interface will be output to stdout.
    #[arg(
        short,
        long,
        value_hint = clap::ValueHint::FilePath,
        value_name = "PATH",
    )]
    output: Option<PathBuf>,

    /// If specified, the interface will be output as JSON rather than Solidity.
    #[arg(long, short)]
    json: bool,

    #[command(flatten)]
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
            let config = Config::load();
            // Search the artifacts folder for the abi file
            if let Some(found_path) = find_file_in_folders(&config.out, &path_or_address) {
                AbiPath::Local { path: found_path.into_os_string().into_string().unwrap(), name: Some("I".to_string() + &path_or_address) }
            } else {
                config = Config::from(&etherscan);
                let chain = config.chain.unwrap_or_default();
                let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
                AbiPath::Etherscan { chain, api_key, address: path_or_address.parse().wrap_err("invalid path or address")? }
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
