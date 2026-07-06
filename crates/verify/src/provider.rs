use crate::{
    etherscan::EtherscanVerificationProvider,
    sourcify::SourcifyVerificationProvider,
    verify::{VerifyArgs, VerifyCheckArgs},
};
use alloy_json_abi::JsonAbi;
use async_trait::async_trait;
use eyre::{Context, Result};
use foundry_common::compile::compile_target_abi;
use foundry_compilers::{
    Project,
    artifacts::{Source, StandardJsonCompilerInput, vyper::VyperInput},
    compilers::solc::SolcCompiler,
    multi::MultiCompilerSettings,
    solc::{Solc, SolcLanguage},
};
use foundry_config::{Chain, Config, EtherscanConfigError};
use semver::Version;
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Container with data required for contract verification.
#[derive(Debug, Clone)]
pub struct VerificationContext {
    pub config: Config,
    pub project: Project,
    pub target_path: PathBuf,
    pub target_name: String,
    pub compiler_version: Version,
    pub compiler_settings: MultiCompilerSettings,
}

impl VerificationContext {
    pub fn new(
        target_path: PathBuf,
        target_name: String,
        compiler_version: Version,
        config: Config,
        compiler_settings: MultiCompilerSettings,
    ) -> Result<Self> {
        let mut project = config.project()?;
        project.no_artifacts = true;

        let solc = Solc::find_or_install(&compiler_version)?;
        project.compiler.solc = Some(SolcCompiler::Specific(solc));

        Ok(Self { config, project, target_name, target_path, compiler_version, compiler_settings })
    }

    pub fn get_solc_standard_json_input(&self) -> Result<StandardJsonCompilerInput> {
        let mut input: StandardJsonCompilerInput = self
            .project
            .standard_json_input(&self.target_path)
            .wrap_err("Failed to get standard json input")?
            .normalize_evm_version(&self.compiler_version);

        let mut settings = self.compiler_settings.solc.settings.clone();
        settings.libraries.libs = input
            .settings
            .libraries
            .libs
            .into_iter()
            .map(|(f, libs)| {
                (f.strip_prefix(self.project.root()).unwrap_or(&f).to_path_buf(), libs)
            })
            .collect();

        settings.remappings = input.settings.remappings;
        settings.sanitize(&self.compiler_version, SolcLanguage::Solidity);
        input.settings = settings;

        Ok(input)
    }

    /// Creates Vyper standard JSON input for verification.
    pub fn get_vyper_standard_json_input(&self) -> Result<VyperInput> {
        let path = Path::new(&self.target_path);
        let sources = Source::read_all_from(path, &["vy", "vyi"])?;
        Ok(VyperInput::new(sources, self.compiler_settings.vyper.clone(), &self.compiler_version))
    }

    /// Compiles target contract requesting only ABI and returns it.
    pub fn get_target_abi(&self) -> Result<JsonAbi> {
        compile_target_abi(&self.project, &self.target_path, &self.target_name)
    }
}

/// An abstraction for various verification providers such as etherscan, sourcify, blockscout
#[async_trait]
pub trait VerificationProvider {
    /// Returns the provider type, used to assert the selected provider in tests.
    fn provider_type(&self) -> VerificationProviderType;

    /// This should ensure the verify request can be prepared successfully.
    ///
    /// Caution: Implementers must ensure that this _never_ sends the actual verify request
    /// `[VerificationProvider::verify]`, instead this is supposed to evaluate whether the given
    /// [`VerifyArgs`] are valid to begin with. This should prevent situations where there's a
    /// contract deployment that's executed before the verify request and the subsequent verify task
    /// fails due to misconfiguration.
    async fn preflight_verify_check(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<()>;

    /// Submits the verification request for the targeted contract.
    ///
    /// Returns `Some(check_args)` if a follow-up status check is possible (the request was
    /// accepted by the provider), or `None` if the submission was a no-op (e.g. the contract
    /// was already verified).
    async fn submit(
        &mut self,
        args: VerifyArgs,
        context: VerificationContext,
    ) -> Result<Option<VerifyCheckArgs>>;

    /// Convenience wrapper: [`Self::submit`]s and, if `args.watch` is set, polls
    /// [`Self::check`] until completion.
    async fn verify(&mut self, args: VerifyArgs, context: VerificationContext) -> Result<()> {
        let watch = args.watch;
        let check_args = self.submit(args, context).await?;
        if watch && let Some(check_args) = check_args {
            return self.check(check_args).await;
        }
        Ok(())
    }

    /// Checks whether the contract is verified.
    async fn check(&self, args: VerifyCheckArgs) -> Result<()>;
}

impl FromStr for VerificationProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "e" | "etherscan" => Ok(Self::Etherscan),
            "s" | "sourcify" => Ok(Self::Sourcify),
            "b" | "blockscout" => Ok(Self::Blockscout),
            "o" | "oklink" => Ok(Self::Oklink),
            "c" | "custom" => Ok(Self::Custom),
            _ => Err(format!("Unknown provider: {s}")),
        }
    }
}

impl fmt::Display for VerificationProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Etherscan => {
                write!(f, "etherscan")?;
            }
            Self::Sourcify => {
                write!(f, "sourcify")?;
            }
            Self::Blockscout => {
                write!(f, "blockscout")?;
            }
            Self::Oklink => {
                write!(f, "oklink")?;
            }
            Self::Custom => {
                write!(f, "custom")?;
            }
        };
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum VerificationProviderType {
    Etherscan,
    #[default]
    Sourcify,
    Blockscout,
    Oklink,
    /// Custom verification provider, requires compatibility with the Etherscan API.
    Custom,
}

impl VerificationProviderType {
    /// Returns the corresponding `VerificationProvider` for the key.
    ///
    /// `is_explicit` should be `true` when the user explicitly passed `--verifier`; `false` when
    /// the value is the default (Sourcify). An explicit flag always takes precedence over the
    /// `ETHERSCAN_API_KEY` environment variable.
    pub fn client(
        &self,
        key: Option<&str>,
        chain: Option<Chain>,
        has_url: bool,
        is_explicit: bool,
    ) -> Result<Box<dyn VerificationProvider>> {
        let has_key = key.is_some_and(|k| !k.is_empty());

        // 1. Explicit `--verifier sourcify` always wins over ETHERSCAN_API_KEY.
        if is_explicit && self.is_sourcify() {
            return Ok(Box::<SourcifyVerificationProvider>::default());
        }

        // 2. `--verifier etherscan` (explicit): check chain support and require key.
        if self.is_etherscan() {
            if let Some(chain) = chain
                && (chain.etherscan_urls().is_none() || chain.is_custom_sourcify())
                && !has_url
            {
                eyre::bail!(EtherscanConfigError::UnknownChain(
                    "when using Etherscan verifier".to_string(),
                    chain
                ))
            }
            if !has_key {
                eyre::bail!("ETHERSCAN_API_KEY must be set to use Etherscan as a verifier")
            }
            return Ok(Box::<EtherscanVerificationProvider>::default());
        }

        // 3. Explicit `--verifier blockscout | oklink | custom`: require a URL.
        if is_explicit && matches!(self, Self::Blockscout | Self::Oklink | Self::Custom) {
            if !has_url {
                eyre::bail!("No verifier URL specified for verifier {}", self);
            }
            return Ok(Box::<EtherscanVerificationProvider>::default());
        }

        // 4. No explicit `--verifier` but ETHERSCAN_API_KEY is set: prefer Etherscan when the chain
        //    is supported; otherwise warn and fall through to the Sourcify default. See <https://github.com/foundry-rs/foundry/issues/10774>.
        //    Custom-Sourcify chains (e.g. Tempo) register Sourcify-compatible URLs under
        //    etherscan_urls() and are hard-excluded here regardless of whether a URL is present.
        if has_key {
            if let Some(chain) = chain
                && (chain.is_custom_sourcify() || chain.etherscan_urls().is_none() && !has_url)
            {
                if chain.is_custom_sourcify() {
                    sh_warn!(
                        "ETHERSCAN_API_KEY is set but chain {chain} uses a Sourcify-compatible \
                         API. Falling back to Sourcify. Pass `--verifier sourcify` to suppress \
                         this warning."
                    )?;
                } else {
                    sh_warn!(
                        "ETHERSCAN_API_KEY is set but chain {chain} has no known Etherscan API \
                         URL. Falling back to Sourcify. Pass --verifier-url <URL> or \
                         `--verifier <provider>` to override."
                    )?;
                }
                // Fall through to branch 5 (Sourcify default) below.
            } else {
                return Ok(Box::<EtherscanVerificationProvider>::default());
            }
        }

        // 5. No key, no explicit verifier: default to Sourcify.
        if self.is_sourcify() {
            return Ok(Box::<SourcifyVerificationProvider>::default());
        }

        // 6. No valid provider.
        eyre::bail!(
            "No valid verification provider specified. Pass the --verifier flag to specify a provider or set the ETHERSCAN_API_KEY environment variable to use Etherscan as a verifier."
        )
    }

    pub const fn is_sourcify(&self) -> bool {
        matches!(self, Self::Sourcify)
    }

    pub const fn is_etherscan(&self) -> bool {
        matches!(self, Self::Etherscan)
    }

    pub const fn is_custom(&self) -> bool {
        matches!(self, Self::Custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etherscan_allows_unknown_chain_with_verifier_url() {
        let chain = Chain::from(3658348u64);
        let provider = VerificationProviderType::Etherscan
            .client(Some("key"), Some(chain), true, true)
            .unwrap();
        assert_eq!(provider.provider_type(), VerificationProviderType::Etherscan);
    }

    #[test]
    fn etherscan_rejects_unknown_chain_without_verifier_url() {
        let chain = Chain::from(3658348u64);
        let res = VerificationProviderType::Etherscan.client(Some("key"), Some(chain), false, true);
        match res {
            Ok(_) => panic!("expected unknown-chain error"),
            Err(err) => {
                assert!(err.to_string().contains("No known Etherscan API URL"));
            }
        }
    }

    // Regression: explicit --verifier etherscan on a custom-Sourcify chain (e.g. Tempo) without
    // --verifier-url must be rejected even though etherscan_urls() returns Some for the chain.
    #[test]
    fn explicit_etherscan_on_custom_sourcify_chain_without_url_bails() {
        let tempo = Chain::from(4217u64); // NamedChain::Tempo
        let res = VerificationProviderType::Etherscan.client(Some("key"), Some(tempo), false, true);
        assert!(res.is_err(), "expected error for Etherscan on custom-Sourcify chain w/o URL");
    }

    // Custom-Sourcify chain with an explicit --verifier-url is allowed for Etherscan.
    #[test]
    fn explicit_etherscan_on_custom_sourcify_chain_with_url_is_ok() {
        let tempo = Chain::from(4217u64);
        let provider = VerificationProviderType::Etherscan
            .client(Some("key"), Some(tempo), true, true)
            .unwrap();
        assert_eq!(provider.provider_type(), VerificationProviderType::Etherscan);
    }

    // Implicit ETHERSCAN_API_KEY on a supported chain selects Etherscan; on a custom-Sourcify
    // chain it must fall back to Sourcify regardless of whether a --verifier-url is present.
    #[test]
    fn implicit_etherscan_custom_sourcify_chain_falls_back_to_sourcify() {
        // Baseline: implicit key on a normal chain -> Etherscan.
        let provider = VerificationProviderType::Sourcify
            .client(Some("mykey"), Some(Chain::mainnet()), false, false)
            .unwrap();
        assert_eq!(provider.provider_type(), VerificationProviderType::Etherscan);

        // Custom-Sourcify chain without URL -> Sourcify.
        let provider = VerificationProviderType::Sourcify
            .client(Some("mykey"), Some(Chain::from(4217u64)), false, false)
            .expect("expected fallback to Sourcify, got error");
        assert_eq!(provider.provider_type(), VerificationProviderType::Sourcify);

        // Custom-Sourcify chain with URL -> still Sourcify (URL does not override the exclusion).
        let provider = VerificationProviderType::Sourcify
            .client(Some("mykey"), Some(Chain::from(4217u64)), true, false)
            .expect("expected fallback to Sourcify, got error");
        assert_eq!(provider.provider_type(), VerificationProviderType::Sourcify);
    }

    // Regression test for <https://github.com/foundry-rs/foundry/issues/10774>:
    // when --verifier is not set, ETHERSCAN_API_KEY is set, but the chain has no known
    // Etherscan API URL, `client()` must NOT bail; it should warn and fall back to Sourcify.
    // (Behavior is verified more strictly via `VerifierArgs::resolve` tests in `verify.rs`.)
    #[test]
    fn implicit_etherscan_unknown_chain_falls_back_to_sourcify() {
        // Baseline: implicit key on a normal chain -> Etherscan.
        let provider = VerificationProviderType::Sourcify
            .client(Some("mykey"), Some(Chain::mainnet()), false, false)
            .unwrap();
        assert_eq!(provider.provider_type(), VerificationProviderType::Etherscan);

        // Unknown chain: same call must fall back to Sourcify, not bail.
        let provider = VerificationProviderType::Sourcify
            .client(Some("mykey"), Some(Chain::from(3658348u64)), false, false)
            .expect("expected fallback to Sourcify, got error");
        assert_eq!(provider.provider_type(), VerificationProviderType::Sourcify);
    }
}
