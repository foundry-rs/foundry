use super::{provider::VerificationProvider, VerifyArgs, VerifyCheckArgs};
use crate::retry::RETRY_CHECK_ON_VERIFY;
use alloy_json_abi::Function;
use eyre::{eyre, Context, Result};
use foundry_block_explorers::{
    errors::EtherscanError,
    utils::lookup_compiler_version,
    verify::{CodeFormat, VerifyContract},
    Client,
};
use foundry_cli::utils::{get_cached_entry_by_name, read_constructor_args_file, LoadConfig};
use foundry_common::{abi::encode_function_args, retry::Retry};
use foundry_compilers::{
    artifacts::CompactContract, cache::CacheEntry, info::ContractInfo, Project, Solc,
};
use foundry_config::{Chain, Config, SolcReq};
use foundry_evm::hashbrown::HashSet;
use futures::FutureExt;
use once_cell::sync::Lazy;
use regex::Regex;
use semver::{BuildMetadata, Version};
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

mod flatten;
mod standard_json;

pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?P<commit>commit\.[0-9,a-f]{8})").unwrap());

pub static BASE_URL: &str = "https://www.oklink.com/";

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct OKLinkVerificationProvider {
    /// Memoized cached entry of the target contract
    cached_entry: Option<(PathBuf, CacheEntry, CompactContract)>,
}

/// The contract source provider for [OKLinkVerificationProvider]
///
/// Returns source, contract_name and the source [CodeFormat]
trait OklinkSourceProvider: Send + Sync + Debug {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> Result<(String, String, CodeFormat)>;
}

#[async_trait::async_trait]
impl VerificationProvider for OKLinkVerificationProvider {
    async fn preflight_check(&mut self, args: VerifyArgs) -> Result<()> {
        let _ = self.prepare_request(&args).await?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs) -> Result<()> {
        let (oklink, verify_args) = self.prepare_request(&args).await?;

        if !args.skip_is_verified_check && self.is_contract_verified(&oklink, &verify_args).await? {
            println!(
                "\nContract [{}] {:?} is already verified. Skipping verification.",
                verify_args.contract_name,
                verify_args.address.to_checksum(None)
            );
            return Ok(());
        }

        trace!(target: "forge::verify", ?verify_args, "submitting verification request");

        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| async {
                println!(
                    "\nSubmitting verification for [{}] {}.",
                    verify_args.contract_name, verify_args.address
                );

                let resp = oklink
                    .submit_contract_verification(&verify_args)
                    .await
                    .wrap_err_with(|| {
                        // valid json
                        let args = serde_json::to_string(&verify_args).unwrap();
                        error!(target: "forge::verify", ?args, "Failed to submit verification");
                        format!("Failed to submit contract verification, payload:\n{args}")
                    })?;

                trace!(target: "forge::verify", ?resp, "Received verification response");

                if resp.status == "0" {
                    if resp.result == "Contract source code already verified"
                        // specific for blockscout response
                        || resp.result == "Smart-contract already verified."
                    {
                        return Ok(None)
                    }

                    if resp.result.starts_with("Unable to locate ContractCode at") {
                        warn!("{}", resp.result);
                        return Err(eyre!("Oklink could not detect the deployment."))
                    }

                    warn!("Failed verify submission: {:?}", resp);
                    eprintln!(
                        "Encountered an error verifying this contract:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );
                    std::process::exit(1);
                }

                Ok(Some(resp))
            })
            .await?;

        if let Some(resp) = resp {
            println!(
                "Submitted contract for verification:\n\tResponse: `{}`\n\tGUID: `{}`",
                resp.message, resp.result,
            );

            if args.watch {
                let check_args = VerifyCheckArgs {
                    id: resp.result,
                    etherscan: args.etherscan,
                    oklink: args.oklink,
                    retry: RETRY_CHECK_ON_VERIFY,
                    verifier: args.verifier,
                };
                // return check_args.run().await
                return self.check(check_args).await;
            }
        } else {
            println!("Contract source code already verified");
        }

        Ok(())
    }

    /// Executes the command to check verification status on Oklink
    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let oklink = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.oklink.key().as_deref(),
        )?;
        let retry: Retry = args.retry.into();
        retry
            .run_async(|| {
                async {
                    let resp = oklink
                        .check_contract_verification_status(args.id.clone())
                        .await
                        .wrap_err("Failed to request verification status")?;

                    trace!(target: "forge::verify", ?resp, "Received verification response");

                    eprintln!(
                        "Contract verification status:\nResponse: `{}`\nDetails: `{}`",
                        resp.message, resp.result
                    );

                    if resp.result == "Pending in queue" {
                        return Err(eyre!("Verification is still pending...",));
                    }

                    if resp.result == "Unable to verify" {
                        return Err(eyre!("Unable to verify.",));
                    }

                    if resp.result == "Already Verified" {
                        println!("Contract source code already verified");
                        return Ok(());
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

impl OKLinkVerificationProvider {
    /// Create a source provider
    fn source_provider(&self, args: &VerifyArgs) -> Box<dyn OklinkSourceProvider> {
        if args.flatten {
            Box::new(flatten::OklinkFlattenedSource)
        } else {
            Box::new(standard_json::OklinkStandardJsonSource)
        }
    }

    /// Return the memoized cache entry for the target contract.
    /// Read the artifact from cache on first access.
    fn cache_entry(
        &mut self,
        project: &Project,
        contract: &ContractInfo,
    ) -> Result<&(PathBuf, CacheEntry, CompactContract)> {
        if let Some(ref entry) = self.cached_entry {
            return Ok(entry);
        }

        let cache = project.read_cache_file()?;
        let (path, entry) = if let Some(path) = contract.path.as_ref() {
            let path = project.root().join(path);
            (
                path.clone(),
                cache
                    .entry(&path)
                    .ok_or_else(|| {
                        eyre::eyre!(format!("Cache entry not found for {}", path.display()))
                    })?
                    .to_owned(),
            )
        } else {
            get_cached_entry_by_name(&cache, &contract.name)?
        };
        let contract: CompactContract = cache.read_artifact(path.clone(), &contract.name)?;
        Ok(self.cached_entry.insert((path, entry, contract)))
    }

    /// Configures the API request to the oklink API using the given [`VerifyArgs`].
    async fn prepare_request(&mut self, args: &VerifyArgs) -> Result<(Client, VerifyContract)> {
        let config = args.try_load_config_emit_warnings()?;
        let client = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.oklink.key().as_deref(),
        )?;
        let verify_args = self.create_verify_request(args, Some(config)).await?;
        Ok((client, verify_args))
    }

    /// Queries the oklink API to verify if the contract is already verified.
    async fn is_contract_verified(
        &self,
        oklink: &Client,
        verify_contract: &VerifyContract,
    ) -> Result<bool> {
        let check = oklink.contract_abi(verify_contract.address).await;
        if let Err(err) = check {
            match err {
                EtherscanError::ContractCodeNotVerified(_) => return Ok(false),
                error => return Err(error.into()),
            }
        }
        Ok(true)
    }

    /// Create an oklink client
    pub(crate) fn client(
        &self,
        chain: Chain,
        verifier_url: Option<&str>,
        oklink_key: Option<&str>,
    ) -> Result<Client> {
        let oklink_api_url = verifier_url.map(str::to_owned);
        let api_url = oklink_api_url.as_deref();

        let mut builder = Client::builder();
        builder = if let Some(api_url) = api_url {
            let api_url = api_url.trim_end_matches('/');
            builder.with_api_url(api_url)?.with_url(BASE_URL)?
        } else {
            builder.chain(chain)?
        };
        builder
            .with_api_key(oklink_key.unwrap_or_default())
            .build()
            .wrap_err("Failed to create oklink client")
    }

    /// Creates the `VerifyContract` oklink request in order to verify the contract
    ///
    /// If `--flatten` is set to `true` then this will send with [`CodeFormat::SingleFile`]
    /// otherwise this will use the [`CodeFormat::StandardJsonInput`]
    pub async fn create_verify_request(
        &mut self,
        args: &VerifyArgs,
        config: Option<Config>,
    ) -> Result<VerifyContract> {
        let mut config =
            if let Some(config) = config { config } else { args.try_load_config_emit_warnings()? };

        config.libraries.extend(args.libraries.clone());

        let project = config.project()?;

        let contract_path = self.contract_path(args, &project)?;
        let compiler_version = self.compiler_version(args, &config, &project)?;
        let (source, contract_name, code_format) =
            self.source_provider(args).source(args, &project, &contract_path, &compiler_version)?;

        let compiler_version = format!("v{}", ensure_solc_build_metadata(compiler_version).await?);
        let constructor_args = self.constructor_args(args, &project)?;
        let mut verify_args =
            VerifyContract::new(args.address, contract_name, source, compiler_version)
                .constructor_arguments(constructor_args)
                .code_format(code_format);

        if args.via_ir {
            // we explicitly set this __undocumented__ argument to true if provided by the user,
            // though this info is also available in the compiler settings of the standard json
            // object if standard json is used
            // unclear how oklink interprets this field in standard-json mode
            verify_args = verify_args.via_ir(true);
        }

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

    /// Get the target contract path. If it wasn't provided, attempt a lookup
    /// in cache. Validate the path indeed exists on disk.
    fn contract_path(&mut self, args: &VerifyArgs, project: &Project) -> Result<PathBuf> {
        let path = if let Some(path) = args.contract.path.as_ref() {
            project.root().join(path)
        } else {
            let (path, _, _) = self.cache_entry(project, &args.contract).wrap_err(
                "If cache is disabled, contract info must be provided in the format <path>:<name>",
            )?;
            path.to_owned()
        };

        // check that the provided contract is part of the source dir
        if !path.exists() {
            eyre::bail!("Contract {:?} does not exist.", path);
        }

        Ok(path)
    }

    /// Parse the compiler version.
    /// The priority desc:
    ///     1. Through CLI arg `--compiler-version`
    ///     2. `solc` defined in foundry.toml
    ///     3. The version contract was last compiled with.
    fn compiler_version(
        &mut self,
        args: &VerifyArgs,
        config: &Config,
        project: &Project,
    ) -> Result<Version> {
        if let Some(ref version) = args.compiler_version {
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

        let (_, entry, _) = self.cache_entry(project, &args.contract).wrap_err(
            "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
        )?;
        let artifacts = entry.artifacts_versions().collect::<Vec<_>>();

        if artifacts.is_empty() {
            eyre::bail!("No matching artifact found for {}", args.contract.name);
        }

        // ensure we have a single version
        let unique_versions = artifacts.iter().map(|a| a.0.to_string()).collect::<HashSet<_>>();
        if unique_versions.len() > 1 {
            let versions = unique_versions.into_iter().collect::<Vec<_>>();
            warn!("Ambiguous compiler versions found in cache: {}", versions.join(", "));
            eyre::bail!("Compiler version has to be set in `foundry.toml`. If the project was not deployed with foundry, specify the version through `--compiler-version` flag.")
        }

        // we have a unique version
        let mut version = artifacts[0].0.clone();
        version.build = match RE_BUILD_COMMIT.captures(version.build.as_str()) {
            Some(cap) => BuildMetadata::new(cap.name("commit").unwrap().as_str())?,
            _ => BuildMetadata::EMPTY,
        };

        Ok(version)
    }

    /// Return the optional encoded constructor arguments. If the path to
    /// constructor arguments was provided, read them and encode. Otherwise,
    /// return whatever was set in the [VerifyArgs] args.
    fn constructor_args(&mut self, args: &VerifyArgs, project: &Project) -> Result<Option<String>> {
        if let Some(ref constructor_args_path) = args.constructor_args_path {
            let (_, _, contract) = self.cache_entry(project, &args.contract).wrap_err(
                "Cache must be enabled in order to use the `--constructor-args-path` option",
            )?;
            let abi =
                contract.abi.as_ref().ok_or_else(|| eyre!("Can't find ABI in cached artifact."))?;
            let constructor = abi
                .constructor()
                .ok_or_else(|| eyre!("Can't retrieve constructor info from artifact ABI."))?;
            #[allow(deprecated)]
            let func = Function {
                name: "constructor".to_string(),
                inputs: constructor.inputs.clone(),
                outputs: vec![],
                state_mutability: alloy_json_abi::StateMutability::NonPayable,
            };
            let encoded_args = encode_function_args(
                &func,
                read_constructor_args_file(constructor_args_path.to_path_buf())?,
            )?;
            let encoded_args = hex::encode(encoded_args);
            return Ok(Some(encoded_args[8..].into()));
        }

        Ok(args.constructor_args.clone())
    }
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
async fn ensure_solc_build_metadata(version: Version) -> Result<Version> {
    if version.build != BuildMetadata::EMPTY {
        Ok(version)
    } else {
        Ok(lookup_compiler_version(&version).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use foundry_common::fs;
    use foundry_test_utils::forgetest_async;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_check() {
        let args: VerifyCheckArgs = VerifyCheckArgs::parse_from([
            "foundry-cli",
            "--verifier",
            "oklink",
            "--verifier-url",
            "https://www.oklink.com/api/explorer/v1/contract/verify/async/api/ethgoerli/",
            "--chain",
            "goerli",
            "0x1AbDA080f4b493672336289e0A580FB1783306FD",
        ]);

        let oklink = OKLinkVerificationProvider::default();
        let result = oklink.check(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_on_disabled_cache_and_missing_info() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let root_path = root.as_os_str().to_str().unwrap();

        let config = r"
                [profile.default]
                cache = false
            ";

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();

        let address = "0x1AbDA080f4b493672336289e0A580FB1783306FD";
        let contract_name = "Counter";
        let src_dir = "src";
        fs::create_dir_all(root.join(src_dir)).unwrap();
        let contract_path = format!("{src_dir}/Counter.sol");
        fs::write(root.join(&contract_path), "").unwrap();

        let mut oklink = OKLinkVerificationProvider::default();

        // No compiler argument
        let args = VerifyArgs::parse_from([
            "foundry-cli",
            address,
            &format!("{contract_path}:{contract_name}"),
            "--root",
            root_path,
        ]);

        let result = oklink.preflight_check(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
        );

        // No contract path
        let args =
            VerifyArgs::parse_from(["foundry-cli", address, contract_name, "--root", root_path]);

        let result = oklink.preflight_check(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "If cache is disabled, contract info must be provided in the format <path>:<name>"
        );

        // Constructor args path
        let args = VerifyArgs::parse_from([
            "foundry-cli",
            address,
            &format!("{contract_path}:{contract_name}"),
            "--constructor-args-path",
            ".",
            "--compiler-version",
            "0.8.15",
            "--root",
            root_path,
        ]);

        let result = oklink.preflight_check(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cache must be enabled in order to use the `--constructor-args-path` option",
        );
    }

    forgetest_async!(respects_path_for_duplicate, |prj, cmd| {
        prj.add_source("Counter1", "contract Counter {}").unwrap();
        prj.add_source("Counter2", "contract Counter {}").unwrap();

        cmd.args(["build", "--force"]).ensure_execute_success().unwrap();

        let args = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter1.sol:Counter",
            "--root",
            &prj.root().to_string_lossy(),
        ]);

        let mut oklink = OKLinkVerificationProvider::default();
        oklink.preflight_check(args).await.unwrap();
    });

    forgetest_async!(test_verify, |prj, cmd| {
        prj.add_source(
            "Counter1",
            "// SPDX-License-Identifier: UNLICENSED
            pragma solidity ^0.8.13;
            
            contract Counter {
                uint256 public number;
            
                function setNumber(uint256 newNumber) public {
                    number = newNumber;
                }
            
                function increment() public {
                    number++;
                }
            }
        ",
        )
        .unwrap();
        cmd.args(["build", "--force"]).ensure_execute_success().unwrap();

        let args = VerifyArgs::parse_from([
            "foundry-cli",
            "--verifier",
            "oklink",
            "--verifier-url",
            "https://www.oklink.com/api/explorer/v1/contract/verify/async/api/ethgoerli/",
            "--chain",
            "goerli",
            "--num-of-optimizations",
            "200",
            "--watch",
            "--compiler-version",
            "v0.8.19+commit.7dd6d404",
            "--skip-is-verified-check",
            "0x1AbDA080f4b493672336289e0A580FB1783306FD",
            "src/Counter1.sol:Counter",
            "--root",
            &prj.root().to_string_lossy(),
        ]);

        let mut oklink = OKLinkVerificationProvider::default();
        let result = oklink.verify(args).await;
        assert!(result.is_ok());
    });
}
