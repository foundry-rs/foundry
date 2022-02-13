//! Verify contract source on etherscan

use crate::cmd::build::BuildArgs;
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{contract::VerifyContract, Client},
};

/// Command to list remappings
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "the target contract address")]
    address: Address,

    #[clap(help = "the target contract path")]
    contract: String,

    #[clap(long, help = "the encoded constructor arguments calldata")]
    args: Option<String>,

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

/// Run the verify command to submit the contract's source code for verification on etherscan
pub async fn run(args: &VerifyArgs) -> eyre::Result<()> {
    if args.etherscan_key == "" {
        eyre::bail!("Etherscan API key is required");
    }

    let project = args.opts.project()?;
    let contract = project
        .flatten(&project.root().join(&args.contract))
        .map_err(|err| eyre::eyre!("Failed to flatten contract: {}", err))?;

    let etherscan = Client::new(args.chain_id.try_into()?, &args.etherscan_key)
        .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

    let mut verify_args = VerifyContract::new(args.address, contract, String::new());

    if let Some(optimizations) = args.num_of_optimizations {
        verify_args = verify_args.optimization(true).runs(optimizations);
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
