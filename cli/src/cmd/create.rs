//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use cast::SimpleCast;
use ethers::{
    abi::{Function, FunctionExt},
    prelude::{
        artifacts::{Source, Sources},
        ContractFactory, Http, Middleware, MinimalCombinedArtifacts, Provider, Signer,
        SignerMiddleware,
    },
    solc::cache::SolFilesCache,
};
use eyre::Result;

use crate::opts::forge::ContractInfo;
use std::{path::PathBuf, sync::Arc};
use structopt::StructOpt;

#[derive(Debug, Clone, StructOpt)]
pub struct CreateArgs {
    #[structopt(long, help = "constructor args calldata arguments.")]
    constructor_args: Vec<String>,

    #[structopt(flatten)]
    opts: BuildArgs,

    #[structopt(flatten)]
    eth: EthereumOpts,

    #[structopt(help = "contract source info `<path>:<contractname>` or `<contractname>`")]
    contract: ContractInfo,

    #[structopt(long, help = "verify on Etherscan")]
    verify: bool,
}

impl Cmd for CreateArgs {
    type Output = ();
    fn run(self) -> Result<Self::Output> {
        /*
         * Read contract name
         */
        let project = self.opts.project()?;
        let compiled = project.compile()?;

        if compiled.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(compiled.to_string())
        }

        let (abi, bin) = match self.contract.path {
            Some(path) => {
                // Get requested artifact location
                let abs_path = std::fs::canonicalize(PathBuf::from(path))?;
                let mut sources = Sources::new();
                sources.insert(abs_path.clone(), Source::read(&abs_path)?);

                // Get artifact
                let mut config = SolFilesCache::builder().insert_files(sources.clone(), None)?;
                config.files.entry(abs_path).and_modify(|f| f.artifacts = vec![self.contract.name]);

                // Get Bytecode from the only existing artififact
                let artifact = config
                    .read_artifacts::<MinimalCombinedArtifacts>(project.artifacts_path())?
                    .values()
                    .collect::<Vec<_>>()[0]
                    .clone();

                (
                    artifact.abi.ok_or(eyre::Error::msg("message"))?,
                    artifact.bin.ok_or(eyre::Error::msg("message"))?,
                )
            }
            None => {
                // Find using only the contract name

                let mut has_found_contract = false;
                let mut contract_artifact = None;

                for (name, artifact) in compiled.into_artifacts() {
                    let artifact_contract_name = name.split(':').collect::<Vec<_>>()[1];

                    if artifact_contract_name == self.contract.name {
                        if has_found_contract {
                            eyre::bail!("contract with duplicate name. pass path")
                        }
                        has_found_contract = true;
                        contract_artifact = Some(artifact);
                    }
                }

                match contract_artifact {
                    Some(artifact) => (
                        artifact.abi.ok_or(eyre::Error::msg("message"))?,
                        artifact.bin.ok_or(eyre::Error::msg("message"))?,
                    ),
                    None => {
                        eyre::bail!("could not find artifact")
                    }
                }
            }
        };
        let provider = Provider::<Http>::try_from(self.eth.rpc_url.as_str())?;

        let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
        let chain_id = rt.block_on(provider.get_chainid())?;
        let mut args = None;
        if let Some(constructor) = abi.clone().constructor {
            // convert constructor into function
            #[allow(deprecated)]
            let fun = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs,
                outputs: vec![],
                constant: false,
                state_mutability: Default::default(),
            };

            args = Some(SimpleCast::calldata(fun.abi_signature(), &self.constructor_args)?);
        } else if !self.constructor_args.is_empty() {
            eyre::bail!("No constructor found but contract arguments provided")
        }

        if let Some(signer) = rt.block_on(self.eth.signer_with(chain_id, provider))? {
            match signer {
                WalletType::Ledger(signer) => {
                    println!("ASDAS");

                    println!("address {:?}", format!("0x{}", signer.default_sender().unwrap()));
                    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
                    let arc_signer = Arc::new(signer);
                    let factory =
                        ContractFactory::new(abi, bin.as_bytes().unwrap().clone(), arc_signer);

                    let deployer = match args {
                        Some(args) => factory.deploy(args)?,
                        None => factory.deploy(())?,
                    };

                    println!("{:?}", rt.block_on(deployer.send()).unwrap().address());
                    // println!("{:?}", rt.block_on(deployer.call()).unwrap());
                }
                WalletType::Local(signer) => {
                    deploy(abi, bin, args, signer)?;
                }
                WalletType::Trezor(signer) => {
                    println!("address {:?}", format!("0x{}", signer.default_sender().unwrap()));
                    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
                    let arc_signer = Arc::new(signer);
                    let _arc_signer = Arc::clone(&arc_signer);
                    let factory =
                        ContractFactory::new(abi, bin.as_bytes().unwrap().clone(), arc_signer);

                    let deployer = match args {
                        Some(args) => factory.deploy(args)?,
                        None => factory.deploy(())?,
                    };

                    println!("address {:?}", format!("0x{}", Arc::clone(&_arc_signer).address()));

                    println!("{:?}", rt.block_on(deployer.legacy().send()).unwrap().address());
                }
            }
        } else {
            eyre::bail!("could not find artifact")
        }

        Ok(())
    }
}

fn deploy(
    abi: ethers::abi::Contract,
    bin: ethers::prelude::artifacts::BytecodeObject,
    args: Option<String>,
    signer: SignerMiddleware<Provider<Http>, impl Signer + 'static>,
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
    let arc_signer = Arc::new(signer);
    let factory = ContractFactory::new(abi, bin.as_bytes().unwrap().clone(), arc_signer);

    let deployer = match args {
        Some(args) => factory.deploy(args)?,
        None => factory.deploy(())?,
    };

    println!("{:?}", rt.block_on(deployer.send()).unwrap().address());

    Ok(())
}
