//! Support for multiple etherscan keys
use crate::{
    resolve::{UnresolvedEnvVarError, RE_PLACEHOLDER},
    Chain,
};
use ethers_core::types::Chain as NamedChain;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, env, env::VarError, fmt, ops::Deref};

/// Container type for API endpoints, like various RPC endpoints
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EtherscanConfigs {
    configs: BTreeMap<String, EtherscanConfig>,
}

// === impl Endpoints ===

impl EtherscanConfigs {
    /// Creates a new list of etherscan configs
    pub fn new(configs: impl IntoIterator<Item = (impl Into<String>, EtherscanConfig)>) -> Self {
        Self { configs: configs.into_iter().map(|(name, url)| (name.into(), url)).collect() }
    }

    /// Returns `true` if this type holds no endpoints
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Returns all (alias -> url) pairs
    pub fn resolved(self) -> ResolvedEtherscanConfigs {
        ResolvedEtherscanConfigs {
            configs: self.configs.into_iter().map(|(name, e)| (name, e.resolve())).collect(),
        }
    }
}

/// Container type for _resolved_ etherscan keys, see [EtherscanConfigs::resolve_all()]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedEtherscanConfigs {
    /// contains all named `ResolvedEtherscanConfig` or an error if we failed to resolve the env
    /// var alias
    configs: BTreeMap<String, Result<ResolvedEtherscanConfig, UnresolvedEnvVarError>>,
}

/// Represents all info required to create an etherscan client
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtherscanConfig {
    /// Chain name/id that can be used to derive the api url
    pub chain: Option<Chain>,
    /// Etherscan API URL
    pub url: Option<String>,
    /// The etherscan API KEY that's required to make requests
    pub key: EtherscanApiKey,
}

// === impl EtherscanConfig ===

impl EtherscanConfig {
    /// Returns the etherscan config required to create a client
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    /// Or no chain or url is configured
    pub fn resolve(self) -> Result<ResolvedEtherscanConfig, UnresolvedEnvVarError> {
        let EtherscanConfig { chain, url, key } = self;
        match (chain, url) {
            (Some(_), Some(url)) => url,
            (Some(chain), None) => if let Ok(chain) = NamedChain::try_from(chain) {},
        }

        todo!()
    }
}

/// Contains required url + api key to set up an etherscan client
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEtherscanConfig {
    /// Etherscan API URL
    pub url: String,
    /// Resolved api key
    pub api_key: String,
}

/// Represents a single etherscan API key
///
/// This type preserves the value as it's stored in the config. If the value is a reference to an
/// env var, then the `EtherscanKey::Key` var will hold the reference (`${MAIN_NET}`) and _not_ the
/// value of the env var itself.
/// In other words, this type does not resolve env vars when it's being deserialized
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtherscanApiKey {
    /// A raw key
    Key(String),
    /// An endpoint that contains at least one `${ENV_VAR}` placeholder
    ///
    /// **Note:** this contains the key or `${ETHERSCAN_KEY}`
    Env(String),
}

// === impl EtherscanApiKey ===

impl EtherscanApiKey {
    /// Returns the key variant
    pub fn as_key(&self) -> Option<&str> {
        match self {
            EtherscanApiKey::Key(url) => Some(url),
            EtherscanApiKey::Env(_) => None,
        }
    }

    /// Returns the env variant
    pub fn as_env(&self) -> Option<&str> {
        match self {
            EtherscanApiKey::Env(val) => Some(val),
            EtherscanApiKey::Key(_) => None,
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
        let endpoint = if RE_PLACEHOLDER.is_match(&val) {
            EtherscanApiKey::Env(val)
        } else {
            EtherscanApiKey::Key(val)
        };

        Ok(endpoint)
    }
}
