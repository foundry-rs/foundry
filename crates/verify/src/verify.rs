//! The `forge verify-bytecode` command.

use crate::{
    RetryArgs,
    etherscan::EtherscanVerificationProvider,
    provider::{VerificationContext, VerificationProvider, VerificationProviderType},
    sourcify::SourcifyVerificationProvider,
    utils::wrap_verifier_url_error,
};
use alloy_primitives::{Address, TxHash, map::HashSet};
use alloy_provider::Provider;
use clap::{Parser, ValueEnum, ValueHint};
use eyre::{Context, Result};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig},
};
use foundry_common::{ContractsByArtifact, compile::ProjectCompiler};
use foundry_compilers::{artifacts::EvmVersion, compilers::solc::Solc, info::ContractInfo};
use foundry_config::{
    Chain, Config, SolcReq,
    figment::{
        Error, Metadata, Profile, Provider as FigmentProvider,
        value::{Dict, Map, Value},
    },
    impl_figment_convert, impl_figment_convert_cast,
};
use itertools::Itertools;
use reqwest::{Client, StatusCode, Url};
use semver::BuildMetadata;
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VerifierCredentialProbe {
    Accepted,
    InvalidApiKey,
    Inconclusive,
}

#[derive(Debug, Deserialize)]
struct EtherscanProbeResponse {
    status: String,
    result: Option<serde_json::Value>,
}

fn verifier_credential_probe_query(api_key: Option<&str>) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("module", "contract".to_string()),
        ("action", "getabi".to_string()),
        ("address", Address::ZERO.to_string()),
    ];
    if let Some(api_key) = api_key {
        query.push(("apikey", api_key.to_string()));
    }
    query
}

fn classify_verifier_credential_response(
    status: StatusCode,
    body: &str,
) -> VerifierCredentialProbe {
    let lower = body.to_lowercase();
    if lower.contains("invalid api key") || lower.contains("invalid_api_key") {
        return VerifierCredentialProbe::InvalidApiKey;
    }

    if lower.contains("contract source code not verified")
        || lower.contains("contract not found")
        || lower.contains("contract was not found")
    {
        return VerifierCredentialProbe::Accepted;
    }

    if status == StatusCode::UNAUTHORIZED {
        return VerifierCredentialProbe::InvalidApiKey;
    }

    if !status.is_success()
        || lower.contains("max rate limit reached")
        || lower.contains("sorry, you have been blocked")
        || lower.contains("checking if the site connection is secure")
    {
        return VerifierCredentialProbe::Inconclusive;
    }

    match serde_json::from_str::<EtherscanProbeResponse>(body) {
        Ok(resp) if resp.status == "1" => VerifierCredentialProbe::Accepted,
        Ok(resp) => resp
            .result
            .and_then(|result| result.as_str().map(str::to_lowercase))
            .map(|result| {
                if result.contains("invalid api key") || result.contains("invalid_api_key") {
                    VerifierCredentialProbe::InvalidApiKey
                } else if result.contains("max rate limit reached") {
                    VerifierCredentialProbe::Inconclusive
                } else if result.contains("contract source code not verified")
                    || result.contains("contract not found")
                    || result.contains("contract was not found")
                {
                    VerifierCredentialProbe::Accepted
                } else {
                    VerifierCredentialProbe::Inconclusive
                }
            })
            .unwrap_or(VerifierCredentialProbe::Inconclusive),
        Err(_) => VerifierCredentialProbe::Inconclusive,
    }
}

fn parse_http_verifier_url(url: &str, label: &str) -> Result<Url> {
    let url = Url::parse(url).wrap_err_with(|| format!("invalid {label} URL `{url}`"))?;
    if !matches!(url.scheme(), "http" | "https") {
        eyre::bail!("invalid {label} URL `{url}`: URL scheme must be http or https");
    }
    Ok(url)
}

async fn probe_verifier_credentials(
    url: Url,
    api_key: Option<&str>,
) -> Result<VerifierCredentialProbe, reqwest::Error> {
    let resp = Client::new()
        .get(url)
        .query(&verifier_credential_probe_query(api_key))
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    Ok(classify_verifier_credential_response(status, &body))
}

/// The programming language used for smart contract development.
///
/// This enum represents the supported contract languages for verification.
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ContractLanguage {
    /// Solidity programming language
    Solidity,
    /// Vyper programming language
    Vyper,
}

/// Verification provider arguments
#[derive(Clone, Debug, Default, Parser)]
pub struct VerifierArgs {
    /// The contract verification provider to use.
    #[arg(long, help_heading = "Verifier options", value_enum)]
    pub verifier: Option<VerificationProviderType>,

    /// The verifier API KEY, if using a custom provider.
    #[arg(long, help_heading = "Verifier options", env = "VERIFIER_API_KEY")]
    pub verifier_api_key: Option<String>,

    /// The verifier URL, if using a custom provider.
    #[arg(long, help_heading = "Verifier options", env = "VERIFIER_URL")]
    pub verifier_url: Option<String>,
}

impl VerifierArgs {
    /// Returns the effective verifier type, defaulting to Sourcify if not explicitly set.
    ///
    /// Note: this is the *defaulted CLI value*, not the actually-selected provider after
    /// considering `ETHERSCAN_API_KEY` / chain support. Use [`Self::resolve`] for that.
    pub fn effective_type(&self) -> VerificationProviderType {
        self.verifier.unwrap_or_default()
    }

    /// Returns true if `--verifier` was explicitly provided by the user.
    pub const fn is_explicitly_set(&self) -> bool {
        self.verifier.is_some()
    }

    /// Resolves the API key with consistent precedence: explicit `--verifier-api-key` first,
    /// then the etherscan config key.
    pub fn resolve_api_key<'a>(&'a self, etherscan_key: Option<&'a str>) -> Option<&'a str> {
        self.verifier_api_key.as_deref().or(etherscan_key)
    }

    /// Makes a lightweight network call to validate that credentials are accepted by the verifier.
    ///
    /// `api_key` must be the already-merged API key (CLI `--verifier-api-key` takes
    /// precedence over config), as returned by [`Self::resolve_api_key`].
    pub async fn check_credentials(
        &self,
        api_key: Option<&str>,
        chain: Chain,
        config: &Config,
    ) -> eyre::Result<()> {
        let resolved = self.resolve(api_key, Some(chain));
        match resolved {
            VerificationProviderType::Etherscan
            | VerificationProviderType::Blockscout
            | VerificationProviderType::Oklink => {
                let etherscan_opts =
                    EtherscanOpts { key: api_key.map(str::to_owned), chain: Some(chain) };
                let client = EtherscanVerificationProvider::default().client(
                    &etherscan_opts,
                    self,
                    config,
                )?;
                match tokio::time::timeout(
                    Duration::from_secs(10),
                    probe_verifier_credentials(
                        client.etherscan_api_url().clone(),
                        client.api_key(),
                    ),
                )
                .await
                {
                    Err(_) => {
                        sh_warn!("verifier credential check timed out, proceeding anyway")?;
                    }
                    Ok(Ok(VerifierCredentialProbe::Accepted)) => {}
                    Ok(Ok(VerifierCredentialProbe::InvalidApiKey)) => {
                        eyre::bail!("verifier credential check failed: invalid API key");
                    }
                    Ok(Ok(VerifierCredentialProbe::Inconclusive) | Err(_)) => {
                        sh_warn!("verifier credential check inconclusive, proceeding anyway")?;
                    }
                }
            }
            VerificationProviderType::Custom => {
                // Custom verifiers may return Etherscan-shaped responses (HTTP 200 with a JSON
                // body) or standard HTTP auth errors (401/403). Check both.
                if let Some(url) = &self.verifier_url {
                    let url = parse_http_verifier_url(url, "verifier")?;
                    match probe_verifier_credentials(url, api_key).await {
                        Err(_) => {
                            sh_warn!("verifier credential check failed, proceeding anyway")?;
                        }
                        Ok(
                            VerifierCredentialProbe::Accepted
                            | VerifierCredentialProbe::Inconclusive,
                        ) => {}
                        Ok(VerifierCredentialProbe::InvalidApiKey) => {
                            eyre::bail!("verifier credential check failed: invalid API key")
                        }
                    }
                }
            }
            VerificationProviderType::Sourcify => {
                // Only probe custom URLs; the default public endpoint is assumed reachable.
                if let Some(url) = &self.verifier_url {
                    let url = parse_http_verifier_url(url, "Sourcify")?;
                    match Client::new()
                        .get(url.clone())
                        .timeout(Duration::from_secs(10))
                        .send()
                        .await
                    {
                        Err(_) => {
                            sh_warn!(
                                "Sourcify URL `{url}` could not be reached, proceeding anyway"
                            )?;
                        }
                        Ok(resp) => {
                            let status = resp.status();
                            if !status.is_success() && status != StatusCode::NOT_FOUND {
                                sh_warn!(
                                    "Sourcify URL `{url}` returned HTTP {status}, proceeding anyway"
                                )?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolves the actual verification provider that will be used at runtime, taking into
    /// account the explicit `--verifier`, the presence of `ETHERSCAN_API_KEY`, and whether the
    /// target chain has a known Etherscan API URL.
    ///
    /// Resolution rules (mirrors [`VerificationProviderType::client`]):
    /// 1. If `--verifier` was explicitly set, that wins.
    /// 2. Otherwise, if an Etherscan API key is set AND the chain is supported (or a custom
    ///    `--verifier-url` is provided), use Etherscan.
    /// 3. Otherwise, fall back to Sourcify.
    pub fn resolve(
        &self,
        etherscan_key: Option<&str>,
        chain: Option<Chain>,
    ) -> VerificationProviderType {
        if let Some(v) = self.verifier {
            return v;
        }
        let has_key = etherscan_key.is_some_and(|k| !k.is_empty());
        // Custom-Sourcify chains (e.g. Tempo) register Sourcify-compatible URLs under
        // etherscan_urls() but are NOT real Etherscan chains. Skip the implicit-Etherscan path
        // entirely for them; the caller must use `--verifier etherscan` explicitly to override.
        if has_key && !chain.is_some_and(|c| c.is_custom_sourcify()) {
            let chain_has_etherscan_url = chain.is_none_or(|c| c.etherscan_urls().is_some());
            if chain_has_etherscan_url || self.verifier_url.is_some() {
                return VerificationProviderType::Etherscan;
            }
        }
        VerificationProviderType::Sourcify
    }
}

/// CLI arguments for `forge verify-contract`.
#[derive(Clone, Debug, Parser)]
pub struct VerifyArgs {
    /// The address of the contract to verify.
    pub address: Address,

    /// The contract identifier in the form `<path>:<contractname>`.
    pub contract: Option<ContractInfo>,

    /// The ABI-encoded constructor arguments. Only for Etherscan.
    #[arg(
        long,
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        visible_alias = "encoded-constructor-args"
    )]
    pub constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub constructor_args_path: Option<PathBuf>,

    /// Try to extract constructor arguments from on-chain creation code.
    #[arg(long)]
    pub guess_constructor_args: bool,

    /// The hash of the transaction which created the contract. Optional for Sourcify.
    #[arg(long)]
    pub creation_transaction_hash: Option<TxHash>,

    /// The `solc` version to use to build the smart contract.
    #[arg(long, value_name = "VERSION")]
    pub compiler_version: Option<String>,

    /// The compilation profile to use to build the smart contract.
    #[arg(long, value_name = "PROFILE_NAME")]
    pub compilation_profile: Option<String>,

    /// The number of optimization runs used to build the smart contract.
    #[arg(long, visible_alias = "optimizer-runs", value_name = "NUM")]
    pub num_of_optimizations: Option<usize>,

    /// Flatten the source code before verifying.
    #[arg(long)]
    pub flatten: bool,

    /// Do not compile the flattened smart contract before verifying (if --flatten is passed).
    #[arg(short, long)]
    pub force: bool,

    /// Do not check if the contract is already verified before verifying.
    #[arg(long)]
    pub skip_is_verified_check: bool,

    /// Wait for verification result after submission.
    #[arg(long)]
    pub watch: bool,

    /// Set pre-linked libraries.
    #[arg(long, help_heading = "Linker options", env = "DAPP_LIBRARIES")]
    pub libraries: Vec<String>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Prints the standard json compiler input.
    ///
    /// The standard json compiler input can be used to manually submit contract verification in
    /// the browser.
    #[arg(long, conflicts_with = "flatten")]
    pub show_standard_json_input: bool,

    /// Use the Yul intermediate representation compilation pipeline.
    #[arg(long)]
    pub via_ir: bool,

    /// The Etherscan license type code to include with the verification request.
    ///
    /// See Etherscan's supported `licenseType` values. This is only used for Etherscan-style
    /// verifiers.
    #[arg(long, value_name = "CODE", help_heading = "Verifier options")]
    pub license_type: Option<String>,

    /// The EVM version to use.
    ///
    /// Overrides the version specified in the config.
    #[arg(long)]
    pub evm_version: Option<EvmVersion>,

    /// Do not auto-detect the `solc` version.
    #[arg(long, help_heading = "Compiler options")]
    pub no_auto_detect: bool,

    /// Specify the solc version, or a path to a local solc, to build with.
    ///
    /// Valid values are in the format `x.y.z`, `solc:x.y.z` or `path/to/solc`.
    #[arg(long = "use", help_heading = "Compiler options", value_name = "SOLC_VERSION")]
    pub use_solc: Option<String>,

    #[command(flatten)]
    pub etherscan: EtherscanOpts,

    #[command(flatten)]
    pub rpc: RpcOpts,

    #[command(flatten)]
    pub retry: RetryArgs,

    #[command(flatten)]
    pub verifier: VerifierArgs,

    /// The contract language (`solidity` or `vyper`).
    ///
    /// Defaults to `solidity` if none provided.
    #[arg(long, value_enum)]
    pub language: Option<ContractLanguage>,
}

impl_figment_convert!(VerifyArgs);

impl FigmentProvider for VerifyArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Verify Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = self.etherscan.dict();
        dict.extend(self.rpc.dict());

        if let Some(root) = self.root.as_ref() {
            dict.insert("root".to_string(), Value::serialize(root)?);
        }
        if let Some(optimizer_runs) = self.num_of_optimizations {
            dict.insert("optimizer".to_string(), Value::serialize(true)?);
            dict.insert("optimizer_runs".to_string(), Value::serialize(optimizer_runs)?);
        }
        if let Some(evm_version) = self.evm_version {
            dict.insert("evm_version".to_string(), Value::serialize(evm_version)?);
        }
        if self.via_ir {
            dict.insert("via_ir".to_string(), Value::serialize(self.via_ir)?);
        }

        if self.no_auto_detect {
            dict.insert("auto_detect_solc".to_string(), Value::serialize(false)?);
        }

        if let Some(ref solc) = self.use_solc {
            let solc = solc.trim_start_matches("solc:");
            dict.insert("solc".to_string(), Value::serialize(solc)?);
        }

        if let Some(api_key) = &self.verifier.verifier_api_key {
            dict.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

struct ProviderRun {
    label: VerificationProviderType,
    args: VerifyArgs,
    provider: Box<dyn VerificationProvider>,
    /// If true, a failed run fails the command; otherwise the failure is logged as a warning.
    required: bool,
}

impl VerifyArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(mut self) -> Result<()> {
        let config = self.load_config()?;

        if self.guess_constructor_args && config.get_rpc_url().is_none() {
            eyre::bail!(
                "You have to provide a valid RPC URL to use --guess-constructor-args feature"
            )
        }

        // If chain is not set, we try to get it from the RPC.
        // If RPC is not set, the default chain is used.
        let chain = match config.get_rpc_url() {
            Some(_) => {
                let provider = utils::get_provider(&config)?;
                utils::get_chain(config.chain, provider).await?
            }
            None => config.chain.unwrap_or_default(),
        };

        let context = self.resolve_context().await?;

        // Set Etherscan options.
        self.etherscan.chain = Some(chain);
        // `get_etherscan_config_with_chain` returns None for chains with no known Etherscan API
        // URL (even when a key was explicitly passed), because `ResolvedEtherscanConfig::create`
        // requires `chain.etherscan_urls()`. Fall back to the raw `etherscan_api_key` from config
        // so that the key survives for warning/fallback logic in `client()`.
        self.etherscan.key = config
            .get_etherscan_config_with_chain(Some(chain))?
            .map(|c| c.key)
            .or_else(|| config.etherscan_api_key.clone());

        // Capture whether the user explicitly provided a verifier URL *before* any auto-injection.
        // This is passed to `client()` so that an auto-injected Sourcify URL does not look like a
        // user-supplied Etherscan-compatible URL and cause the wrong provider to be selected.
        let had_user_verifier_url = self.verifier.verifier_url.is_some();

        // Resolve provider BEFORE URL injection so that the auto-injected Sourcify URL cannot
        // influence routing. For custom-Sourcify chains (e.g. Tempo), etherscan_urls() returns
        // Some but is_custom_sourcify() excludes them from the Etherscan path in resolve().
        let etherscan_key = self.etherscan.key();
        let resolved = self.verifier.resolve(etherscan_key.as_deref(), self.etherscan.chain);

        // For chains with Sourcify-compatible APIs, inject their URL only when we've resolved to
        // Sourcify and the user did not already supply a --verifier-url.
        if resolved.is_sourcify()
            && !had_user_verifier_url
            && let Some(url) = sourcify_api_url(chain)
        {
            self.verifier.verifier_url = Some(url);
        }

        if self.show_standard_json_input {
            let args = EtherscanVerificationProvider::default()
                .create_verify_request(&self, &context)
                .await?;
            sh_println!("{}", args.source)?;
            return Ok(());
        }

        let verifier_url = self.verifier.verifier_url.clone();
        sh_status!("Start verifying contract `{}` deployed on {chain}", self.address)?;
        if let Some(version) = &self.evm_version {
            sh_status!("EVM version: {version}")?;
        }
        if let Some(version) = &self.compiler_version {
            sh_status!("Compiler version: {version}")?;
        }
        if let Some(optimizations) = &self.num_of_optimizations {
            sh_status!("Optimizations:    {optimizations}")?
        }
        if let Some(args) = &self.constructor_args
            && !args.is_empty()
        {
            sh_status!("Constructor args: {args}")?
        }
        let using_etherscan = resolved.is_etherscan();
        let runs =
            self.collect_runs(chain, etherscan_key.as_deref(), resolved, had_user_verifier_url)?;

        // Submit every provider before polling any of them
        let mut required_err = None;
        let mut pending_check = None;
        for ProviderRun { label, args, mut provider, required } in runs {
            sh_status!("\nVerifying on {label}...")?;
            let watch = args.watch;
            match provider.submit(args, context.clone()).await {
                Ok(check_args) => {
                    if required
                        && watch
                        && let Some(check_args) = check_args
                    {
                        pending_check = Some((label, provider, check_args));
                    }
                }
                Err(err) => {
                    if required {
                        required_err = Some(wrap_verifier_url_error(
                            err,
                            verifier_url.as_deref(),
                            using_etherscan,
                        ));
                    } else {
                        sh_warn!("{label} verification failed: {err}")?;
                    }
                }
            }
        }

        // Poll the primary submission for completion
        if required_err.is_none()
            && let Some((label, provider, check_args)) = pending_check
        {
            sh_status!("\nWaiting for {label} verification result...")?;
            if let Err(err) = provider.check(check_args).await {
                required_err =
                    Some(wrap_verifier_url_error(err, verifier_url.as_deref(), using_etherscan));
            }
        }

        required_err.map_or(Ok(()), Err)
    }

    /// Plans the set of verification submissions to run for this invocation.
    ///
    /// `resolved` is the decision from [`VerifierArgs::resolve`] for the primary verifier.
    /// Sourcify is added as an auxiliary run whenever the primary is not Sourcify.
    fn collect_runs(
        &self,
        chain: Chain,
        etherscan_key: Option<&str>,
        resolved: VerificationProviderType,
        had_user_verifier_url: bool,
    ) -> Result<Vec<ProviderRun>> {
        let mut runs = Vec::new();

        let primary_provider = resolved.client(
            etherscan_key,
            self.etherscan.chain,
            had_user_verifier_url,
            self.verifier.is_explicitly_set(),
        )?;
        let primary_is_sourcify = resolved.is_sourcify();
        runs.push(ProviderRun {
            label: resolved,
            args: self.clone(),
            provider: primary_provider,
            required: true,
        });

        // Skip the auxiliary Sourcify submission when the user is clearly using a
        // non-public setup: an explicit `--verifier custom` or a local/dev chain.
        let is_private_setup = resolved.is_custom() || is_dev_chain(chain);
        if !primary_is_sourcify && !is_private_setup {
            let mut args = self.clone();
            args.verifier.verifier = Some(VerificationProviderType::Sourcify);
            args.verifier.verifier_api_key = None;
            // For chains with Sourcify-compatible APIs, use the chain's URL from etherscan_urls
            // Otherwise, drop the URL so Sourcify falls back to its default.
            args.verifier.verifier_url = sourcify_api_url(chain);
            args.watch = false;
            runs.push(ProviderRun {
                label: VerificationProviderType::Sourcify,
                args,
                provider: Box::<SourcifyVerificationProvider>::default(),
                required: false,
            });
        }

        Ok(runs)
    }

    /// Returns the configured verification provider
    pub fn verification_provider(&self) -> Result<Box<dyn VerificationProvider>> {
        self.verifier.effective_type().client(
            self.etherscan.key().as_deref(),
            self.etherscan.chain,
            self.verifier.verifier_url.is_some(),
            self.verifier.is_explicitly_set(),
        )
    }

    /// Resolves [VerificationContext] object either from entered contract name or by trying to
    /// match bytecode located at given address.
    pub async fn resolve_context(&self) -> Result<VerificationContext> {
        let mut config = self.load_config()?;
        config.libraries.extend(self.libraries.clone());

        let project = config.project()?;

        if let Some(ref contract) = self.contract {
            let contract_path = if let Some(ref path) = contract.path {
                project.root().join(PathBuf::from(path))
            } else {
                project.find_contract_path(&contract.name)?
            };

            let cache = project.read_cache_file().ok();

            let mut version = if let Some(ref version) = self.compiler_version {
                version.trim_start_matches('v').parse()?
            } else if let Some(ref solc) = config.solc {
                match solc {
                    SolcReq::Version(version) => version.to_owned(),
                    SolcReq::Local(solc) => Solc::new(solc)?.version,
                }
            } else if let Some(entry) =
                cache.as_ref().and_then(|cache| cache.files.get(&contract_path).cloned())
            {
                let unique_versions = entry
                    .artifacts
                    .get(&contract.name)
                    .map(|artifacts| artifacts.keys().collect::<HashSet<_>>())
                    .unwrap_or_default();

                if unique_versions.is_empty() {
                    eyre::bail!(
                        "No matching artifact found for {}. This could be due to:\n\
                        - Compiler version mismatch - the contract was compiled with a different Solidity version than what's being used for verification",
                        contract.name
                    );
                } else if unique_versions.len() > 1 {
                    warn!(
                        "Ambiguous compiler versions found in cache: {}",
                        unique_versions.iter().join(", ")
                    );
                    eyre::bail!(
                        "Compiler version has to be set in `foundry.toml`. If the project was not deployed with foundry, specify the version through `--compiler-version` flag."
                    )
                }

                unique_versions.into_iter().next().unwrap().to_owned()
            } else {
                eyre::bail!(
                    "If cache is disabled, compiler version must be either provided with `--compiler-version` option or set in foundry.toml"
                )
            };

            let settings = if let Some(profile) = &self.compilation_profile {
                if profile == "default" {
                    &project.settings
                } else if let Some(settings) = project.additional_settings.get(profile.as_str()) {
                    settings
                } else {
                    eyre::bail!("Unknown compilation profile: {}", profile)
                }
            } else if let Some((cache, entry)) = cache
                .as_ref()
                .and_then(|cache| Some((cache, cache.files.get(&contract_path)?.clone())))
            {
                let profiles = entry
                    .artifacts
                    .get(&contract.name)
                    .and_then(|artifacts| {
                        let mut cached_artifacts = artifacts.get(&version);
                        // If we try to verify with specific build version and no cached artifacts
                        // found, then check if we have artifacts cached for same version but
                        // without any build metadata.
                        // This could happen when artifacts are built / cached
                        // with a version like `0.8.20` but verify is using a compiler-version arg
                        // as `0.8.20+commit.a1b79de6`.
                        // See <https://github.com/foundry-rs/foundry/issues/9510>.
                        if cached_artifacts.is_none() && version.build != BuildMetadata::EMPTY {
                            version.build = BuildMetadata::EMPTY;
                            cached_artifacts = artifacts.get(&version);
                        }
                        cached_artifacts
                    })
                    .map(|artifacts| artifacts.keys().collect::<HashSet<_>>())
                    .unwrap_or_default();

                if profiles.is_empty() {
                    eyre::bail!(
                        "No matching artifact found for {} with compiler version {}. This could be due to:\n\
                        - Compiler version mismatch - the contract was compiled with a different Solidity version",
                        contract.name,
                        version
                    );
                } else if profiles.len() > 1 {
                    eyre::bail!(
                        "Ambiguous compilation profiles found in cache: {}, please specify the profile through `--compilation-profile` flag",
                        profiles.iter().join(", ")
                    )
                }

                let profile = profiles.into_iter().next().unwrap().to_owned();
                cache.profiles.get(&profile).expect("must be present")
            } else if project.additional_settings.is_empty() {
                &project.settings
            } else {
                eyre::bail!(
                    "If cache is disabled, compilation profile must be provided with `--compilation-profile` option or set in foundry.toml"
                )
            };

            VerificationContext::new(
                contract_path,
                contract.name.clone(),
                version,
                config,
                settings.clone(),
            )
        } else {
            if config.get_rpc_url().is_none() {
                eyre::bail!("You have to provide a contract name or a valid RPC URL")
            }
            let provider = utils::get_provider(&config)?;
            let code = provider.get_code_at(self.address).await?;

            let output = ProjectCompiler::new().quiet(true).compile(&project)?;
            let contracts = ContractsByArtifact::new(
                output.artifact_ids().map(|(id, artifact)| (id, artifact.clone().into())),
            );

            let Some((artifact_id, _)) = contracts.find_by_deployed_code_exact(&code) else {
                eyre::bail!(format!(
                    "Bytecode at {} does not match any local contracts",
                    self.address
                ))
            };

            let settings = project
                .settings_profiles()
                .find_map(|(name, settings)| {
                    (name == artifact_id.profile.as_str()).then_some(settings)
                })
                .expect("must be present");

            VerificationContext::new(
                artifact_id.source.clone(),
                artifact_id.name.split('.').next().unwrap().to_owned(),
                artifact_id.version.clone(),
                config,
                settings.clone(),
            )
        }
    }

    /// Detects the language for verification from source file extension, if none provided.
    pub fn detect_language(&self, ctx: &VerificationContext) -> ContractLanguage {
        self.language.unwrap_or_else(|| {
            match ctx.target_path.extension().and_then(|e| e.to_str()) {
                Some("vy") => ContractLanguage::Vyper,
                _ => ContractLanguage::Solidity,
            }
        })
    }
}

/// Check verification status arguments
#[derive(Clone, Debug, Parser)]
pub struct VerifyCheckArgs {
    /// The verification ID.
    ///
    /// For Etherscan - Submission GUID.
    ///
    /// For Sourcify - Verification Job ID.
    pub id: String,

    #[command(flatten)]
    pub retry: RetryArgs,

    #[command(flatten)]
    pub etherscan: EtherscanOpts,

    #[command(flatten)]
    pub verifier: VerifierArgs,
}

impl_figment_convert_cast!(VerifyCheckArgs);

impl VerifyCheckArgs {
    /// Run the verify command to submit the contract's source code for verification on etherscan
    pub async fn run(self) -> Result<()> {
        sh_status!("Checking verification status on {}", self.etherscan.chain.unwrap_or_default())?;
        self.verifier
            .effective_type()
            .client(
                self.etherscan.key().as_deref(),
                self.etherscan.chain,
                self.verifier.verifier_url.is_some(),
                self.verifier.is_explicitly_set(),
            )?
            .check(self)
            .await
    }
}

impl FigmentProvider for VerifyCheckArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Verify Check Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = self.etherscan.dict();
        if let Some(api_key) = &self.etherscan.key {
            dict.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Returns the Sourcify-compatible API URL for chains that have one registered in `etherscan_urls`.
///
/// Some chains register their Sourcify-compatible verification API under `etherscan_urls` in
/// alloy-chains. This function returns the properly formatted URL for such chains.
fn sourcify_api_url(chain: Chain) -> Option<String> {
    if chain.is_custom_sourcify() {
        chain.etherscan_urls().map(|(api_url, _)| {
            let api_url = api_url.trim_end_matches('/');
            format!("{api_url}/")
        })
    } else {
        None
    }
}

/// Returns `true` for local/dev chains.
const fn is_dev_chain(chain: Chain) -> bool {
    use foundry_config::NamedChain;
    matches!(chain.named(), Some(NamedChain::Dev | NamedChain::AnvilHardhat | NamedChain::Cannon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_verify_contract() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Domains.sol:Domains",
            "--via-ir",
            "--license-type",
            "13",
        ]);
        assert!(args.via_ir);
        assert_eq!(args.license_type.as_deref(), Some("13"));
    }

    #[test]
    fn can_parse_new_compiler_flags() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Domains.sol:Domains",
            "--no-auto-detect",
            "--use",
            "0.8.23",
        ]);
        assert!(args.no_auto_detect);
        assert_eq!(args.use_solc.as_deref(), Some("0.8.23"));
    }

    #[test]
    fn classify_verifier_probe_accepts_not_verified_response() {
        let body =
            r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#;
        assert_eq!(
            classify_verifier_credential_response(StatusCode::OK, body),
            VerifierCredentialProbe::Accepted,
        );
    }

    #[test]
    fn classify_verifier_probe_rejects_invalid_api_key() {
        let body = r#"{"status":"0","message":"NOTOK","result":"Invalid API Key"}"#;
        assert_eq!(
            classify_verifier_credential_response(StatusCode::OK, body),
            VerifierCredentialProbe::InvalidApiKey,
        );
        assert_eq!(
            classify_verifier_credential_response(StatusCode::UNAUTHORIZED, ""),
            VerifierCredentialProbe::InvalidApiKey,
        );
    }

    #[test]
    fn classify_verifier_probe_treats_transient_errors_as_inconclusive() {
        let body = r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#;
        assert_eq!(
            classify_verifier_credential_response(StatusCode::OK, body),
            VerifierCredentialProbe::Inconclusive,
        );
        assert_eq!(
            classify_verifier_credential_response(
                StatusCode::OK,
                "Checking if the site connection is secure",
            ),
            VerifierCredentialProbe::Inconclusive,
        );
        assert_eq!(
            classify_verifier_credential_response(
                StatusCode::FORBIDDEN,
                "Sorry, you have been blocked",
            ),
            VerifierCredentialProbe::Inconclusive,
        );
        assert_eq!(
            classify_verifier_credential_response(StatusCode::FORBIDDEN, ""),
            VerifierCredentialProbe::Inconclusive,
        );
    }

    #[test]
    fn parse_http_verifier_url_rejects_unsupported_schemes() {
        assert!(parse_http_verifier_url("https://example.com/api", "verifier").is_ok());
        assert!(parse_http_verifier_url("http://example.com/api", "verifier").is_ok());

        let err = parse_http_verifier_url("gopher://example.com/api", "verifier").unwrap_err();
        assert!(
            err.to_string().contains("URL scheme must be http or https"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn resolve_explicit_sourcify_overrides_api_key() {
        let args = VerifierArgs {
            verifier: Some(VerificationProviderType::Sourcify),
            verifier_api_key: None,
            verifier_url: None,
        };
        assert_eq!(
            args.resolve(Some("mykey"), Some(Chain::mainnet())),
            VerificationProviderType::Sourcify,
        );
    }

    #[test]
    fn resolve_explicit_etherscan_is_etherscan() {
        let args = VerifierArgs {
            verifier: Some(VerificationProviderType::Etherscan),
            verifier_api_key: None,
            verifier_url: None,
        };
        assert_eq!(
            args.resolve(Some("mykey"), Some(Chain::mainnet())),
            VerificationProviderType::Etherscan,
        );
    }

    #[test]
    fn resolve_implicit_with_key_and_known_chain_uses_etherscan() {
        let args = VerifierArgs { verifier: None, verifier_api_key: None, verifier_url: None };
        assert_eq!(
            args.resolve(Some("mykey"), Some(Chain::mainnet())),
            VerificationProviderType::Etherscan,
        );
    }

    #[test]
    fn resolve_implicit_with_key_and_unknown_chain_falls_back_to_sourcify() {
        let args = VerifierArgs { verifier: None, verifier_api_key: None, verifier_url: None };
        assert_eq!(
            args.resolve(Some("mykey"), Some(Chain::from(3658348u64))),
            VerificationProviderType::Sourcify,
        );
    }

    #[test]
    fn resolve_implicit_with_key_and_unknown_chain_but_url_uses_etherscan() {
        let args = VerifierArgs {
            verifier: None,
            verifier_api_key: None,
            verifier_url: Some("https://example.com/api".to_string()),
        };
        assert_eq!(
            args.resolve(Some("mykey"), Some(Chain::from(3658348u64))),
            VerificationProviderType::Etherscan,
        );
    }

    #[test]
    fn resolve_implicit_no_key_falls_back_to_sourcify() {
        let args = VerifierArgs { verifier: None, verifier_api_key: None, verifier_url: None };
        assert_eq!(args.resolve(None, Some(Chain::mainnet())), VerificationProviderType::Sourcify,);
    }

    // Regression: custom-Sourcify chains (e.g. Tempo) register Sourcify-compatible URLs under
    // etherscan_urls(). An implicit ETHERSCAN_API_KEY must NOT route them to Etherscan.
    #[test]
    fn resolve_implicit_with_key_and_custom_sourcify_chain_falls_back_to_sourcify() {
        let tempo = Chain::from(4217u64); // NamedChain::Tempo
        assert!(tempo.is_custom_sourcify(), "sanity: Tempo should be is_custom_sourcify");
        let args = VerifierArgs { verifier: None, verifier_api_key: None, verifier_url: None };
        assert_eq!(args.resolve(Some("mykey"), Some(tempo)), VerificationProviderType::Sourcify,);
    }

    // Ensure the is_custom_sourcify() guard holds even when a URL is present (e.g. the URL was
    // auto-injected by run()). A user-supplied --verifier-url on a custom-Sourcify chain with a
    // key should still resolve to Sourcify, not Etherscan.
    #[test]
    fn resolve_custom_sourcify_chain_with_url_and_key_stays_sourcify() {
        let tempo = Chain::from(4217u64);
        let args = VerifierArgs {
            verifier: None,
            verifier_api_key: None,
            verifier_url: Some("https://contracts.tempo.xyz/".to_string()),
        };
        assert_eq!(args.resolve(Some("mykey"), Some(tempo)), VerificationProviderType::Sourcify,);
    }

    #[test]
    fn collect_runs_adds_sourcify_provider() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--etherscan-api-key",
            "k",
        ]);
        let resolved = args.verifier.resolve(Some("k"), Some(Chain::mainnet()));
        let runs = args.collect_runs(Chain::mainnet(), Some("k"), resolved, false).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].label, VerificationProviderType::Etherscan);
        assert!(runs[0].required);
        assert_eq!(runs[1].label, VerificationProviderType::Sourcify);
        assert!(!runs[1].required);
        assert!(runs[1].args.verifier.verifier_api_key.is_none());
    }

    #[test]
    fn collect_runs_skips_secondary_when_primary_is_sourcify() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--verifier",
            "sourcify",
        ]);
        let resolved = args.verifier.resolve(None, Some(Chain::mainnet()));
        let runs = args.collect_runs(Chain::mainnet(), None, resolved, false).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label, VerificationProviderType::Sourcify);
        assert!(runs[0].required);
    }

    #[test]
    fn collect_runs_skips_secondary_for_custom_verifier() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--verifier",
            "custom",
            "--verifier-url",
            "https://internal.example.com/api",
        ]);
        let runs = args
            .collect_runs(Chain::mainnet(), None, VerificationProviderType::Custom, true)
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].label, VerificationProviderType::Custom);
    }

    #[test]
    fn collect_runs_skips_secondary_on_dev_chain() {
        let args: VerifyArgs = VerifyArgs::parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000000",
            "src/Counter.sol:Counter",
            "--etherscan-api-key",
            "k",
        ]);
        let anvil = Chain::from(31337u64);
        let resolved = args.verifier.resolve(Some("k"), Some(anvil));
        let runs = args.collect_runs(anvil, Some("k"), resolved, false).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].required);
    }
}
