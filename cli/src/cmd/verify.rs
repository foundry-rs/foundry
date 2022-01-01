#![allow(unused)]
//! Verify contract source on etherscan

use crate::{
    cmd::{build::BuildArgs, read_artifact, Cmd},
    opts::forge::ContractInfo,
    utils,
};

use cast::SimpleCast;
use ethers::{
    abi::{Address, Function, FunctionExt},
    core::types::Chain,
    etherscan::{contract::VerifyContract, Client},
    prelude::Provider,
    providers::Middleware,
};
use eyre::ContextCompat;
use std::convert::TryFrom;
use structopt::StructOpt;

#[derive(Clone, Debug, StructOpt)]
pub struct VerifyArgs {
    #[structopt(help = "contract source info `<path>:<contractname>`")]
    contract: ContractInfo,

    #[structopt(help = "the address of the contract to verify.")]
    address: Address,

    #[structopt(flatten)]
    opts: BuildArgs,

    // TODO: Allow choosing network using the provider or chainid as string
    #[structopt(
        long,
        help = "the chain id of the network you are verifying for",
        default_value = "1"
    )]
    chain_id: u64,

    #[structopt(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
    #[structopt(help = "constructor args calldata arguments.")]
    args: Vec<String>,
}

impl Cmd for VerifyArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let address = self.address;
        let project = self.opts.project()?;

        // get the artifacts
        let compiled = super::compile(&project)?;

        // Get ABI and bytecode
        let (abi, bin, runtime_bin) =
            super::read_artifact(&project, compiled, self.contract.clone())?;

        // find the compiler version corresponding to that contract
        let source_versions = project.source_versions()?;
        let runs = source_versions.optimizer_runs;
        let compiler_version = source_versions
            .versions
            .iter()
            .find_map(|(solc, paths)| {
                let paths =
                    paths.iter().map(|path| format!("{}", path.display())).collect::<Vec<_>>();

                // if a path is specified, check that the path matches
                let found = match self.contract.path {
                    Some(ref path) => paths.contains(&path),
                    None => true,
                };
                if found {
                    Some(solc)
                } else {
                    None
                }
            })
            .expect("no solc version found");

        let mut constructor_args = None;
        let calldata = if let Some(constructor) = abi.constructor {
            // convert constructor into function
            #[allow(deprecated)]
            let fun = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs,
                outputs: vec![],
                constant: false,
                state_mutability: Default::default(),
            };
            if self.args.len() != fun.inputs.len() {
                eyre::bail!("wrong constructor argument count")
            }
            constructor_args = Some(SimpleCast::calldata(fun.abi_signature(), &self.args)?);
        } else if !self.args.is_empty() {
            eyre::bail!("No constructor found but contract arguments provided")
        };

        let chain = match self.chain_id {
            1 => Chain::Mainnet,
            3 => Chain::Ropsten,
            4 => Chain::Rinkeby,
            5 => Chain::Goerli,
            42 => Chain::Kovan,
            100 => Chain::XDai,
            _ => eyre::bail!("unexpected chain {}", self.chain_id),
        };
        let etherscan = Client::new(chain, self.etherscan_key)
            .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

        // FIXME: This is wrong. We need to either flatten down the file and its dependencies, or
        // support multi-file confirmation. This is still a bit tricky as we are missing components
        // for it in ethers-etherscan.
        let source = std::fs::read_to_string(self.contract.name)?;

        let contract = VerifyContract::new(address, source, compiler_version.clone())
            .constructor_arguments(constructor_args)
            .optimization(runs.is_some())
            .runs(runs.unwrap_or_default() as u32);

        let rt = tokio::runtime::Runtime::new()?;
        let resp = rt
            .block_on(etherscan.submit_contract_verification(&contract))
            .map_err(|err| eyre::eyre!("Failed to submit contract verification: {}", err))?;

        if resp.status == "0" {
            if resp.message == "Contract source code already verified" {
                println!("Contract source code already verified.");
            } else {
                eyre::bail!(
                    "Encountered an error verifying this contract:\nResponse: `{}`\nDetails:
        `{}`",
                    resp.message,
                    resp.result
                );
            }
        } else {
            println!(
                r#"Submitted contract for verification:
            Response: `{}`
            GUID: `{}`
            url: {}#code"#,
                resp.message,
                resp.result,
                etherscan.address_url(address)
            );
        }
        Ok(())
    }
}
