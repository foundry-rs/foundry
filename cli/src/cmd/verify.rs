//! Verify contract source on etherscan

use crate::{cmd::build::BuildArgs, opts::forge::ContractInfo};
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{contract::VerifyContract, Client},
};

/// Verification arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "the target contract address")]
    address: Address,

    #[clap(help = "the contract source info `<path>:<contractname>`")]
    contract: ContractInfo,

    #[clap(long, help = "the encoded constructor arguments")]
    constructor_args: Option<String>,

    #[clap(long, help = "the compiler version used during build")]
    compiler_version: String,

    #[clap(long, help = "the number of optimization runs used")]
    num_of_optimizations: Option<u32>,

    // TODO: Allow choosing network using the provider or chainid as string
    #[clap(long, help = "the chain id of the network you are verifying for", default_value = "1")]
    chain_id: u64,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,

    #[clap(flatten)]
    opts: BuildArgs,
}

/// Check verification status arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyCheckArgs {
    #[clap(help = "the verification guid")]
    guid: String,

    // TODO: Allow choosing network using the provider or chainid as string
    #[clap(long, help = "the chain id of the network you are verifying for", default_value = "1")]
    chain_id: u64,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
}

/// Run the verify command to submit the contract's source code for verification on etherscan
pub async fn run_verify(args: &VerifyArgs) -> eyre::Result<()> {
    if args.contract.path.is_none() {
        eyre::bail!("Contract info must be provided in the format <path>:<name>")
    }

    let project = args.opts.project()?;
    let contract = project
        .flatten(&project.root().join(args.contract.path.as_ref().unwrap()))
        .map_err(|err| eyre::eyre!("Failed to flatten contract: {}", err))?;

    let etherscan = Client::new(args.chain_id.try_into()?, &args.etherscan_key)
        .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

    let mut verify_args = VerifyContract::new(
        args.address,
        args.contract.name.clone(),
        contract,
        args.compiler_version.clone(),
    )
    .constructor_arguments(args.constructor_args.clone());

    if let Some(optimizations) = args.num_of_optimizations {
        verify_args = verify_args.optimization(true).runs(optimizations);
    } else {
        verify_args = verify_args.optimization(false);
    }

    let resp = etherscan
        .submit_contract_verification(&verify_args)
        .await
        .map_err(|err| eyre::eyre!("Failed to submit contract verification: {}", err))?;

    if resp.status == "0" {
        if resp.message == "Contract source code already verified" {
            println!("Contract source code already verified.");
            return Ok(())
        }

        eyre::bail!(
            "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
            resp.message,
            resp.result
        );
    }

    println!(
        r#"Submitted contract for verification:
                Response: `{}`
                GUID: `{}`
                url: {}#code"#,
        resp.message,
        resp.result,
        etherscan.address_url(args.address)
    );
    Ok(())
}

pub async fn run_verify_check(args: &VerifyCheckArgs) -> eyre::Result<()> {
    let etherscan = Client::new(args.chain_id.try_into()?, &args.etherscan_key)
        .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

    let resp = etherscan
        .check_contract_verification_status(args.guid.clone())
        .await
        .map_err(|err| eyre::eyre!("Failed to request verification status: {}", err))?;

    if resp.status == "0" {
        if resp.result == "Pending in queue" {
            println!("Verification is pending...");
            return Ok(())
        }

        eyre::bail!(
            "Contract verification failed:\nResponse: `{}`\nDetails: `{}`",
            resp.message,
            resp.result
        );
    }

    println!("Contract successfully verified.");
    Ok(())
}
