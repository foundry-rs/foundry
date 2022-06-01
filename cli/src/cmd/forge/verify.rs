//! Verify contract source on etherscan

use super::build::{CoreBuildArgs, ProjectPathsArgs};
use crate::{cmd::RetryArgs, opts::forge::ContractInfo};
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
        cache::CacheEntry,
        AggregatedCompilerOutput, CompilerInput, Project, Solc,
    },
};
use eyre::{eyre, Context};
use foundry_config::{Chain, Config, SolcReq};
use foundry_utils::Retry;
use futures::FutureExt;
use once_cell::sync::Lazy;
use regex::Regex;
use semver::{BuildMetadata, Version};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tracing::{trace, warn};

pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?P<commit>commit\.[0-9,a-f]{8})"#).unwrap());

pub const RETRY_CHECK_ON_VERIFY: RetryArgs = RetryArgs { retries: 6, delay: Some(10) };

/// Verification arguments
#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    #[clap(help = "The address of the contract to verify.", value_name = "ADDRESS")]
    pub address: Address,

    #[clap(
        help = "The contract identifier in the form `<path>:<contractname>`.",
        value_name = "CONTRACT"
    )]
    pub contract: ContractInfo,

    #[clap(long, help = "the encoded constructor arguments", value_name = "ARGS")]
    pub constructor_args: Option<String>,

    #[clap(
        long,
        help = "The compiler version used to build the smart contract.",
        value_name = "VERSION"
    )]
    pub compiler_version: Option<String>,

    #[clap(
        visible_alias = "optimizer-runs",
        long,
        help = "The number of optimization runs used to build the smart contract.",
        value_name = "NUM"
    )]
    pub num_of_optimizations: Option<usize>,

    #[clap(
        long,
        visible_alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet",
        value_name = "CHAIN"
    )]
    pub chain: Chain,

    #[clap(
        help = "Your Etherscan API key.",
        env = "ETHERSCAN_API_KEY",
        value_name = "ETHERSCAN_KEY"
    )]
    pub etherscan_key: String,

    #[clap(help = "Flatten the source code before verifying.", long = "flatten")]
    pub flatten: bool,

    #[clap(
        short,
        long,
        help = "Do not compile the flattened smart contract before verifying (if --flatten is passed)."
    )]
    pub force: bool,

    #[clap(long, help = "Wait for verification result after submission")]
    pub watch: bool,

    #[clap(flatten)]
    pub retry: RetryArgs,

    #[clap(flatten, next_help_heading = "PROJECT OPTIONS")]
    pub project_paths: ProjectPathsArgs,
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(mut self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        let verify_args = self.create_verify_request().await?;

        trace!("submitting verification request {:?}", verify_args);

        let retry: Retry = self.retry.into();
        let resp = retry.run_async(|| {
            async {
                println!("\nSubmitting verification for [{}] {:?}.", verify_args.contract_name, verify_args.address);
                let resp = etherscan
                    .submit_contract_verification(&verify_args)
                    .await
                    .wrap_err("Failed to submit contract verification")?;

                if resp.status == "0" {
                    if resp.result == "Contract source code already verified" {
                        return Ok(None)
                    }

                    if resp.result.starts_with("Unable to locate ContractCode at") {
                        warn!("{}", resp.result);
                        return Err(eyre!("Etherscan could not detect the deployment."))
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
                "Submitted contract for verification:\n\tResponse: `{}`\n\tGUID: `{}`\n\tURL: {}#code",
                resp.message,
                resp.result,
                etherscan.address_url(self.address)
            );

            if self.watch {
                let check_args = VerifyCheckArgs {
                    guid: resp.result,
                    chain: self.chain,
                    retry: RETRY_CHECK_ON_VERIFY,
                    etherscan_key: self.etherscan_key,
                };
                return check_args.run().await
            }
        } else {
            println!("Contract source code already verified");
        }

        Ok(())
    }

    /// Creates the `VerifyContract` etherescan request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    async fn create_verify_request(&mut self) -> eyre::Result<VerifyContract> {
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
        let config = Config::load();

        if self.contract.path.is_none() && !config.cache {
            eyre::bail!(
                "If cache is disabled, contract info must be provided in the format <path>:<name>"
            );
        }

        let should_read_cache = self.contract.path.is_none() ||
            (self.compiler_version.is_none() && config.solc.is_none());
        let cached_entry = if config.cache && should_read_cache {
            let cache = project.read_cache_file()?;
            Some(crate::cmd::get_cached_entry_by_name(&cache, &self.contract.name)?)
        } else {
            None
        };

        let contract_path = if let Some(ref path) = self.contract.path {
            project.root().join(path)
        } else {
            cached_entry.as_ref().unwrap().0.to_owned()
        };

        // check that the provided contract is part of the source dir
        if !contract_path.exists() {
            eyre::bail!("Contract {:?} does not exist.", contract_path);
        }

        let compiler_version = self.compiler_version(&config, &cached_entry)?;

        let (source, contract_name, code_format) = if self.flatten {
            self.flattened_source(&project, &contract_path, &compiler_version, &contract_path)?
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
            };
        }

        Ok(verify_args)
    }

    /// Parse the compiler version.
    /// The priority desc:
    ///     1. Through CLI arg `--compiler-version`
    ///     2. `solc` defined in foundry.toml  
    fn compiler_version(
        &self,
        config: &Config,
        // cache: &SolFilesCache,
        // contract_path: &Path,
        entry: &Option<(PathBuf, CacheEntry)>,
    ) -> eyre::Result<Version> {
        if let Some(ref version) = self.compiler_version {
            return Ok(version.trim_start_matches('v').parse()?)
        }

        if let Some(ref solc) = config.solc {
            match solc {
                SolcReq::Version(version) => return Ok(version.to_owned()),
                SolcReq::Local(solc) => {
                    if solc.is_file() {
                        return Ok(Solc::new(solc).version()?)
                    }
                }
            }
        }

        if let Some((_, entry)) = entry {
            let artifacts = entry.artifacts_versions().collect::<Vec<_>>();
            if artifacts.len() == 1 {
                let mut version = artifacts[0].0.to_owned();
                version.build = match RE_BUILD_COMMIT.captures(version.build.as_str()) {
                    Some(cap) => BuildMetadata::new(cap.name("commit").unwrap().as_str())?,
                    _ => BuildMetadata::EMPTY,
                };
                return Ok(version)
            }

            if artifacts.is_empty() {
                warn!("no artifacts detected")
            } else {
                warn!(
                    "ambiguous compiler versions found in cache: {}",
                    artifacts.iter().map(|a| a.0.to_string()).collect::<Vec<_>>().join(", ")
                );
            }
        }

        eyre::bail!("Compiler version has to be set in `foundry.toml`. If the project was not deployed with foundry, specify the version through `--compiler-version` flag.")
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
    fn check_flattened(
        &self,
        content: impl Into<String>,
        version: &Version,
        contract_path: &Path,
    ) -> eyre::Result<()> {
        let version = strip_build_meta(version.clone());
        let solc = if let Some(solc) = Solc::find_svm_installed_version(version.to_string())? {
            solc
        } else {
            Solc::blocking_install(&version)?
        };
        let input = CompilerInput {
            language: "Solidity".to_string(),
            sources: BTreeMap::from([("contract.sol".into(), Source { content: content.into() })]),
            settings: Default::default(),
        };

        let out = solc.compile(&input)?;
        if out.has_error() {
            let mut o = AggregatedCompilerOutput::default();
            o.extend(version, out);
            eprintln!("{}", o.diagnostics(&[]));

            eprintln!(
                r#"Failed to compile the flattened code locally.
This could be a bug, please inspect the output of `forge flatten {}` and report an issue.
To skip this solc dry, pass `--force`.
"#,
                contract_path.display()
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
        contract_path: &Path,
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
            "When using flattened source, bytecodeHash must be set to ipfs because Etherscan uses IPFS in its Compiler Settings when re-compiling your code. BytecodeHash is currently: {}. Hint: Set the bytecodeHash key in your foundry.toml :)",
            bch,
        );

        let source = project.flatten(target).wrap_err("Failed to flatten contract")?;

        if !self.force {
            // solc dry run of flattened code
            self.check_flattened(source.clone(), version, contract_path).map_err(|err| {
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
    #[clap(help = "The verification GUID.", value_name = "GUID")]
    guid: String,

    #[clap(
        long,
        visible_alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet",
        value_name = "CHAIN"
    )]
    chain: Chain,

    #[clap(flatten)]
    retry: RetryArgs,

    #[clap(
        help = "Your Etherscan API key.",
        env = "ETHERSCAN_API_KEY",
        value_name = "ETHERSCAN_KEY"
    )]
    etherscan_key: String,
}

impl VerifyCheckArgs {
    /// Executes the command to check verification status on Etherscan
    pub async fn run(self) -> eyre::Result<()> {
        let etherscan = Client::new(self.chain.try_into()?, &self.etherscan_key)
            .wrap_err("Failed to create etherscan client")?;

        println!("Waiting for verification result...");
        let retry: Retry = self.retry.into();
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
                            return Ok(())
                        }

                        if resp.result == "Pending in queue" {
                            return Err(eyre!("Verification is still pending...",))
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
