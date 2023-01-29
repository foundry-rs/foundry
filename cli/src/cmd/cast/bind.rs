#![allow(unused_variables, dead_code)]
use std::path::PathBuf;

use cast::InterfacePath;
use clap::{Parser, ValueHint};
use ethers::prelude::{errors::EtherscanError, Abigen, Client, MultiAbigen};
use forge::Address;
use futures::future::BoxFuture;

use crate::{cmd::Cmd, opts::ClapChain};

static DEFAULT_CRATE_NAME: &str = "foundry-contracts";
static DEFAULT_CRATE_VERSION: &str = "0.0.1";

#[derive(Debug, Clone, Parser)]
pub struct BindArgs {
    #[clap(
        help = "The contract address, or the path to an ABI file.",
        long_help = r#"The contract address, or the path to an ABI file.

If an address is specified, then the ABI is fetched from Etherscan."#,
        value_name = "PATH_OR_ADDRESS"
    )]
    path_or_address: String,

    #[clap(long, short, env = "ETHERSCAN_API_KEY", help = "etherscan API key", value_name = "KEY")]
    etherscan_api_key: Option<String>,

    #[clap(
        help = "Path to where the contract artifacts are stored",
        long = "bindings-path",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    pub bindings: Option<PathBuf>,
    #[clap(
        long = "crate-name",
        help = "The name of the Rust crate to generate. This should be a valid crates.io crate name. However, it is not currently validated by this command.",
        default_value = DEFAULT_CRATE_NAME,
    )]
    crate_name: String,

    #[clap(
        long = "crate-version",
        help = "The version of the Rust crate to generate. This should be a standard semver version string. However, it is not currently validated by this command.",
        default_value = DEFAULT_CRATE_VERSION,
        value_name = "NAME"
    )]
    crate_version: String,

    #[clap(flatten)]
    chain: ClapChain,
}

impl Cmd for BindArgs {
    type Output = BoxFuture<'static, eyre::Result<()>>;

    fn run(self) -> eyre::Result<Self::Output> {
        let cmd = Box::pin(async move {
            let BindArgs {
                path_or_address,
                etherscan_api_key,
                bindings,
                crate_name,
                crate_version,
                chain,
            } = self.clone();

            self.generate_bindings(InterfacePath::Etherscan {
                address: path_or_address.parse::<Address>().unwrap(),
                chain: chain.inner,
                api_key: etherscan_api_key.unwrap(),
            })
            .await
        });

        Ok(cmd)
    }
}

impl BindArgs {
    pub async fn generate_bindings(self, address_or_path: InterfacePath) -> eyre::Result<()> {
        match address_or_path {
            InterfacePath::Etherscan { address, chain, api_key } => {
                let client = Client::new(chain, api_key)?;
                // get the source
                let source = match client.contract_source_code(address).await {
                    Ok(source) => source,
                    Err(EtherscanError::InvalidApiKey) => {
                        eyre::bail!("Invalid Etherscan API key. Did you set it correctly? You may be using an API key for another Etherscan API chain (e.g. Etherscan API key for Polygonscan).")
                    }
                    Err(EtherscanError::ContractCodeNotVerified(address)) => {
                        eyre::bail!("Contract source code at {:?} on {} not verified. Maybe you have selected the wrong chain?", address, chain)
                    }
                    Err(err) => {
                        eyre::bail!(err)
                    }
                };
                let abigens = source
                    .items
                    .iter()
                    .map(|item| Abigen::new(&item.contract_name.clone(), item.abi.clone()).unwrap())
                    .collect::<Vec<Abigen>>();

                let multi = MultiAbigen::from_abigens(abigens);
                // Abigen::new(contract_name, abi_source)

                let bindings = multi.build().unwrap();
                bindings.write_to_crate(
                    &self.crate_name,
                    &self.crate_version,
                    self.get_binding_root(),
                    false,
                )?;
            }
            InterfacePath::Local { path, name } => {
                todo!()
            }
        };
        Ok(())
    }

    fn get_binding_root(&self) -> PathBuf {
        self.bindings.clone().unwrap_or_else(|| std::env::current_dir().unwrap().join("bindings"))
    }
}
