//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use ethers::{
    abi::{Constructor, Contract, Token},
    prelude::{
        artifacts::{BytecodeObject, Source, Sources},
        ContractFactory, Http, Middleware, MinimalCombinedArtifacts, Project, ProjectCompileOutput,
        Provider,
    },
    solc::cache::SolFilesCache,
};
use eyre::Result;
use foundry_utils::parse_tokens;

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
        println!("compiling...");
        let compiled = project.compile()?;

        if self.verify && self.contract.path.is_none() {
            eyre::bail!("verifying requires giving out the source path");
        }

        if compiled.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(compiled.to_string())
        } else if compiled.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        }

        // Get ABI and BIN
        let (abi, bin) = match self.contract.path {
            Some(ref path) => self.get_artifact_from_path(&project, path.clone())?,
            None => self.get_artifact_from_name(compiled)?,
        };

        // Add arguments to constructor
        let provider = Provider::<Http>::try_from(self.eth.rpc_url.as_str())?;
        let params = match abi.constructor {
            Some(ref v) => self.parse_constructor_args(v)?,
            None => vec![],
        };

        // Deploy with signer
        let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
        let chain_id = rt.block_on(provider.get_chainid())?;
        if let Some(signer) = rt.block_on(self.eth.signer_with(chain_id, provider))? {
            match signer {
                WalletType::Ledger(signer) => {
                    rt.block_on(self.deploy(abi, bin, params, signer))?;
                }
                WalletType::Local(signer) => {
                    rt.block_on(self.deploy(abi, bin, params, signer))?;
                }
                WalletType::Trezor(signer) => {
                    rt.block_on(self.deploy(abi, bin, params, signer))?;
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
    // TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
    // contract name?
    fn get_artifact_from_name(
        &self,
        compiled: ProjectCompileOutput<MinimalCombinedArtifacts>,
    ) -> Result<(Contract, BytecodeObject)> {
        let mut has_found_contract = false;
        let mut contract_artifact = None;

        for (name, artifact) in compiled.into_artifacts() {
            // if the contract name
            let mut split = name.split(':');
            let mut artifact_contract_name =
                split.next().ok_or_else(|| eyre::Error::msg("no contract name provided"))?;
            if let Some(new_name) = split.next() {
                artifact_contract_name = new_name;
            };

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
                artifact.abi.ok_or_else(|| {
                    eyre::Error::msg(format!("abi not found for {}", self.contract.name))
                })?,
                artifact.bin.ok_or_else(|| {
                    eyre::Error::msg(format!("bytecode not found for {}", self.contract.name))
                })?,
            ),
            None => {
                eyre::bail!("could not find artifact")
            }
        })
    }

    /// Find using src/ContractSource.sol:ContractName
    // TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
    // path?
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

        let artifacts = config
            .read_artifacts::<MinimalCombinedArtifacts>(project.artifacts_path())?
            .into_values()
            .collect::<Vec<_>>();

        if artifacts.is_empty() {
            eyre::bail!("could not find artifact")
        } else if artifacts.len() > 1 {
            eyre::bail!("duplicate contract name in the same source file")
        }
        let artifact = artifacts[0].clone();

        Ok((
            artifact.abi.ok_or_else(|| {
                eyre::Error::msg(format!("abi not found for {}", self.contract.name))
            })?,
            artifact.bin.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", self.contract.name))
            })?,
        ))
    }

    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Contract,
        bin: BytecodeObject,
        args: Vec<Token>,
        provider: M,
    ) -> Result<()> {
        let deployer_address =
            provider.default_sender().expect("no sender address set for provider");
        let bin = bin.into_bytes().unwrap_or_else(|| {
            panic!("no bytecode found in bin object for {}", self.contract.name)
        });
        let factory = ContractFactory::new(abi, bin, Arc::new(provider));

        let deployer = factory.deploy_tokens(args)?;
        let deployed_contract = deployer.send().await?;

        println!("Deployer: {:?}", deployer_address);
        println!("Deployed to: {:?}", deployed_contract.address());

        Ok(())
    }

    fn parse_constructor_args(&self, constructor: &Constructor) -> Result<Vec<Token>> {
        let params = constructor
            .inputs
            .iter()
            .zip(&self.constructor_args)
            .map(|(input, arg)| (&input.kind, arg.as_str()))
            .collect::<Vec<_>>();

        parse_tokens(params, true)
    }
}
