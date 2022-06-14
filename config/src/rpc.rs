//! Support for multiple RPC-endpoints

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, env, env::VarError, fmt};

/// Container type for rpc endpoints
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RpcEndpoints {
    endpoints: BTreeMap<String, RpcEndpoint>,
}

// === impl RpcEndpoints ===

impl RpcEndpoints {
    /// Creates anew list of endpoints
    pub fn new(endpoints: impl IntoIterator<Item = (impl Into<String>, RpcEndpoint)>) -> Self {
        Self { endpoints: endpoints.into_iter().map(|(name, url)| (name.into(), url)).collect() }
    }

    /// Returns `true` if this type holds no endpoints
    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// Returns all (alias -> url) pairs
    ///
    /// # Errors
    ///
    /// returns an error if it contains a reference to an env var that is not set
    pub fn resolve_all(self) -> Result<BTreeMap<String, String>, VarError> {
        self.endpoints.into_iter().map(|(name, e)| (e.resolve().map(|url| (name, url)))).collect()
    }
}

/// Represents a single endpoint
///
/// This type preserves the value as it's stored in the config. If the value is a reference to an
/// env var, then the `RpcEndpoint::Env` var will hold the reference (`${MAIN_NET}`) and _not_ the
/// value of the env var itself.
/// In other words, this type does not resolve env vars when it's being deserialized
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcEndpoint {
    /// A raw Url (ws, http)
    Url(String),
    // Reference to an env var in the form of `${ENV_VAR}`
    Env(String),
}

// === impl RpcEndpoint ===

impl RpcEndpoint {
    /// Returns the url variant
    pub fn as_url(&self) -> Option<&str> {
        match self {
            RpcEndpoint::Url(url) => Some(url),
            RpcEndpoint::Env(_) => None,
        }
    }

    /// Returns the url variant
    pub fn as_env(&self) -> Option<&str> {
        match self {
            RpcEndpoint::Env(val) => Some(val),
            RpcEndpoint::Url(_) => None,
        }
    }

    /// Returns the url this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, VarError> {
        match self {
            RpcEndpoint::Url(url) => Ok(url),
            RpcEndpoint::Env(v) => env::var(v),
        }
    }
}

impl fmt::Display for RpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcEndpoint::Url(url) => url.fmt(f),
            RpcEndpoint::Env(var) => {
                write!(f, "${{{var}}}")
            }
        }
    }
}

impl TryFrom<RpcEndpoint> for String {
    type Error = VarError;

    fn try_from(value: RpcEndpoint) -> Result<Self, Self::Error> {
        value.resolve()
    }
}

impl Serialize for RpcEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RpcEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = String::deserialize(deserializer)?;
        let endpoint = if val.starts_with('$') {
            RpcEndpoint::Env(parse_env_ref(&val))
        } else {
            RpcEndpoint::Url(val)
        };

        Ok(endpoint)
    }
}

/// Extracts the value surrounded by `${<val>}`
///
/// TODO(mattsse): make this a bit more sophisticated
fn parse_env_ref(val: &str) -> String {
    val.trim_start_matches("${").trim_end_matches('}').to_string()
}
