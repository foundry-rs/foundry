//! Create command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::{EthereumOpts, WalletType},
};
use ethers::{
    abi::{Abi, Constructor, Token},
    prelude::{artifacts::BytecodeObject, ContractFactory, Http, Middleware, Provider},
};
use eyre::Result;
use foundry_utils::parse_tokens;

use crate::opts::forge::ContractInfo;
use std::sync::Arc;
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
}

impl Cmd for CreateArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        // Find Project & Compile
        let project = self.opts.project()?;
        let compiled = super::compile(&project)?;

        // Get ABI and BIN
        let (abi, bin, _) = super::read_artifact(&project, compiled, self.contract.clone())?;

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
    async fn deploy<M: Middleware + 'static>(
        self,
        abi: Abi,
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
