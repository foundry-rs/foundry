mod flatten;
mod standard_json;

use super::{VerifyArgs, VerifyCheckArgs};
use crate::cmd::{
    forge::verify::provider::VerificationProvider, get_cached_entry_by_name,
    read_constructor_args_file, retry::RETRY_CHECK_ON_VERIFY, LoadConfig,
};
use cast::SimpleCast;
use ethers::{
    abi::Function,
    etherscan::{
        utils::lookup_compiler_version,
        verify::{CodeFormat, VerifyContract},
        Client,
    },
    prelude::errors::EtherscanError,
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
    Lazy::new(|| Regex::new(r"(?P<commit>commit\.[0-9,a-f]{8})").unwrap());

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EtherscanVerificationProvider {
    /// Memoized cached entry of the target contract
    cached_entry: Option<(PathBuf, CacheEntry, CompactContract)>,
}

/// The contract source provider for [EtherscanVerificationProvider]
///
/// Returns source, contract_name and the source [CodeFormat]
trait EtherscanSourceProvider: Send + Sync + Debug {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> eyre::Result<(String, String, CodeFormat)>;
}

#[async_trait::async_trait]
impl VerificationProvider for EtherscanVerificationProvider {
    async fn preflight_check(&mut self, args: VerifyArgs) -> eyre::Result<()> {
        let _ = self.prepare_request(&args).await?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs) -> eyre::Result<()> {
        let (etherscan, verify_args) = self.prepare_request(&args).await?;

        if self.is_contract_verified(&etherscan, &verify_args).await? {
            println!(
                "\nContract [{}] {:?} is already verified. Skipping verification.",
                verify_args.contract_name,
                SimpleCast::to_checksum_address(&verify_args.address)
            );

            return Ok(())
        }

        trace!(target : "forge::verify", ?verify_args,  "submitting verification request");

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
                        error!(target : "forge::verify",  ?args, "Failed to submit verification");
                        format!("Failed to submit contract verification, payload:\n{args}")
                    })?;

                trace!(target : "forge::verify",  ?resp, "Received verification response");

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
                    etherscan: args.etherscan,
                    retry: RETRY_CHECK_ON_VERIFY,
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
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key.as_deref(),
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

                    trace!(target : "forge::verify",  ?resp, "Received verification response");

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

    /// Return the memoized cache entry for the target contract.
    /// Read the artifact from cache on first access.
    fn cache_entry(
        &mut self,
        project: &Project,
        contract_name: &str,
    ) -> eyre::Result<&(PathBuf, CacheEntry, CompactContract)> {
        if let Some(ref entry) = self.cached_entry {
            return Ok(entry)
        }

        let cache = project.read_cache_file()?;
        let (path, entry) = get_cached_entry_by_name(&cache, contract_name)?;
        let contract: CompactContract = cache.read_artifact(path.clone(), contract_name)?;
        Ok(self.cached_entry.insert((path, entry, contract)))
    }

    /// Configures the API request to the etherscan API using the given [`VerifyArgs`].
    async fn prepare_request(
        &mut self,
        args: &VerifyArgs,
    ) -> eyre::Result<(Client, VerifyContract)> {
        let config = args.try_load_config_emit_warnings()?;
        let etherscan = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key.as_deref(),
            &config,
        )?;
        let verify_args = self.create_verify_request(args, Some(config)).await?;

        Ok((etherscan, verify_args))
    }

    /// Queries the etherscan API to verify if the contract is already verified.
    async fn is_contract_verified(
        &self,
        etherscan: &Client,
        verify_contract: &VerifyContract,
    ) -> eyre::Result<bool> {
        let check = etherscan.contract_abi(verify_contract.address).await;

        if let Err(err) = check {
            match err {
                EtherscanError::ContractCodeNotVerified(_) => return Ok(false),
                error => return Err(error.into()),
            }
        }

        Ok(true)
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

        let api_url =
            verifier_url.or_else(|| etherscan_config.as_ref().map(|c| c.api_url.as_str()));
        let base_url = etherscan_config
            .as_ref()
            .and_then(|c| c.browser_url.as_deref())
            .or_else(|| chain.etherscan_urls().map(|urls| urls.1));

        let etherscan_key =
            etherscan_key.or_else(|| etherscan_config.as_ref().map(|c| c.key.as_str()));

        let mut builder = Client::builder();

        builder = if let Some(api_url) = api_url {
            builder.with_api_url(api_url)?.with_url(base_url.unwrap_or(api_url))?
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
        &mut self,
        args: &VerifyArgs,
        config: Option<Config>,
    ) -> eyre::Result<VerifyContract> {
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
    fn contract_path(&mut self, args: &VerifyArgs, project: &Project) -> eyre::Result<PathBuf> {
        let path = match args.contract.path.as_ref() {
            Some(path) => project.root().join(path),
            None => {
                let (path, _, _) = self.cache_entry(project, &args.contract.name).wrap_err(
                    "If cache is disabled, contract info must be provided in the format <path>:<name>",
                )?;
                path.to_owned()
            }
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

        let (_, entry, _) = self.cache_entry(project, &args.contract.name).wrap_err(
            "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
        )?;
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
            warn!("No artifacts detected")
        } else {
            let versions = artifacts.iter().map(|a| a.0.to_string()).collect::<Vec<_>>();
            warn!("Ambiguous compiler versions found in cache: {}", versions.join(", "));
        }

        eyre::bail!("Compiler version has to be set in `foundry.toml`. If the project was not deployed with foundry, specify the version through `--compiler-version` flag.")
    }

    /// Return the optional encoded constructor arguments. If the path to
    /// constructor arguments was provided, read them and encode. Otherwise,
    /// return whatever was set in the [VerifyArgs] args.
    fn constructor_args(
        &mut self,
        args: &VerifyArgs,
        project: &Project,
    ) -> eyre::Result<Option<String>> {
        if let Some(ref constructor_args_path) = args.constructor_args_path {
            let (_, _, contract) = self.cache_entry(project, &args.contract.name).wrap_err(
                "Cache must be enabled in order to use the `--constructor-args-path` option",
            )?;
            let abi = contract.abi.as_ref().ok_or(eyre!("Can't find ABI in cached artifact."))?;
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
            return Ok(Some(encoded_args[8..].into()))
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
async fn ensure_solc_build_metadata(version: Version) -> eyre::Result<Version> {
    if version.build != BuildMetadata::EMPTY {
        Ok(version)
    } else {
        Ok(lookup_compiler_version(&version).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::LoadConfig;
    use clap::Parser;
    use foundry_cli_test_utils::tempfile::tempdir;
    use foundry_common::fs;

    #[test]
    fn can_extract_etherscan_verify_config() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

                [etherscan]
                mumbai = { key = "dummykey", chain = 80001, url = "https://api-testnet.polygonscan.com/" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();

        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0xd8509bee9c9bf012282ad33aba0d87241baf5064",
            "src/Counter.sol:Counter",
            "--chain",
            "mumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config();

        let etherscan = EtherscanVerificationProvider::default();
        let client = etherscan
            .client(
                args.etherscan.chain.unwrap_or_default(),
                args.verifier.verifier_url.as_deref(),
                args.etherscan.key.as_deref(),
                &config,
            )
            .unwrap();
        assert_eq!(client.etherscan_api_url().as_str(), "https://api-testnet.polygonscan.com/");

        assert!(format!("{client:?}").contains("dummykey"));

        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0xd8509bee9c9bf012282ad33aba0d87241baf5064",
            "src/Counter.sol:Counter",
            "--chain",
            "mumbai",
            "--verifier-url",
            "https://verifier-url.com/",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config();

        let etherscan = EtherscanVerificationProvider::default();
        let client = etherscan
            .client(
                args.etherscan.chain.unwrap_or_default(),
                args.verifier.verifier_url.as_deref(),
                args.etherscan.key.as_deref(),
                &config,
            )
            .unwrap();
        assert_eq!(client.etherscan_api_url().as_str(), "https://verifier-url.com/");
        assert!(format!("{client:?}").contains("dummykey"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_on_disabled_cache_and_missing_info() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let root_path = root.as_os_str().to_str().unwrap();

        let config = r#"
                [profile.default]
                cache = false
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();

        let address = "0xd8509bee9c9bf012282ad33aba0d87241baf5064";
        let contract_name = "Counter";
        let src_dir = "src";
        fs::create_dir_all(root.join(src_dir)).unwrap();
        let contract_path = format!("{src_dir}/Counter.sol");
        fs::write(root.join(&contract_path), "").unwrap();

        let mut etherscan = EtherscanVerificationProvider::default();

        // No compiler argument
        let args = VerifyArgs::parse_from([
            "foundry-cli",
            address,
            &format!("{contract_path}:{contract_name}"),
            "--root",
            root_path,
        ]);

        let result = etherscan.preflight_check(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
        );

        // No contract path
        let args =
            VerifyArgs::parse_from(["foundry-cli", address, contract_name, "--root", root_path]);

        let result = etherscan.preflight_check(args).await;
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

        let result = etherscan.preflight_check(args).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cache must be enabled in order to use the `--constructor-args-path` option",
        );
    }
}
