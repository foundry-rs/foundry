use crate::opts::ClapChain;
use cast::{AbiPath, SimpleCast};
use clap::Parser;
use ethers::types::Address;
use eyre::WrapErr;
use foundry_common::fs;
use foundry_config::Config;
use std::path::{Path, PathBuf};

/// CLI arguments for `cast interface`.
#[derive(Debug, Clone, Parser)]
pub struct InterfaceArgs {
    #[clap(
        help = "The contract address, or the path to an ABI file.",
        long_help = r#"The contract address, or the path to an ABI file.

If an address is specified, then the ABI is fetched from Etherscan."#,
        value_name = "PATH_OR_ADDRESS"
    )]
    path_or_address: String,
    #[clap(long, short, help = "The name to use for the generated interface", value_name = "NAME")]
    name: Option<String>,
    #[clap(
        long,
        short,
        default_value = "^0.8.10",
        help = "Solidity pragma version.",
        value_name = "VERSION"
    )]
    pragma: String,
    #[clap(
        short,
        help = "The path to the output file.",
        long_help = "The path to the output file. If not specified, the interface will be output to stdout.",
        value_name = "PATH"
    )]
    output_location: Option<PathBuf>,
    #[clap(long, short, env = "ETHERSCAN_API_KEY", help = "etherscan API key", value_name = "KEY")]
    etherscan_api_key: Option<String>,
    #[clap(flatten)]
    chain: ClapChain,
}

impl InterfaceArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let InterfaceArgs {
            path_or_address,
            name,
            pragma,
            output_location,
            etherscan_api_key,
            chain,
        } = self;
        let interfaces = if Path::new(&path_or_address).exists() {
            SimpleCast::generate_interface(AbiPath::Local { path: path_or_address, name }).await?
        } else {
            let api_key = etherscan_api_key.or_else(|| {
                    let config = Config::load();
                    config.get_etherscan_api_key(Some(chain.inner))
                }).ok_or_else(|| eyre::eyre!("No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY env var, or setting the -e CLI argument or etherscan-api-key in foundry.toml"))?;

            SimpleCast::generate_interface(AbiPath::Etherscan {
                chain: chain.inner,
                api_key,
                address: path_or_address
                    .parse::<Address>()
                    .wrap_err("Invalid address provided. Did you make a typo?")?,
            })
            .await?
        };

        // put it all together
        let pragma = format!("pragma solidity {pragma};");
        let interfaces =
            interfaces.iter().map(|iface| iface.source.to_string()).collect::<Vec<_>>().join("\n");
        let res = format!("{pragma}\n\n{interfaces}");

        // print or write to file
        if let Some(loc) = output_location {
            fs::create_dir_all(loc.parent().unwrap())?;
            fs::write(&loc, res)?;
            println!("Saved interface at {}", loc.display());
        } else {
            println!("{res}");
        }
        Ok(())
    }
}
