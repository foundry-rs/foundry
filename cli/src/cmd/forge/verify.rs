//! Verify contract source on etherscan

use std::path::Path;

use crate::{
    cmd::forge::{build::BuildArgs, flatten::CoreFlattenArgs},
    opts::forge::ContractInfo,
};
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{contract::VerifyContract, Client},
    prelude::Project,
    solc::artifacts::BytecodeHash,
};
use eyre::Context;

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

    #[clap(
        help = "flatten the source code. Make sure to use bytecodehash='ipfs'",
        long = "flatten"
    )]
    flatten: bool,

    #[clap(flatten)]
    opts: CoreFlattenArgs,
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

fn flattened_source(
    args: &VerifyArgs,
    project: &Project,
    target: &Path,
) -> eyre::Result<(String, String)> {
    let bch = project
        .solc_config
        .settings
        .metadata
        .as_ref()
        .and_then(|m| m.bytecode_hash)
        .unwrap_or_default();

    eyre::ensure!(
        bch == BytecodeHash::Ipfs,
        "When using flattened source, bytecodeHash must be set to ipfs. BytecodeHash is currently: {}. Hint: Set the bytecodeHash key in your foundry.toml :)",
        bch,
    );

    let source = project.flatten(target).wrap_err("Failed to flatten contract")?;
    let name = args.contract.name.clone();
    Ok((source, name))
}

fn standard_json_source(
    args: &VerifyArgs,
    project: &Project,
    target: &Path,
) -> eyre::Result<(String, String)> {
    let input =
        project.standard_json_input(target).wrap_err("Failed to get standard json input")?;

    let source = serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;
    let name = format!(
        "{}:{}",
        &project.root().join(args.contract.path.as_ref().unwrap()).to_string_lossy(),
        args.contract.name.clone()
    );
    Ok((source, name))
}

/// Run the verify command to submit the contract's source code for verification on etherscan
pub async fn run_verify(args: &VerifyArgs) -> eyre::Result<()> {
    if args.contract.path.is_none() {
        eyre::bail!("Contract info must be provided in the format <path>:<name>")
    }

    let CoreFlattenArgs {
        root,
        contracts,
        remappings,
        remappings_env,
        cache_path,
        lib_paths,
        hardhat,
    } = args.opts.clone();

    let build_args = BuildArgs {
        root,
        contracts,
        remappings,
        remappings_env,
        cache_path,
        lib_paths,
        out_path: None,
        compiler: Default::default(),
        names: false,
        sizes: false,
        ignored_error_codes: vec![],
        no_auto_detect: false,
        use_solc: None,
        offline: false,
        force: false,
        hardhat,
        libraries: vec![],
        watch: Default::default(),
        via_ir: false,
        config_path: None,
    };

    let project = build_args.project()?;
    let target = &project.root().join(args.contract.path.as_ref().unwrap());

    let (source, contract_name) = if args.flatten {
        flattened_source(args, &project, target)?
    } else {
        standard_json_source(args, &project, target)?
    };

    let etherscan = Client::new(args.chain_id.try_into()?, &args.etherscan_key)
        .wrap_err("Failed to create etherscan client")?;

    let mut verify_args =
        VerifyContract::new(args.address, contract_name, source, args.compiler_version.clone())
            .constructor_arguments(args.constructor_args.clone());

    verify_args = if let Some(optimizations) = args.num_of_optimizations {
        verify_args.optimization(true).runs(optimizations)
    } else {
        verify_args.optimization(false)
    };

    let resp = etherscan
        .submit_contract_verification(&verify_args)
        .await
        .wrap_err("Failed to submit contract verification")?;

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
        .wrap_err("Failed to create etherscan client")?;

    let resp = etherscan
        .check_contract_verification_status(args.guid.clone())
        .await
        .wrap_err("Failed to request verification status")?;

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
