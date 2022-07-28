//! Support for multiple RPC-endpoints

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, env, env::VarError, fmt, ops::Deref};

/// A regex that matches `${val}` placeholders
pub static RE_PLACEHOLDER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)(?P<outer>\$\{\s*(?P<inner>.*?)\s*})").unwrap());

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
    pub fn resolved(self) -> ResolvedRpcEndpoints {
        ResolvedRpcEndpoints {
            endpoints: self.endpoints.into_iter().map(|(name, e)| (name, e.resolve())).collect(),
        }
    }
}

impl Deref for RpcEndpoints {
    type Target = BTreeMap<String, RpcEndpoint>;

    fn deref(&self) -> &Self::Target {
        &self.endpoints
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
    /// An endpoint that contains at least one `${ENV_VAR}` placeholder
    ///
    /// **Note:** this contains the endpoint as is, like `https://eth-mainnet.alchemyapi.io/v2/${API_KEY}` or `${EPC_ENV_VAR}`
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
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            RpcEndpoint::Url(url) => Ok(url),
            RpcEndpoint::Env(val) => Self::interpolate(&val),
        }
    }

    /// Replaces all Env var placeholders in the input string with the values they hold
    pub fn interpolate(input: &str) -> Result<String, UnresolvedEnvVarError> {
        let mut res = input.to_string();

        // loop over all placeholders in the input and replace them one by one
        for caps in RE_PLACEHOLDER.captures_iter(input) {
            let var = &caps["inner"];
            let value = env::var(var)
                .map_err(|source| UnresolvedEnvVarError { var: var.to_string(), source })?;

            res = res.replacen(&caps["outer"], &value, 1);
        }
        Ok(res)
    }
}

impl fmt::Display for RpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcEndpoint::Url(url) => url.fmt(f),
            RpcEndpoint::Env(var) => var.fmt(f),
        }
    }
}

impl TryFrom<RpcEndpoint> for String {
    type Error = UnresolvedEnvVarError;

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
        let endpoint = if RE_PLACEHOLDER.is_match(&val) {
            RpcEndpoint::Env(val)
        } else {
            RpcEndpoint::Url(val)
        };

        Ok(endpoint)
    }
}

/// Container type for _resolved_ RPC endpoints, see [RpcEndpoints::resolve_all()]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedRpcEndpoints {
    /// contains all named endpoints and their URL or an error if we failed to resolve the env var
    /// alias
    endpoints: BTreeMap<String, Result<String, UnresolvedEnvVarError>>,
}

// === impl ResolvedRpcEndpoints ===

impl ResolvedRpcEndpoints {
    /// Returns true if there's an endpoint that couldn't be resolved
    pub fn has_unresolved(&self) -> bool {
        self.endpoints.values().any(|val| val.is_err())
    }
}

impl Deref for ResolvedRpcEndpoints {
    type Target = BTreeMap<String, Result<String, UnresolvedEnvVarError>>;

    fn deref(&self) -> &Self::Target {
        &self.endpoints
    }
}

/// Error when we failed to resolve an env var
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedEnvVarError {
    /// Var that couldn't be resolved
    pub var: String,
    /// the `env::var` error
    pub source: VarError,
}

impl fmt::Display for UnresolvedEnvVarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to resolve env var `{}`: {}", self.var, self.source)
    }
}

impl std::error::Error for UnresolvedEnvVarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_find_placeholder() {
        let val = "https://eth-mainnet.alchemyapi.io/v2/346273846238426342";
        assert!(!RE_PLACEHOLDER.is_match(val));

        let val = "${RPC_ENV}";
        assert!(RE_PLACEHOLDER.is_match(val));

        let val = "https://eth-mainnet.alchemyapi.io/v2/${API_KEY}";
        assert!(RE_PLACEHOLDER.is_match(val));

        let cap = RE_PLACEHOLDER.captures(val).unwrap();
        assert_eq!(cap.name("outer").unwrap().as_str(), "${API_KEY}");
        assert_eq!(cap.name("inner").unwrap().as_str(), "API_KEY");
    }
}
