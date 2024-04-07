use super::{provider::VerificationProvider, VerifyArgs, VerifyCheckArgs};
use alloy_json_abi::Function;
use ethers_providers::Middleware;
use eyre::{eyre, Context, OptionExt, Result};
use foundry_block_explorers::{
    errors::EtherscanError,
    utils::lookup_compiler_version,
    verify::{CodeFormat, VerifyContract},
    Client, Response, ResponseData,
};
use foundry_cli::utils::{self, get_cached_entry_by_name, read_constructor_args_file, LoadConfig};
use foundry_common::{abi::encode_function_args, retry::Retry, types::ToEthers};
use foundry_compilers::{
    artifacts::{BytecodeObject, CompactContract, StandardJsonCompilerInput},
    cache::CacheEntry,
    info::ContractInfo,
    Artifact, Project, Solc,
};
use foundry_config::{Chain, Config, SolcReq};
use foundry_evm::{constants::DEFAULT_CREATE2_DEPLOYER, hashbrown::HashSet};
use futures::FutureExt;
use once_cell::sync::Lazy;
use regex::Regex;
use semver::{BuildMetadata, Version};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
};

pub static OKLINK_URL: &str = "https://www.oklink.com/";
pub static RE_BUILD_COMMIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?P<commit>commit\.[0-9,a-f]{8})").unwrap());

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct OklinkVerificationProvider {
    /// Memoized cached entry of the target contract
    cached_entry: Option<(PathBuf, CacheEntry, CompactContract)>,
}

#[derive(Clone, Debug, Serialize)]
struct Query<T: Serialize> {
    #[serde(skip_serializing_if = "Option::is_none")]
    apikey: Option<String>,
    module: String,
    action: String,
    #[serde(flatten)]
    other: T,
}

#[async_trait::async_trait]
impl VerificationProvider for OklinkVerificationProvider {
    async fn preflight_check(&mut self, args: VerifyArgs) -> Result<()> {
        let _ = self.prepare_request(&args).await?;
        Ok(())
    }

    async fn verify(&mut self, args: VerifyArgs) -> Result<()> {
        let (_, verify_args) = self.prepare_request(&args).await?;

        trace!(target: "forge::verify", ?verify_args, "submitting verification request");
        let client = reqwest::Client::new();
        let retry: Retry = args.retry.into();
        let resp = retry
            .run_async(|| async {
                println!(
                    "\nSubmitting verification for [{}] {}.",
                    verify_args.contract_name, verify_args.address
                );
                let api_key: Option<String> = args.etherscan.key().clone();
                let body = create_query(api_key,"contract".to_string(), "verifysourcecode".to_string(), &verify_args);
                debug!("body {:?}", body);
                let resp = client
                .post(args.verifier.verifier_url.clone().unwrap())
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Ok-Access-Key", &args.etherscan.key().unwrap())
                .header("x-apiKey", &args.etherscan.key().unwrap())
                .form(&body)
                .send()
                .await?
                .text()
                .await
                .wrap_err_with(|| {
                            // valid json
                            let args = serde_json::to_string(&verify_args).unwrap();
                            error!(target: "forge::verify", ?args, "Failed to submit verification");
                            format!("Failed to submit contract verification, payload:\n{args}")
                        })?;

                debug!(target: "forge::verify", ?resp, "Received verification response");
                let resp = sanitize_response::<String>(&resp)?;

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
                "Submitted contract for verification:\n\tResponse: `{}`\n\tGUID: `{}`\n\tURL: {}",
                resp.message, resp.result, OKLINK_URL
            );
        } else {
            println!("Contract source code already verified");
        }

        Ok(())
    }

    /// Executes the command to check verification status on Oklink
    async fn check(&self, args: VerifyCheckArgs) -> Result<()> {
        let retry: Retry = args.retry.into();
        let client = reqwest::Client::new();
        let api_key: Option<String> = args.etherscan.key().clone();
        let body = create_query(
            api_key,
            "contract".to_string(),
            "checkverifystatus".to_string(),
            HashMap::from([("guid", args.id.clone())]),
        );
        debug!("body {:?}", body);

        retry
            .run_async(|| {
                async {
                    let resp = client
                        .post(args.verifier.verifier_url.clone().unwrap())
                        .header("Content-Type", "application/x-www-form-urlencoded")
                        .header("Ok-Access-Key", &args.etherscan.key().unwrap())
                        .header("x-apiKey", &args.etherscan.key().unwrap())
                        .form(&body)
                        .send()
                        .await?
                        .text()
                        .await
                        .wrap_err("Failed to request verification status")?;

                    debug!(target: "forge::verify", ?resp, "Received verification response");
                    let resp = sanitize_response::<String>(&resp)?;

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

impl OklinkVerificationProvider {
    fn source(
        &self,
        args: &VerifyArgs,
        project: &Project,
        target: &Path,
        version: &Version,
    ) -> Result<(String, String, CodeFormat)> {
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

        // remove all incompatible settings
        input.settings.sanitize(version);

        let source =
            serde_json::to_string(&input).wrap_err("Failed to parse standard json input")?;

        trace!(target: "forge::verify", standard_json=source, "determined standard json input");

        let name = format!(
            "{}:{}",
            target.strip_prefix(project.root()).unwrap_or(target).display(),
            args.contract.name.clone()
        );
        Ok((source, name, CodeFormat::StandardJsonInput))
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
        let oklink = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key().as_deref(),
            &config,
        )?;
        let verify_args = self.create_verify_request(args, Some(config)).await?;

        Ok((oklink, verify_args))
    }

    /// Create an oklink client
    pub(crate) fn client(
        &self,
        chain: Chain,
        verifier_url: Option<&str>,
        oklink_key: Option<&str>,
        config: &Config,
    ) -> Result<Client> {
        let oklink_key = oklink_key.unwrap();

        let mut builder = Client::builder();

        builder = if let Some(api_url) = verifier_url {
            let api_url = api_url.trim_end_matches('/');
            builder.with_api_url(api_url)?.with_url(OKLINK_URL)?
        } else {
            eyre::bail!("must pass the verifier URL")
        };
        debug!("{:?}", builder);

        builder.with_api_key(oklink_key).build().wrap_err("Failed to create oklink client")
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
            self.source(args, &project, &contract_path, &compiler_version)?;

        let compiler_version = format!("v{}", ensure_solc_build_metadata(compiler_version).await?);
        let constructor_args = self.constructor_args(args, &project, &config).await?;
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
    async fn constructor_args(
        &mut self,
        args: &VerifyArgs,
        project: &Project,
        config: &Config,
    ) -> Result<Option<String>> {
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
        if args.guess_constructor_args {
            return Ok(Some(self.guess_constructor_args(args, project, config).await?));
        }

        Ok(args.constructor_args.clone())
    }

    /// Uses Oklink API to fetch contract creation transaction.
    /// If transaction is a create transaction or a invocation of default CREATE2 deployer, tries to
    /// match provided creation code with local bytecode of the target contract.
    /// If bytecode match, returns latest bytes of on-chain creation code as constructor arguments.
    async fn guess_constructor_args(
        &mut self,
        args: &VerifyArgs,
        project: &Project,
        config: &Config,
    ) -> Result<String> {
        let provider = utils::get_provider(config)?;
        let client = self.client(
            args.etherscan.chain.unwrap_or_default(),
            args.verifier.verifier_url.as_deref(),
            args.etherscan.key.as_deref(),
            config,
        )?;

        let creation_data = client.contract_creation_data(args.address).await?;
        let transaction = provider
            .get_transaction(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction data from RPC")?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction receipt from RPC")?;

        let maybe_creation_code: &[u8];

        if receipt.contract_address == Some(args.address.to_ethers()) {
            maybe_creation_code = &transaction.input;
        } else if transaction.to == Some(DEFAULT_CREATE2_DEPLOYER.to_ethers()) {
            maybe_creation_code = &transaction.input[32..];
        } else {
            eyre::bail!("Fetching of constructor arguments is not supported for contracts created by contracts")
        }

        let contract_path = self.contract_path(args, project)?.to_string_lossy().into_owned();
        let output = project.compile()?;
        let artifact = output
            .find(contract_path, &args.contract.name)
            .ok_or_eyre("Contract artifact wasn't found locally")?;
        let bytecode = artifact
            .get_bytecode_object()
            .ok_or_eyre("Contract artifact does not contain bytecode")?;

        let bytecode = match bytecode.as_ref() {
            BytecodeObject::Bytecode(bytes) => Ok(bytes),
            BytecodeObject::Unlinked(_) => {
                Err(eyre!("You have to provide correct libraries to use --guess-constructor-args"))
            }
        }?;

        if maybe_creation_code.starts_with(bytecode) {
            let constructor_args = &maybe_creation_code[bytecode.len()..];
            Ok(hex::encode(constructor_args))
        } else {
            eyre::bail!("Local bytecode doesn't match on-chain bytecode")
        }
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
fn create_query<T: Serialize>(
    api_key: Option<String>,
    module: String,
    action: String,
    other: T,
) -> Query<T> {
    Query { apikey: api_key, module, action, other }
}
fn sanitize_response<T: DeserializeOwned>(res: impl AsRef<str>) -> Result<Response<T>> {
    let res = res.as_ref();
    let res: ResponseData<T> = serde_json::from_str(res).map_err(|error| {
        error!(target: "etherscan", ?res, "Failed to deserialize response: {}", error);
        if res == "Page not found" {
            EtherscanError::PageNotFound
        } else {
            EtherscanError::Serde { error, content: res.to_string() }
        }
    })?;

    match res {
        ResponseData::Error { result, message, status } => {
            if let Some(ref result) = result {
                if result.starts_with("Max rate limit reached") {
                    return Err(EtherscanError::RateLimitExceeded.into());
                } else if result.to_lowercase() == "invalid api key" {
                    return Err(EtherscanError::InvalidApiKey.into());
                }
            }
            Err(EtherscanError::ErrorResponse { status, message, result }.into())
        }
        ResponseData::Success(res) => Ok(res),
    }
}
