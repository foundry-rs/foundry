//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use cast::SimpleCast;
use ethers::{
    abi::{Contract, Function, FunctionExt},
    prelude::{
        artifacts::{BytecodeObject, Source, Sources},
        ContractFactory, Http, Middleware, MinimalCombinedArtifacts, Project, ProjectCompileOutput,
        Provider, Signer, SignerMiddleware,
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
        // Find Project & Compile
        let project = self.opts.project()?;
        let compiled = project.compile()?;

        if self.verify && self.contract.path.is_none() {
            eyre::bail!("verifying requires giving out the source path");
        }

        if compiled.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(compiled.to_string())
        }

        // Get ABI and BIN
        let (abi, bin) = match self.contract.path {
            Some(ref path) => self.get_artifact_from_path(&project, path.clone())?,
            None => self.get_artifact_from_name(compiled)?,
        };

        // Add arguments to constructor
        let provider = Provider::<Http>::try_from(self.eth.rpc_url.as_str())?;
        let constructor_with_args = self.build_constructor_with_args(abi.clone())?;

        // Deploy with signer
        let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
        let chain_id = rt.block_on(provider.get_chainid())?;
        if let Some(signer) = rt.block_on(self.eth.signer_with(chain_id, provider))? {
            match signer {
                WalletType::Ledger(signer) => {
                    rt.block_on(self.deploy(abi, bin, constructor_with_args, signer))?;
                }
                WalletType::Local(signer) => {
                    rt.block_on(self.deploy(abi, bin, constructor_with_args, signer))?;
                }
                WalletType::Trezor(signer) => {
                    rt.block_on(self.deploy(abi, bin, constructor_with_args, signer))?;
                }
            }
        } else {
            eyre::bail!("could not find artifact")
        }

        Ok(())
    }
}

impl CreateArgs {
    /// Find using only ContractName
    fn get_artifact_from_name(
        &self,
        compiled: ProjectCompileOutput<MinimalCombinedArtifacts>,
    ) -> Result<(Contract, BytecodeObject)> {
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

        Ok(match contract_artifact {
            Some(artifact) => (
                artifact.abi.ok_or_else(|| eyre::Error::msg("message"))?,
                artifact.bin.ok_or_else(|| eyre::Error::msg("message"))?,
            ),
            None => {
                eyre::bail!("could not find artifact")
            }
        })
    }

    /// Find using src/ContractSource.sol:ContractName
    fn get_artifact_from_path(
        &self,
        project: &Project,
        path: String,
    ) -> Result<(Contract, BytecodeObject)> {
        // Get sources from the requested location
        let abs_path = std::fs::canonicalize(PathBuf::from(path))?;
        let mut sources = Sources::new();
        sources.insert(abs_path.clone(), Source::read(&abs_path)?);

        // Get artifact from the contract name and sources
        let mut config = SolFilesCache::builder().insert_files(sources.clone(), None)?;
        config.files.entry(abs_path).and_modify(|f| f.artifacts = vec![self.contract.name.clone()]);

        // let binding
        let _artifacts =
            config.read_artifacts::<MinimalCombinedArtifacts>(project.artifacts_path())?;

        let artifacts = _artifacts.values().collect::<Vec<_>>();

        if artifacts.is_empty() {
            eyre::bail!("could not find artifact")
        } else if artifacts.len() > 1 {
            eyre::bail!("duplicate contract name in the same source file")
        }

        Ok((
            artifacts[0].clone().abi.ok_or_else(|| eyre::Error::msg("message"))?,
            artifacts[0].clone().bin.ok_or_else(|| eyre::Error::msg("message"))?,
        ))
    }

    async fn deploy(
        self,
        abi: Contract,
        bin: BytecodeObject,
        args: Option<String>,
        signer: SignerMiddleware<Provider<Http>, impl Signer + 'static>,
    ) -> Result<()> {
        let deployer_address = signer.address();
        let factory = ContractFactory::new(abi, bin.as_bytes().unwrap().clone(), Arc::new(signer));

        let deployer = match args {
            Some(args) => factory.deploy(args)?,
            None => factory.deploy(())?,
        };

        let deployed_contract = deployer.send().await?;

        println!("Deployer: {:?}", deployer_address);
        println!("Deployed to: {:?}", deployed_contract.address());

        Ok(())
    }

    fn build_constructor_with_args(&self, abi: Contract) -> Result<Option<String>> {
        if let Some(constructor) = abi.constructor {
            // convert constructor into function
            #[allow(deprecated)]
            let fun = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs,
                outputs: vec![],
                constant: false,
                state_mutability: Default::default(),
            };

            Ok(Some(SimpleCast::calldata(fun.abi_signature(), &self.constructor_args)?))
        } else if !self.constructor_args.is_empty() {
            eyre::bail!("No constructor found but contract arguments provided")
        } else {
            Ok(None)
        }
    }
}
