//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use ethers::{
    abi::{Abi, Constructor, Token},
    prelude::{artifacts::BytecodeObject, ContractFactory, Http, Middleware, Provider},
    types::Chain,
};

use eyre::Result;
use foundry_utils::parse_tokens;
use std::fs;

use crate::opts::forge::ContractInfo;
use clap::{Parser, ValueHint};
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Parser)]
pub struct CreateArgs {
    #[clap(
        long,
        multiple_values = true,
        help = "constructor args calldata arguments",
        name = "constructor_args",
        conflicts_with = "constructor_args_path"
    )]
    constructor_args: Vec<String>,

    #[clap(
        long,
        help = "path to a file containing the constructor args",
        value_hint = ValueHint::FilePath,
        name = "constructor_args_path",
        conflicts_with = "constructor_args",
    )]
    constructor_args_path: Option<PathBuf>,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(flatten)]
    eth: EthereumOpts,

    #[clap(help = "contract source info `<path>:<contractname>` or `<contractname>`")]
    contract: ContractInfo,

    #[clap(
        long,
        help = "use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559"
    )]
    legacy: bool,
}

impl Cmd for CreateArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        // Find Project & Compile
        let project = self.opts.project()?;
        let compiled = super::compile(&project)?;

        // Get ABI and BIN
        let (abi, bin, _) = super::read_artifact(&project, compiled, self.contract.clone())?;

        let bin = match bin.object {
            BytecodeObject::Bytecode(_) => bin.object,
            _ => eyre::bail!("Dynamic linking not supported in `create` command - deploy the library contract first, then provide the address to link at compile time")
        };

        // Add arguments to constructor
        let provider = Provider::<Http>::try_from(self.eth.rpc_url()?)?;
        let params = match abi.constructor {
            Some(ref v) => {
                let constructor_args =
                    if let Some(ref constructor_args_path) = self.constructor_args_path {
                        if !std::path::Path::new(&constructor_args_path).exists() {
                            eyre::bail!("constructor args path not found");
                        }
                        let file = fs::read_to_string(constructor_args_path)?;
                        file.split(' ').map(|s| s.to_string()).collect::<Vec<String>>()
                    } else {
                        self.constructor_args.clone()
                    };
                self.parse_constructor_args(v, &constructor_args)?
            }
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
    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Abi,
        bin: BytecodeObject,
        args: Vec<Token>,
        provider: M,
    ) -> Result<()> {
        let chain = provider.get_chainid().await?.as_u64();
        let deployer_address =
            provider.default_sender().expect("no sender address set for provider");
        let bin = bin.into_bytes().unwrap_or_else(|| {
            panic!("no bytecode found in bin object for {}", self.contract.name)
        });
        let factory = ContractFactory::new(abi, bin, Arc::new(provider));

        let deployer = factory.deploy_tokens(args)?;
        let deployer = if self.legacy ||
            Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default()
        {
            deployer.legacy()
        } else {
            deployer
        };

        let deployed_contract = deployer.send().await?;

        println!("Deployer: {:?}", deployer_address);
        println!("Deployed to: {:?}", deployed_contract.address());

        Ok(())
    }

    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> Result<Vec<Token>> {
        let params = constructor
            .inputs
            .iter()
            .zip(constructor_args)
            .map(|(input, arg)| (&input.kind, arg.as_str()))
            .collect::<Vec<_>>();

        parse_tokens(params, true)
    }
}
