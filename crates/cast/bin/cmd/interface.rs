use cast::{AbiPath, SimpleCast};
use clap::Parser;
use eyre::Result;
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_config::Config;
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
    #[clap(long, short, default_value = "^0.8.10", value_name = "VERSION")]
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
        let config = Config::from(&etherscan);
        let chain = config.chain_id.unwrap_or_default();
        let source = if Path::new(&path_or_address).exists() {
            AbiPath::Local { path: path_or_address, name }
        } else {
            let api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();
            let chain = chain.named()?;
            AbiPath::Etherscan { chain, api_key, address: path_or_address.parse()? }
        };
        let interfaces = SimpleCast::generate_interface(source).await?;

        // put it all together
        let res = if json {
            interfaces.into_iter().map(|iface| iface.json_abi).collect::<Vec<_>>().join("\n")
        } else {
            let pragma = format!("pragma solidity {pragma};");
            let interfaces = interfaces
                .iter()
                .map(|iface| iface.source.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            format!("{pragma}\n\n{interfaces}")
        };

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
