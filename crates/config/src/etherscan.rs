//! Support for multiple Etherscan keys.

use crate::{
    Chain, Config, NamedChain,
    resolve::{RE_PLACEHOLDER, UnresolvedEnvVarError, interpolate},
};
use figment::{
    Error, Metadata, Profile, Provider,
    providers::Env,
    value::{Dict, Map},
};
use foundry_block_explorers::EtherscanApiVersion;
use heck::ToKebabCase;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    fmt,
    ops::{Deref, DerefMut},
    time::Duration,
};

/// The user agent to use when querying the etherscan API.
pub const ETHERSCAN_USER_AGENT: &str = concat!("foundry/", env!("CARGO_PKG_VERSION"));

/// A [Provider] that provides Etherscan API key from the environment if it's not empty.
///
/// This prevents `ETHERSCAN_API_KEY=""` if it's set but empty
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub(crate) struct EtherscanEnvProvider;

impl Provider for EtherscanEnvProvider {
    fn metadata(&self) -> Metadata {
        Env::raw().metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = Dict::default();
        let env_provider = Env::raw().only(&["ETHERSCAN_API_KEY"]);
        if let Some((key, value)) = env_provider.iter().next()
            && !value.trim().is_empty()
        {
            dict.insert(key.as_str().to_string(), value.into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Errors that can occur when creating an `EtherscanConfig`
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EtherscanConfigError {
    #[error(transparent)]
    Unresolved(#[from] UnresolvedEnvVarError),

    #[error(
        "No known Etherscan API URL for chain `{1}`. To fix this, please:\n\
        1. Specify a `url` {0}\n\
        2. Verify the chain `{1}` is correct"
    )]
    UnknownChain(String, Chain),

    #[error("At least one of `url` or `chain` must be present{0}")]
    MissingUrlOrChain(String),
}

/// Container type for Etherscan API keys and URLs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EtherscanConfigs {
    configs: BTreeMap<String, EtherscanConfig>,
}

impl EtherscanConfigs {
    /// Creates a new list of etherscan configs
    pub fn new(configs: impl IntoIterator<Item = (impl Into<String>, EtherscanConfig)>) -> Self {
        Self { configs: configs.into_iter().map(|(name, config)| (name.into(), config)).collect() }
    }

    /// Returns `true` if this type doesn't contain any configs
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Returns the first config that matches the chain
    pub fn find_chain(&self, chain: Chain) -> Option<&EtherscanConfig> {
        self.configs.values().find(|config| config.chain == Some(chain))
    }

    /// Returns all (alias -> url) pairs
    pub fn resolved(self, default_api_version: EtherscanApiVersion) -> ResolvedEtherscanConfigs {
        ResolvedEtherscanConfigs {
            configs: self
                .configs
                .into_iter()
                .map(|(name, e)| {
                    let resolved = e.resolve(Some(&name), default_api_version);
                    (name, resolved)
                })
                .collect(),
        }
    }
}

impl Deref for EtherscanConfigs {
    type Target = BTreeMap<String, EtherscanConfig>;

    fn deref(&self) -> &Self::Target {
        &self.configs
    }
}

impl DerefMut for EtherscanConfigs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.configs
    }
}

/// Container type for _resolved_ etherscan keys, see [`EtherscanConfigs::resolved`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedEtherscanConfigs {
    /// contains all named `ResolvedEtherscanConfig` or an error if we failed to resolve the env
    /// var alias
    configs: BTreeMap<String, Result<ResolvedEtherscanConfig, EtherscanConfigError>>,
}

impl ResolvedEtherscanConfigs {
    /// Creates a new list of resolved etherscan configs
    pub fn new(
        configs: impl IntoIterator<Item = (impl Into<String>, ResolvedEtherscanConfig)>,
    ) -> Self {
        Self {
            configs: configs.into_iter().map(|(name, config)| (name.into(), Ok(config))).collect(),
        }
    }

    /// Returns the first config that matches the chain
    pub fn find_chain(
        self,
        chain: Chain,
    ) -> Option<Result<ResolvedEtherscanConfig, EtherscanConfigError>> {
        for (_, config) in self.configs.into_iter() {
            match config {
                Ok(c) if c.chain == Some(chain) => return Some(Ok(c)),
                Err(e) => return Some(Err(e)),
                _ => continue,
            }
        }
        None
    }

    /// Returns true if there's a config that couldn't be resolved
    pub fn has_unresolved(&self) -> bool {
        self.configs.values().any(|val| val.is_err())
    }
}

impl Deref for ResolvedEtherscanConfigs {
    type Target = BTreeMap<String, Result<ResolvedEtherscanConfig, EtherscanConfigError>>;

    fn deref(&self) -> &Self::Target {
        &self.configs
    }
}

impl DerefMut for ResolvedEtherscanConfigs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.configs
    }
}

/// Represents all info required to create an etherscan client
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtherscanConfig {
    /// The chain name or EIP-155 chain ID used to derive the API URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Chain>,
    /// Etherscan API URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Etherscan API Version. Defaults to v2
    #[serde(default, alias = "api-version", skip_serializing_if = "Option::is_none")]
    pub api_version: Option<EtherscanApiVersion>,
    /// The etherscan API KEY that's required to make requests
    pub key: EtherscanApiKey,
}

impl EtherscanConfig {
    /// Returns the etherscan config required to create a client.
    ///
    /// # Errors
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set or
    /// no chain or url is configured
    pub fn resolve(
        self,
        alias: Option<&str>,
        default_api_version: EtherscanApiVersion,
    ) -> Result<ResolvedEtherscanConfig, EtherscanConfigError> {
        let Self { chain, mut url, key, api_version } = self;

        let api_version = api_version.unwrap_or(default_api_version);

        if let Some(url) = &mut url {
            *url = interpolate(url)?;
        }

        let (chain, alias) = match (chain, alias) {
            // fill one with the other
            (Some(chain), None) => (Some(chain), Some(chain.to_string())),
            (None, Some(alias)) => {
                // alloy chain is parsed as kebab case
                (
                    alias.to_kebab_case().parse().ok().or_else(|| {
                        // if this didn't work try to parse as json because the deserialize impl
                        // supports more aliases
                        serde_json::from_str::<NamedChain>(&format!("\"{alias}\""))
                            .map(Into::into)
                            .ok()
                    }),
                    Some(alias.into()),
                )
            }
            // leave as is
            (Some(chain), Some(alias)) => (Some(chain), Some(alias.into())),
            (None, None) => (None, None),
        };
        let key = key.resolve()?;

        match (chain, url) {
            (Some(chain), Some(api_url)) => Ok(ResolvedEtherscanConfig {
                api_url,
                api_version,
                browser_url: chain.etherscan_urls().map(|(_, url)| url.to_string()),
                key,
                chain: Some(chain),
            }),
            (Some(chain), None) => ResolvedEtherscanConfig::create(key, chain, api_version)
                .ok_or_else(|| {
                    let msg = alias.map(|a| format!("for `{a}`")).unwrap_or_default();
                    EtherscanConfigError::UnknownChain(msg, chain)
                }),
            (None, Some(api_url)) => Ok(ResolvedEtherscanConfig {
                api_url,
                browser_url: None,
                key,
                chain: None,
                api_version,
            }),
            (None, None) => {
                let msg = alias
                    .map(|a| format!(" for Etherscan config with unknown alias `{a}`"))
                    .unwrap_or_default();
                Err(EtherscanConfigError::MissingUrlOrChain(msg))
            }
        }
    }
}

/// Contains required url + api key to set up an etherscan client
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEtherscanConfig {
    /// Etherscan API URL.
    #[serde(rename = "url")]
    pub api_url: String,
    /// Optional browser URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_url: Option<String>,
    /// The resolved API key.
    pub key: String,
    /// Etherscan API Version.
    #[serde(default)]
    pub api_version: EtherscanApiVersion,
    /// The chain name or EIP-155 chain ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Chain>,
}

impl ResolvedEtherscanConfig {
    /// Creates a new instance using the api key and chain
    pub fn create(
        api_key: impl Into<String>,
        chain: impl Into<Chain>,
        api_version: EtherscanApiVersion,
    ) -> Option<Self> {
        let chain = chain.into();
        let (api_url, browser_url) = chain.etherscan_urls()?;
        Some(Self {
            api_url: api_url.to_string(),
            api_version,
            browser_url: Some(browser_url.to_string()),
            key: api_key.into(),
            chain: Some(chain),
        })
    }

    /// Sets the chain value and consumes the type
    ///
    /// This is only used to set derive the appropriate Cache path for the etherscan client
    pub fn with_chain(mut self, chain: impl Into<Chain>) -> Self {
        self.set_chain(chain);
        self
    }

    /// Sets the chain value
    pub fn set_chain(&mut self, chain: impl Into<Chain>) -> &mut Self {
        let chain = chain.into();
        if let Some((api, browser)) = chain.etherscan_urls() {
            self.api_url = api.to_string();
            self.browser_url = Some(browser.to_string());
        }
        self.chain = Some(chain);
        self
    }

    /// Returns the corresponding `foundry_block_explorers::Client`, configured with the `api_url`,
    /// `api_key` and cache
    pub fn into_client(
        self,
    ) -> Result<foundry_block_explorers::Client, foundry_block_explorers::errors::EtherscanError>
    {
        let Self { api_url, browser_url, key: api_key, chain, api_version } = self;

        let chain = chain.unwrap_or_default();
        let cache = Config::foundry_etherscan_chain_cache_dir(chain);

        if let Some(cache_path) = &cache {
            // we also create the `sources` sub dir here
            if let Err(err) = std::fs::create_dir_all(cache_path.join("sources")) {
                warn!("could not create etherscan cache dir: {:?}", err);
            }
        }

        let api_url = into_url(&api_url)?;
        let client = reqwest::Client::builder()
            .user_agent(ETHERSCAN_USER_AGENT)
            .tls_built_in_root_certs(api_url.scheme() == "https")
            .build()?;
        let mut client_builder = foundry_block_explorers::Client::builder()
            .with_client(client)
            .with_api_version(api_version)
            .with_api_key(api_key)
            .with_cache(cache, Duration::from_secs(24 * 60 * 60));
        if let Some(browser_url) = browser_url {
            client_builder = client_builder.with_url(browser_url)?;
        }
        client_builder.chain(chain)?.build()
    }
}

/// Represents a single etherscan API key
///
/// This type preserves the value as it's stored in the config. If the value is a reference to an
/// env var, then the `EtherscanKey::Key` var will hold the reference (`${MAIN_NET}`) and _not_ the
/// value of the env var itself.
/// In other words, this type does not resolve env vars when it's being deserialized
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EtherscanApiKey {
    /// A raw key
    Key(String),
    /// An endpoint that contains at least one `${ENV_VAR}` placeholder
    ///
    /// **Note:** this contains the key or `${ETHERSCAN_KEY}`
    Env(String),
}

impl EtherscanApiKey {
    /// Returns the key variant
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Self::Key(url) => Some(url),
            Self::Env(_) => None,
        }
    }

    /// Returns the env variant
    pub fn as_env(&self) -> Option<&str> {
        match self {
            Self::Env(val) => Some(val),
            Self::Key(_) => None,
        }
    }

    /// Returns the key this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            Self::Key(key) => Ok(key),
            Self::Env(val) => interpolate(&val),
        }
    }
}

impl Serialize for EtherscanApiKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EtherscanApiKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = String::deserialize(deserializer)?;
        let endpoint = if RE_PLACEHOLDER.is_match(&val) { Self::Env(val) } else { Self::Key(val) };

        Ok(endpoint)
    }
}

impl fmt::Display for EtherscanApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => key.fmt(f),
            Self::Env(var) => var.fmt(f),
        }
    }
}

/// This is a hack to work around `IntoUrl`'s sealed private functions, which can't be called
/// normally.
#[inline]
fn into_url(url: impl reqwest::IntoUrl) -> std::result::Result<reqwest::Url, reqwest::Error> {
    url.into_url()
}

#[cfg(test)]
mod tests {
    use super::*;
    use NamedChain::Mainnet;

    #[test]
    fn can_create_client_via_chain() {
        let mut configs = EtherscanConfigs::default();
        configs.insert(
            "mainnet".to_string(),
            EtherscanConfig {
                chain: Some(Mainnet.into()),
                url: None,
                key: EtherscanApiKey::Key("ABCDEFG".to_string()),
                api_version: None,
            },
        );

        let mut resolved = configs.resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("mainnet").unwrap().unwrap();
        // None version = None
        assert_eq!(config.api_version, EtherscanApiVersion::V2);
        let client = config.into_client().unwrap();
        assert_eq!(*client.etherscan_api_version(), EtherscanApiVersion::V2);
    }

    #[test]
    fn can_create_v1_client_via_chain() {
        let mut configs = EtherscanConfigs::default();
        configs.insert(
            "mainnet".to_string(),
            EtherscanConfig {
                chain: Some(Mainnet.into()),
                url: None,
                api_version: Some(EtherscanApiVersion::V1),
                key: EtherscanApiKey::Key("ABCDEG".to_string()),
            },
        );

        let mut resolved = configs.resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("mainnet").unwrap().unwrap();
        assert_eq!(config.api_version, EtherscanApiVersion::V1);
        let client = config.into_client().unwrap();
        assert_eq!(*client.etherscan_api_version(), EtherscanApiVersion::V1);
    }

    #[test]
    fn can_create_client_via_url_and_chain() {
        let mut configs = EtherscanConfigs::default();
        configs.insert(
            "mainnet".to_string(),
            EtherscanConfig {
                chain: Some(Mainnet.into()),
                url: Some("https://api.etherscan.io/api".to_string()),
                key: EtherscanApiKey::Key("ABCDEFG".to_string()),
                api_version: None,
            },
        );

        let mut resolved = configs.resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("mainnet").unwrap().unwrap();
        let _ = config.into_client().unwrap();
    }

    #[test]
    fn can_create_client_via_url_and_chain_env_var() {
        let mut configs = EtherscanConfigs::default();
        let env = "_CONFIG_ETHERSCAN_API_KEY";
        configs.insert(
            "mainnet".to_string(),
            EtherscanConfig {
                chain: Some(Mainnet.into()),
                url: Some("https://api.etherscan.io/api".to_string()),
                api_version: None,
                key: EtherscanApiKey::Env(format!("${{{env}}}")),
            },
        );

        let mut resolved = configs.clone().resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("mainnet").unwrap();
        assert!(config.is_err());

        unsafe {
            std::env::set_var(env, "ABCDEFG");
        }

        let mut resolved = configs.resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("mainnet").unwrap().unwrap();
        assert_eq!(config.key, "ABCDEFG");
        let client = config.into_client().unwrap();
        assert_eq!(*client.etherscan_api_version(), EtherscanApiVersion::V2);

        unsafe {
            std::env::remove_var(env);
        }
    }

    #[test]
    fn resolve_etherscan_alias_config() {
        let mut configs = EtherscanConfigs::default();
        configs.insert(
            "blast_sepolia".to_string(),
            EtherscanConfig {
                chain: None,
                url: Some("https://api.etherscan.io/api".to_string()),
                key: EtherscanApiKey::Key("ABCDEFG".to_string()),
                api_version: None,
            },
        );

        let mut resolved = configs.clone().resolved(EtherscanApiVersion::V2);
        let config = resolved.remove("blast_sepolia").unwrap().unwrap();
        assert_eq!(config.chain, Some(Chain::blast_sepolia()));
    }

    #[test]
    fn resolve_etherscan_alias() {
        let config = EtherscanConfig {
            chain: None,
            url: Some("https://api.etherscan.io/api".to_string()),
            key: EtherscanApiKey::Key("ABCDEFG".to_string()),
            api_version: None,
        };
        let resolved =
            config.clone().resolve(Some("base_sepolia"), EtherscanApiVersion::V2).unwrap();
        assert_eq!(resolved.chain, Some(Chain::base_sepolia()));

        let resolved = config.resolve(Some("base-sepolia"), EtherscanApiVersion::V2).unwrap();
        assert_eq!(resolved.chain, Some(Chain::base_sepolia()));
    }
}
