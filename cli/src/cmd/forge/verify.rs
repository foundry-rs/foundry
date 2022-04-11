//! Verify contract source on etherscan

use crate::{
    cmd::forge::{build::BuildArgs, flatten::CoreFlattenArgs},
    opts::forge::ContractInfo,
};
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{
        contract::{CodeFormat, VerifyContract},
        Client,
    },
    solc::{artifacts::Source, AggregatedCompilerOutput, CompilerInput, Solc},
};
use foundry_config::Chain;
use semver::Version;
use std::collections::BTreeMap;
use tracing::warn;

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

    #[clap(alias = "optimizations", long, help = "the number of optimization runs used")]
    num_of_optimizations: Option<u32>,

    #[clap(
        long,
        help = "the chain id of the network you are verifying for",
        default_value = "mainnet"
    )]
    chain_id: Chain,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,

    #[clap(
        help = "flatten the source code. Make sure to use bytecodehash='ipfs'",
        long = "flatten"
    )]
    flatten: bool,

    #[clap(flatten)]
    opts: CoreFlattenArgs,

    #[clap(
        short,
        long,
        help = r#"usually the command will try to compile the flattened code locally first to ensure it's valid.
This flag we skip that process and send the content directly to the endpoint."#
    )]
    force: bool,
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        if self.contract.path.is_none() {
            eyre::bail!("Contract info must be provided in the format <path>:<name>")
        }

        let verify_args = self.create_verify_request()?;

        let etherscan = Client::new(self.chain_id.try_into()?, &self.etherscan_key)
            .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

        let resp = etherscan
            .submit_contract_verification(&verify_args)
            .await
            .map_err(|err| eyre::eyre!("Failed to submit contract verification: {}", err))?;

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
        let CoreFlattenArgs {
            root,
            contracts,
            remappings,
            remappings_env,
            cache_path,
            lib_paths,
            hardhat,
        } = self.opts.clone();

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

        let (source, contract_name, code_format) = if self.flatten {
            // NOTE: user need to set bytecodehash='ipfs' for this otherwise verification won't work
            // see: https://github.com/gakonst/foundry/issues/1236

            let source = project
                .flatten(&project.root().join(self.contract.path.as_ref().unwrap()))
                .map_err(|err| eyre::eyre!("Failed to flatten contract: {}", err))?;

            if !self.force {
                // solc dry run of flattened code
                self.check_flattened(source.clone()).map_err(|err| {
                    eyre::eyre!(
                        "Failed to compile the flattened code locally: `{}`\
To skip this solc dry, have a look at the  `--force` flag of this command.",
                        err
                    )
                })?;
            }

            (source, self.contract.name.clone(), CodeFormat::SingleFile)
        } else {
            let input = project
                .standard_json_input(&project.root().join(self.contract.path.as_ref().unwrap()))
                .map_err(|err| eyre::eyre!("Failed to get standard json input: {}", err))?;

            let source = serde_json::to_string(&input)
                .map_err(|err| eyre::eyre!("Failed to parse standard json input: {}", err))?;

            let contract_name = format!(
                "{}:{}",
                &project.root().join(self.contract.path.as_ref().unwrap()).to_string_lossy(),
                self.contract.name.clone()
            );

            (source, contract_name, CodeFormat::StandardJsonInput)
        };

        let mut verify_args =
            VerifyContract::new(self.address, contract_name, source, self.compiler_version.clone())
                .constructor_arguments(self.constructor_args.clone())
                .code_format(code_format);

        verify_args = if let Some(optimizations) = self.num_of_optimizations {
            verify_args.optimization(true).runs(optimizations)
        } else {
            verify_args.optimization(false)
        };

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
To skip this solc dry, have a look at the  `--force` flag of this command.
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
    #[clap(help = "the verification guid")]
    guid: String,

    #[clap(
        long,
        help = "the chain id of the network you are verifying for",
        default_value = "mainnet"
    )]
    chain_id: Chain,

    #[clap(help = "your etherscan api key", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
}

impl VerifyCheckArgs {
    /// Executes the command to check verification status on Etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain_id.try_into()?, &self.etherscan_key)
            .map_err(|err| eyre::eyre!("Failed to create etherscan client: {}", err))?;

        let resp = etherscan
            .check_contract_verification_status(self.guid.clone())
            .await
            .map_err(|err| eyre::eyre!("Failed to request verification status: {}", err))?;

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
