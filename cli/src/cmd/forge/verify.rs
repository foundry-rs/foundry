//! Verify contract source on etherscan

use super::build::{CoreBuildArgs, ProjectPathsArgs};
use crate::opts::forge::ContractInfo;
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{
        contract::{CodeFormat, VerifyContract},
        Client,
    },
    solc::{
        artifacts::{BytecodeHash, Source},
        AggregatedCompilerOutput, CompilerInput, Project, Solc,
    },
};
use eyre::Context;
use foundry_config::Chain;
use semver::Version;
use std::{collections::BTreeMap, path::Path};
use tracing::{trace, warn};

/// Verification arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "The address of the contract to verify.")]
    address: Address,

    #[clap(help = "The contract identifier in the form `<path>:<contractname>`.")]
    contract: ContractInfo,

    #[clap(long, help = "the encoded constructor arguments")]
    constructor_args: Option<String>,

    #[clap(long, help = "The compiler version used to build the smart contract.")]
    compiler_version: String,

    #[clap(
        alias = "optimizer-runs",
        long,
        help = "The number of optimization runs used to build the smart contract."
    )]
    num_of_optimizations: Option<u32>,

    #[clap(
        long,
        alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet"
    )]
    chain: Chain,

    #[clap(help = "Your Etherscan API key.", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,

    #[clap(help = "Flatten the source code before verifying.", long = "flatten")]
    flatten: bool,

    #[clap(
        short,
        long,
        help = "Do not compile the flattened smart contract before verifying (if --flatten is passed)."
    )]
    force: bool,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    project_paths: ProjectPathsArgs,
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        if self.contract.path.is_none() {
            eyre::bail!("Contract info must be provided in the format <path>:<name>")
        }

        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        let verify_args = self.create_verify_request()?;

        trace!("submitting verification request {:?}", verify_args);

        let resp = etherscan
            .submit_contract_verification(&verify_args)
            .await
            .wrap_err("Failed to submit contract verification")?;

        if resp.status == "0" {
            if resp.message == "Contract source code already verified" {
                println!("Contract source code already verified.");
                return Ok(())
            }

            if resp.result == "Contract source code already verified" {
                println!("Contract source code already verified");
                return Ok(())
            }

            warn!("Failed verify submission: {:?}", resp);

            eprintln!(
                "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                resp.message, resp.result
            );
            std::process::exit(1)
        }

        println!(
            r#"Submitted contract for verification:
    Response: `{}`
    GUID: `{}`
    url: {}#code"#,
            resp.message,
            resp.result,
            etherscan.address_url(self.address)
        );
        Ok(())
    }

    /// Creates the `VerifyContract` etherescan request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    fn create_verify_request(&self) -> eyre::Result<VerifyContract> {
        let build_args = CoreBuildArgs {
            project_paths: self.project_paths.clone(),
            out_path: Default::default(),
            compiler: Default::default(),
            ignored_error_codes: vec![],
            no_auto_detect: false,
            use_solc: None,
            offline: false,
            force: false,
            libraries: vec![],
            via_ir: false,
            revert_strings: None,
        };

        let project = build_args.project()?;

        // check that the provided contract is part of the source dir
        let contract_path =
            project.root().join(self.contract.path.as_ref().expect("Is present; qed"));

        if !contract_path.exists() {
            eyre::bail!("Contract {:?} does not exist.", contract_path);
        }

        let (source, contract_name, code_format) = if self.flatten {
            flattened_source(self, &project, &contract_path)?
        } else {
            standard_json_source(self, &project, &contract_path)?
        };

        let mut verify_args =
            VerifyContract::new(self.address, contract_name, source, self.compiler_version.clone())
                .constructor_arguments(self.constructor_args.clone())
                .code_format(code_format);

        if code_format == CodeFormat::SingleFile {
            verify_args = if let Some(optimizations) = self.num_of_optimizations {
                verify_args.optimization(true).runs(optimizations)
            } else {
                verify_args.optimization(false)
            };
        }

        Ok(verify_args)
    }

    /// Parses the [Version] from the provided compiler version
    ///
    /// All etherscan supported compiler versions are listed here <https://etherscan.io/solcversions>
    ///
    /// **Note:** this is only for local compilation as a dry run, therefore this will return a
    /// sanitized variant of the specific version so that it can be installed. This is merely
    /// intended to ensure the flattened code can be compiled without errors.
    ///
    /// # Example
    ///
    /// the `compiler_version` `v0.8.7+commit.e28d00a7` will be returned as `0.8.7`
    fn sanitized_solc_version(&self) -> eyre::Result<Version> {
        let v: Version = self.compiler_version.trim_start_matches('v').parse()?;
        Ok(Version::new(v.major, v.minor, v.patch))
    }

    /// Attempts to compile the flattened content locally with the compiler version.
    ///
    /// This expects the completely flattened `content´ and will try to compile it using the
    /// provided compiler. If the compiler is missing it will be installed.
    ///
    /// # Errors
    ///
    /// If it failed to install a missing solc compiler
    ///
    /// # Exits
    ///
    /// If the solc compiler output contains errors, this could either be due to a bug in the
    /// flattening code or could to conflict in the flattened code, for example if there are
    /// multiple interfaces with the same name.
    fn check_flattened(&self, content: impl Into<String>) -> eyre::Result<()> {
        let version: Version = self.sanitized_solc_version()?;
        let solc = if let Some(solc) = Solc::find_svm_installed_version(version.to_string())? {
            solc
        } else {
            Solc::blocking_install(&version)?
        };
        let input = CompilerInput {
            language: "Solidity".to_string(),
            sources: BTreeMap::from([("constract.sol".into(), Source { content: content.into() })]),
            settings: Default::default(),
        };

        let out = solc.compile(&input)?;
        if out.has_error() {
            let mut o = AggregatedCompilerOutput::default();
            o.extend(version, out);
            eprintln!("{}", o.diagnostics(&[]));

            eprintln!(
                r#"Failed to compile the flattened code locally.
This could be a bug, please inspect the outout of `forge flatten {}` and report an issue.
To skip this solc dry, pass `--force`.
"#,
                self.contract.path.as_ref().expect("Path is some;")
            );
            std::process::exit(1)
        }

        Ok(())
    }
}

/// Check verification status arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyCheckArgs {
    #[clap(help = "The verification GUID.")]
    guid: String,

    #[clap(
        long,
        alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet"
    )]
    chain: Chain,

    #[clap(help = "Your Etherscan API key.", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
}

impl VerifyCheckArgs {
    /// Executes the command to check verification status on Etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        let resp = etherscan
            .check_contract_verification_status(self.guid.clone())
            .await
            .wrap_err("Failed to request verification status")?;

        if resp.status == "0" {
            if resp.result == "Pending in queue" {
                println!("Verification is pending...");
                return Ok(())
            }

            if resp.result == "Already Verified" {
                println!("Contract source code already verified");
                return Ok(())
            }

            warn!("Failed verification: {:?}", resp);

            eprintln!(
                "Contract verification failed:\nResponse: `{}`\nDetails: `{}`",
                resp.message, resp.result
            );

            std::process::exit(1);
        }

        println!("Contract successfully verified.");
        Ok(())
    }
}

fn flattened_source(
    args: &VerifyArgs,
    project: &Project,
    target: &Path,
) -> eyre::Result<(String, String, CodeFormat)> {
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

    if !args.force {
        // solc dry run of flattened code
        args.check_flattened(source.clone()).map_err(|err| {
            eyre::eyre!(
                "Failed to compile the flattened code locally: `{}`\
To skip this solc dry, have a look at the  `--force` flag of this command.",
                err
            )
        })?;
    }

    let name = args.contract.name.clone();
    Ok((source, name, CodeFormat::SingleFile))
}

fn standard_json_source(
    args: &VerifyArgs,
    project: &Project,
    target: &Path,
) -> eyre::Result<(String, String, CodeFormat)> {
    let input = project
        .standard_json_input(target)
        .wrap_err("Failed to get standard json input")?
        .normalize_evm_version(&args.sanitized_solc_version()?);

    let source = serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;
    let name = format!(
        "{}:{}",
        target.strip_prefix(project.root()).unwrap_or(target).display(),
        args.contract.name.clone()
    );
    Ok((source, name, CodeFormat::StandardJsonInput))
}
