use super::{VerifyArgs, VerifyCheckArgs};
use crate::cmd::{
    forge::verify::provider::VerificationProvider, read_constructor_args_file,
    retry::RETRY_CHECK_ON_VERIFY, LoadConfig,
};
use async_trait::async_trait;
use cast::SimpleCast;
use ethers::{
    abi::Function,
    etherscan::{
        utils::lookup_compiler_version,
        verify::{CodeFormat, VerifyContract},
        Client,
    },
    prelude::artifacts::StandardJsonCompilerInput,
    solc::{
        artifacts::{BytecodeHash, CompactContract, Source},
        cache::CacheEntry,
        AggregatedCompilerOutput, CompilerInput, Project, Solc,
    },
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
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tracing::{error, trace, warn};

pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?P<commit>commit\.[0-9,a-f]{8})"#).unwrap());

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EtherscanVerificationProvider;

#[async_trait]
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
            if args.contract.path.is_none() {
                eyre::bail!(
                    "If cache is disabled, contract info must be provided in the format <path>:<name>"
                );
            } else if args.constructor_args_path.is_some() {
                eyre::bail!(
                    "Cache must be enabled in order to use the `--constructor-args-path` option"
                );
            }
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

        let (source, contract_name, code_format) = if args.flatten {
            self.flattened_source(
                args,
                &project,
                &contract_path,
                &compiler_version,
                &contract_path,
            )?
        } else {
            self.standard_json_source(args, &project, &contract_path, &compiler_version)?
        };

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
            eprintln!("{}", o.diagnostics(&[], Default::default()));

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
        args: &VerifyArgs,
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

        if !args.force {
            // solc dry run of flattened code
            self.check_flattened(source.clone(), version, contract_path).map_err(|err| {
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
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> eyre::Result<(String, String, CodeFormat)> {
        let mut input: StandardJsonCompilerInput = project
            .standard_json_input(target)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(version);

        input.settings.libraries.libs = input
            .settings
            .libraries
            .libs
            .into_iter()
            .map(|(f, libs)| (f.strip_prefix(project.root()).unwrap_or(&f).to_path_buf(), libs))
            .collect();

        // TODO: make sanitization logic shared between types in ethers
        let input: StandardJsonCompilerInput = CompilerInput::from(input).sanitized(version).into();

        let source =
            serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;

        trace!(standard_json = source, target = "forge::verify", "determined standard json input");

        let name = format!(
            "{}:{}",
            target.strip_prefix(project.root()).unwrap_or(target).display(),
            args.contract.name.clone()
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
