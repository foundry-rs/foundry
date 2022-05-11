//! Verify contract source on etherscan

use super::build::{CoreBuildArgs, ProjectPathsArgs};
use crate::{
    cmd::RetryArgs,
    opts::{forge::ContractInfo, EthereumOpts},
};
use clap::Parser;
use ethers::{
    abi::Address,
    etherscan::{
        contract::{CodeFormat, VerifyContract},
        utils::lookup_compiler_version,
        Client,
    },
    solc::{
        artifacts::{BytecodeHash, Source},
        AggregatedCompilerOutput, CompilerInput, Project, Solc,
    },
};
use eyre::{eyre, Context};
use foundry_config::{Chain, Config, SolcReq};
use futures::FutureExt;
use semver::{BuildMetadata, Version};
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
    compiler_version: Option<String>,

    #[clap(
        alias = "optimizer-runs",
        long,
        help = "The number of optimization runs used to build the smart contract."
    )]
    num_of_optimizations: Option<usize>,

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

    #[clap(long, help = "Wait for verification result after submission")]
    watch: bool,

    #[clap(flatten)]
    retry: RetryArgs,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    project_paths: ProjectPathsArgs,
}

impl VerifyArgs {
    pub fn new(
        address: Address,
        contract: ContractInfo,
        constructor_args: Option<String>,
        num_of_optimizations: Option<usize>,
        eth: EthereumOpts,
        project_paths: ProjectPathsArgs,
        flatten: bool,
        force: bool,
        watch: bool,
        retry: RetryArgs,
    ) -> eyre::Result<Self> {
        Ok(Self {
            address,
            contract,
            compiler_version: None, // TODO:
            constructor_args,
            num_of_optimizations,
            chain: eth.chain.into(),
            flatten,
            force,
            watch,
            project_paths,
            etherscan_key: eth
                .etherscan_api_key
                .ok_or(eyre::eyre!("ETHERSCAN_API_KEY must be set"))?,
            retry,
        })
    }

    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        if self.contract.path.is_none() {
            eyre::bail!("Contract info must be provided in the format <path>:<name>")
        }

        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        let verify_args = self.create_verify_request().await?;

        trace!("submitting verification request {:?}", verify_args);

        println!("addr {}", self.address.to_string());

        let retry = self.retry.clone();
        let resp = retry.run_async(|| {
            async {
                let resp = etherscan
                    .submit_contract_verification(&verify_args)
                    .await
                    .wrap_err("Failed to submit contract verification")?;

                if resp.status == "0" {
                    if resp.message == "Contract source code already verified" {
                        println!("{}", resp.result);
                        return Ok(None);
                    }

                    if resp.result.starts_with("Unable to locate ContractCode at") {
                        println!("Unable to locate ContractCode at");
                        warn!("Unable to locate ContractCode at");
                        return Err(eyre!("not ready"));
                    }

                    warn!("Failed verify submission: {:?}", resp);
                    eprintln!(
                        "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );
                    std::process::exit(1);
                }

                Ok(Some(resp))
            }
            .boxed()
        }).await?;

        if let Some(resp) = resp {
            println!(
                "Submitted contract for verification:\nResponse: `{}`\nGUID: `{}`\nurl: {}#code",
                resp.message,
                resp.result,
                etherscan.address_url(self.address)
            );

            if self.watch {
                let check_args = VerifyCheckArgs {
                    guid: resp.result,
                    chain: self.chain,
                    retry: RetryArgs::new(6, Some(10)),
                    etherscan_key: self.etherscan_key.clone(),
                };
                return check_args.run().await;
            }
        }

        Ok(())
    }

    /// Creates the `VerifyContract` etherescan request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    async fn create_verify_request(&self) -> eyre::Result<VerifyContract> {
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

        if !contract_path.starts_with(project.sources_path()) {
            eyre::bail!("Contract {:?} is outside of project source directory", contract_path);
        }

        let config = Config::load();
        let compiler_version = self.compiler_version(&config)?;

        let (source, contract_name, code_format) = if self.flatten {
            self.flattened_source(&project, &contract_path, &compiler_version)?
        } else {
            self.standard_json_source(&project, &contract_path, &compiler_version)?
        };

        let compiler_version = ensure_solc_build_metadata(compiler_version).await?;
        let compiler_version = format!("v{}", compiler_version);
        let mut verify_args =
            VerifyContract::new(self.address, contract_name, source, compiler_version)
                .constructor_arguments(self.constructor_args.clone())
                .code_format(code_format);

        if code_format == CodeFormat::SingleFile {
            verify_args = if let Some(optimizations) = self.num_of_optimizations {
                verify_args.optimized().runs(optimizations as u32)
            } else if config.optimizer {
                verify_args.optimized().runs(config.optimizer_runs.try_into()?)
            } else {
                verify_args.not_optimized()
            }
        }

        Ok(verify_args)
    }

    /// Parse the compiler version.
    /// The priority desc:
    ///     1. Through CLI arg `--compiler-version`
    ///     2. `solc` defined in foundry.toml  
    fn compiler_version(&self, config: &Config) -> eyre::Result<Version> {
        if let Some(ref version) = self.compiler_version {
            return Ok(version.trim_start_matches('v').parse()?);
        }

        if let Some(ref solc) = config.solc {
            match solc {
                SolcReq::Version(version) => return Ok(version.to_owned()),
                SolcReq::Local(solc) => {
                    if solc.is_file() {
                        return Ok(Solc::new(solc).version()?);
                    }
                }
            }
        }

        eyre::bail!("Compiler version has to be set in `foundry.toml`. If the project were not deployed with foundry, specify the version through `--compiler-version` flag.")
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
    fn check_flattened(&self, content: impl Into<String>, version: &Version) -> eyre::Result<()> {
        let version = strip_build_meta(version.clone());
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
            o.extend(version.clone(), out);
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

    fn flattened_source(
        &self,
        project: &Project,
        target: &Path,
        version: &Version,
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

        if !self.force {
            // solc dry run of flattened code
            self.check_flattened(source.clone(), version).map_err(|err| {
                eyre::eyre!(
                    "Failed to compile the flattened code locally: `{}`\
    To skip this solc dry, have a look at the  `--force` flag of this command.",
                    err
                )
            })?;
        }

        let name = self.contract.name.clone();
        Ok((source, name, CodeFormat::SingleFile))
    }

    fn standard_json_source(
        &self,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> eyre::Result<(String, String, CodeFormat)> {
        let input = project
            .standard_json_input(target)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(version);

        let source =
            serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;
        let name = format!(
            "{}:{}",
            target.strip_prefix(project.root()).unwrap_or(target).display(),
            self.contract.name.clone()
        );
        Ok((source, name, CodeFormat::StandardJsonInput))
    }
}

/// Strips [BuildMetadata] from the [Version]
///
/// **Note:** this is only for local compilation as a dry run, therefore this will return a
/// sanitized variant of the specific version so that it can be installed. This is merely
/// intended to ensure the flattened code can be compiled without errors.
fn strip_build_meta(version: Version) -> Version {
    if version.build != BuildMetadata::EMPTY {
        Version::new(version.major, version.minor, version.patch)
    } else {
        version
    }
}

/// Given any solc [Version] return a [Version] with build metadata
///
/// # Example
/// ```
/// let version = Version::new(1, 2, 3);
/// let version = ensure_solc_build_metadata(version).await?;
/// assert_ne!(version.build, BuildMetadata::EMPTY);
/// ```
async fn ensure_solc_build_metadata(version: Version) -> eyre::Result<Version> {
    if version.build != BuildMetadata::EMPTY {
        Ok(version)
    } else {
        Ok(lookup_compiler_version(&version).await?)
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

    #[clap(flatten)]
    retry: RetryArgs,

    #[clap(help = "Your Etherscan API key.", env = "ETHERSCAN_API_KEY")]
    etherscan_key: String,
}

impl VerifyCheckArgs {
    /// Executes the command to check verification status on Etherscan
    pub async fn run(&self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        println!("Waiting for verification result...");
        let retry = self.retry.clone();
        retry
            .run_async(|| {
                async {
                    let resp = etherscan
                        .check_contract_verification_status(self.guid.clone())
                        .await
                        .wrap_err("Failed to request verification status")?;

                    if resp.status == "0" {
                        if resp.result == "Already Verified" {
                            println!("Contract source code already verified");
                            return Ok(());
                        }

                        if resp.result == "Pending in queue" {
                            return Err(eyre!("Verification is still pending...",));
                        }

                        eprintln!(
                            "Contract verification failed:\nResponse: `{}`\nDetails: `{}`",
                            resp.message, resp.result
                        );
                        std::process::exit(1);
                    }

                    println!("Contract successfully verified.");
                    Ok(())
                }
                .boxed()
            })
            .await
            .wrap_err("Checking verification result failed:")
    }
}
