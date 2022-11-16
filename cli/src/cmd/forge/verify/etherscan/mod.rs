mod flatten;
mod standard_json;

use super::{VerifyArgs, VerifyCheckArgs};
use crate::cmd::{
    forge::verify::provider::VerificationProvider, read_constructor_args_file,
    retry::RETRY_CHECK_ON_VERIFY, LoadConfig,
};
use cast::SimpleCast;
use ethers::{
    abi::Function,
    etherscan::{
        utils::lookup_compiler_version,
        verify::{CodeFormat, VerifyContract},
        Client,
    },
    solc::{artifacts::CompactContract, cache::CacheEntry, Project, Solc},
};
use eyre::{eyre, Context};
use foundry_common::abi::encode_args;
use foundry_config::{Chain, Config, SolcReq};
use foundry_utils::Retry;
use futures::FutureExt;
use once_cell::sync::Lazy;
use regex::Regex;
use rustc_hex::ToHex;
use semver::{BuildMetadata, Version};
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};
use tracing::{error, trace, warn};

pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?P<commit>commit\.[0-9,a-f]{8})"#).unwrap());

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EtherscanVerificationProvider;

#[async_trait::async_trait]
impl VerificationProvider for EtherscanVerificationProvider {
    async fn preflight_check(&self, args: VerifyArgs) -> eyre::Result<()> {
        let _ = self.prepare_request(&args).await?;
        Ok(())
    }

    async fn verify(&self, args: VerifyArgs) -> eyre::Result<()> {
        let (etherscan, verify_args) = self.prepare_request(&args).await?;

        trace!(?verify_args, target = "forge::verify", "submitting verification request");

        let retry: Retry = args.retry.into();
        let resp = retry.run_async(|| {
            async {
                println!("\nSubmitting verification for [{}] {:?}.", verify_args.contract_name, SimpleCast::to_checksum_address(&verify_args.address));
                let resp = etherscan
                    .submit_contract_verification(&verify_args)
                    .await
                    .wrap_err_with(|| {
                        // valid json
                        let args = serde_json::to_string(&verify_args).unwrap();
                        error!(?args, target = "forge::verify", "Failed to submit verification");
                        format!("Failed to submit contract verification, payload:\n{args}")
                    })?;

                trace!(?resp, target = "forge::verify", "Received verification response");

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
                "Submitted contract for verification:\n\tResponse: `{}`\n\tGUID: `{}`\n\tURL:
        {}",
                resp.message,
                resp.result,
                etherscan.address_url(args.address)
            );

            if args.watch {
                let check_args = VerifyCheckArgs {
                    id: resp.result,
                    chain: args.chain,
                    retry: RETRY_CHECK_ON_VERIFY,
                    etherscan_key: args.etherscan_key,
                    verifier: args.verifier,
                };
                // return check_args.run().await
                return self.check(check_args).await
            }
        } else {
            println!("Contract source code already verified");
        }

        Ok(())
    }

    /// Executes the command to check verification status on Etherscan
    async fn check(&self, args: VerifyCheckArgs) -> eyre::Result<()> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.chain,
            args.verifier.verifier_url.as_deref(),
            args.etherscan_key.as_deref(),
            &config,
        )?;
        let retry: Retry = args.retry.into();
        retry
            .run_async(|| {
                async {
                    let resp = etherscan
                        .check_contract_verification_status(args.id.clone())
                        .await
                        .wrap_err("Failed to request verification status")?;

                    trace!(?resp, target = "forge::verify", "Received verification response");

                    eprintln!(
                        "Contract verification status:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );

                    if resp.result == "Pending in queue" {
                        return Err(eyre!("Verification is still pending...",))
                    }

                    if resp.result == "Unable to verify" {
                        return Err(eyre!("Unable to verify.",))
                    }

                    if resp.result == "Already Verified" {
                        println!("Contract source code already verified");
                        return Ok(())
                    }

                    if resp.status == "0" {
                        println!("Contract failed to verify.");
                        std::process::exit(1);
                    }

                    if resp.result == "Pass - Verified" {
                        println!("Contract successfully verified");
                    }

                    Ok(())
                }
                .boxed()
            })
            .await
            .wrap_err("Checking verification result failed:")
    }
}

impl EtherscanVerificationProvider {
    /// Create a source provider
    fn source_provider(&self, args: &VerifyArgs) -> Box<dyn EtherscanSourceProvider> {
        if args.flatten {
            Box::new(flatten::EtherscanFlattenedSource)
        } else {
            Box::new(standard_json::EtherscanStandardJsonSource)
        }
    }

    /// Configures the API request to the etherscan API using the given [`VerifyArgs`].
    async fn prepare_request(&self, args: &VerifyArgs) -> eyre::Result<(Client, VerifyContract)> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.chain,
            args.verifier.verifier_url.as_deref(),
            args.etherscan_key.as_deref(),
            &config,
        )?;
        let verify_args = self.create_verify_request(args, Some(config)).await?;

        Ok((etherscan, verify_args))
    }

    /// Create an etherscan client
    pub(crate) fn client(
        &self,
        chain: Chain,
        verifier_url: Option<&str>,
        etherscan_key: Option<&str>,
        config: &Config,
    ) -> eyre::Result<Client> {
        let etherscan_config = config.get_etherscan_config_with_chain(Some(chain))?;

        let url = verifier_url.or_else(|| etherscan_config.as_ref().map(|c| c.api_url.as_str()));
        let etherscan_key =
            etherscan_key.or_else(|| etherscan_config.as_ref().map(|c| c.key.as_str()));

        let mut builder = Client::builder();

        builder = if let Some(url) = url {
            builder.with_api_url(url)?.with_url(url)?
        } else {
            builder.chain(chain.to_owned().try_into()?)?
        };

        builder
            .with_api_key(etherscan_key.unwrap_or_default())
            .build()
            .wrap_err("Failed to create etherscan client")
    }

    /// Creates the `VerifyContract` etherscan request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    pub async fn create_verify_request(
        &self,
        args: &VerifyArgs,
        config: Option<Config>,
    ) -> eyre::Result<VerifyContract> {
        let mut config =
            if let Some(config) = config { config } else { args.try_load_config_emit_warnings()? };

        config.libraries.extend(args.libraries.clone());

        let project = config.project()?;

        if !config.cache {
            eyre::ensure!(
                args.contract.path.is_some(),
                "If cache is disabled, contract info must be provided in the format <path>:<name>"
            );
            eyre::ensure!(
                args.constructor_args_path.is_none(),
                "Cache must be enabled in order to use the `--constructor-args-path` option"
            );
            eyre::ensure!(
                args.compiler_version.is_some() || config.solc.is_some(),
                "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
            );
        }

        let should_read_cache = args.contract.path.is_none() ||
            (args.compiler_version.is_none() && config.solc.is_none()) ||
            args.constructor_args_path.is_some();
        let (cached_entry, contract) = if config.cache && should_read_cache {
            let cache = project.read_cache_file()?;
            let cached_entry = crate::cmd::get_cached_entry_by_name(&cache, &args.contract.name)?;
            let contract: CompactContract =
                cache.read_artifact(cached_entry.0.clone(), &args.contract.name)?;
            (Some(cached_entry), Some(contract))
        } else {
            (None, None)
        };

        let contract_path = if let Some(ref path) = args.contract.path {
            project.root().join(path)
        } else {
            cached_entry.as_ref().unwrap().0.to_owned()
        };

        // check that the provided contract is part of the source dir
        if !contract_path.exists() {
            eyre::bail!("Contract {:?} does not exist.", contract_path);
        }

        let compiler_version = self.compiler_version(args, &config, &cached_entry)?;

        let (source, contract_name, code_format) =
            self.source_provider(args).source(args, &project, &contract_path, &compiler_version)?;

        let compiler_version = ensure_solc_build_metadata(compiler_version).await?;
        let compiler_version = format!("v{compiler_version}");
        let constructor_args = if let Some(ref constructor_args_path) = args.constructor_args_path {
            let abi = contract.unwrap().abi.ok_or(eyre!("Can't find ABI in cached artifact."))?;
            let constructor = abi
                .constructor()
                .ok_or(eyre!("Can't retrieve constructor info from artifact ABI."))?;
            #[allow(deprecated)]
            let func = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs.clone(),
                outputs: vec![],
                constant: None,
                state_mutability: Default::default(),
            };
            let encoded_args = encode_args(
                &func,
                &read_constructor_args_file(constructor_args_path.to_path_buf())?,
            )?
            .to_hex::<String>();
            Some(encoded_args[8..].into())
        } else {
            args.constructor_args.clone()
        };
        let mut verify_args =
            VerifyContract::new(args.address, contract_name, source, compiler_version)
                .constructor_arguments(constructor_args)
                .code_format(code_format);

        if code_format == CodeFormat::SingleFile {
            verify_args = if let Some(optimizations) = args.num_of_optimizations {
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
        args: &VerifyArgs,
        config: &Config,
        entry: &Option<(PathBuf, CacheEntry)>,
    ) -> eyre::Result<Version> {
        if let Some(ref version) = args.compiler_version {
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
}

trait EtherscanSourceProvider: Send + Sync + Debug {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> eyre::Result<(String, String, CodeFormat)>;
}

/// Given any solc [Version] return a [Version] with build metadata
///
/// # Example
///
/// ```ignore
/// use semver::{BuildMetadata, Version};
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
