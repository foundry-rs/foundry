use crate::{cmd::Cmd, opts::ClapChain};
use cast::AbiPath;
use clap::{Parser, ValueHint};
use ethers::prelude::{errors::EtherscanError, Abigen, Client, MultiAbigen};
use forge::Address;
use foundry_common::fs::json_files;
use futures::future::BoxFuture;
use std::path::{Path, PathBuf};

static DEFAULT_CRATE_NAME: &str = "foundry-contracts";
static DEFAULT_CRATE_VERSION: &str = "0.0.1";

/// CLI arguments for `cast bind`.
#[derive(Debug, Clone, Parser)]
pub struct BindArgs {
    #[clap(
        help = "The contract address, or the path to an ABI Directory.",
        long_help = r#"The contract address, or the path to an ABI Directory.

If an address is specified, then the ABI is fetched from Etherscan."#,
        value_name = "PATH_OR_ADDRESS"
    )]
    path_or_address: String,

    #[clap(long, short, env = "ETHERSCAN_API_KEY", help = "etherscan API key", value_name = "KEY")]
    etherscan_api_key: Option<String>,
    #[clap(
        help = "Path to where bindings will be stored",
        long = "output-dir",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    pub output_dir: Option<PathBuf>,

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

    #[clap(long = "seperate-files", help = "Generate bindings as seperate files.")]
    seperate_files: bool,

    #[clap(flatten)]
    chain: ClapChain,
}

impl Cmd for BindArgs {
    type Output = BoxFuture<'static, eyre::Result<()>>;

    fn run(self) -> eyre::Result<Self::Output> {
        let cmd = Box::pin(async move {
            let bind_args = self.clone();
            if Path::new(&self.path_or_address).exists() {
                self.generate_bindings(AbiPath::Local {
                    path: bind_args.path_or_address,
                    name: None,
                })
                .await
            } else {
                self.generate_bindings(AbiPath::Etherscan {
                    address: bind_args.path_or_address.parse::<Address>().unwrap(),
                    chain: bind_args.chain.inner,
                    api_key: bind_args.etherscan_api_key.unwrap(),
                })
                .await
            }
        });

        Ok(cmd)
    }
}

impl BindArgs {
    pub async fn generate_bindings(&self, address_or_path: AbiPath) -> eyre::Result<()> {
        match address_or_path {
            AbiPath::Etherscan { address, chain, api_key } => {
                let client = Client::new(chain, api_key)?;
                let metadata = &client.contract_source_code(address).await?.items[0];
                let address = if metadata.implementation.is_some() {
                    metadata.implementation.unwrap()
                } else {
                    address
                };
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
                    .map(|item| Abigen::new(item.contract_name.clone(), item.abi.clone()).unwrap())
                    .collect::<Vec<Abigen>>();

                let multi = MultiAbigen::from_abigens(abigens);

                let bindings = multi.build().unwrap();
                println!("Generating bindings for {} contracts", bindings.len());
                bindings.write_to_crate(
                    &self.crate_name,
                    &self.crate_version,
                    self.get_binding_root(),
                    !self.seperate_files,
                )?;
            }
            AbiPath::Local { path, name: _ } => {
                let abis = json_files(Path::new(&path))
                    .into_iter()
                    .filter_map(|path| {
                        let stem = path.file_stem()?;
                        if stem.to_str()?.ends_with(".metadata") {
                            None
                        } else {
                            Some(path)
                        }
                    })
                    .map(Abigen::from_file)
                    .collect::<Result<Vec<_>, _>>()?;
                let multi = MultiAbigen::from_abigens(abis);

                let bindings = multi.build().unwrap();
                println!("Generating bindings for {} contracts", bindings.len());
                bindings.write_to_crate(
                    &self.crate_name,
                    &self.crate_version,
                    self.get_binding_root(),
                    !self.seperate_files,
                )?;
            }
        };
        Ok(())
    }

    fn get_binding_root(&self) -> PathBuf {
        self.output_dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap().join("bindings"))
    }
}
