//! Support for multiple RPC-endpoints

use crate::resolve::{interpolate, UnresolvedEnvVarError, RE_PLACEHOLDER};
use serde::{ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    fmt,
    ops::{Deref, DerefMut},
};

/// Container type for API endpoints, like various RPC endpoints
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RpcEndpoints {
    endpoints: BTreeMap<String, RpcEndpointConfig>,
}

// === impl RpcEndpoints ===

impl RpcEndpoints {
    /// Creates a new list of endpoints
    pub fn new(
        endpoints: impl IntoIterator<Item = (impl Into<String>, impl Into<RpcEndpointType>)>,
    ) -> Self {
        Self {
            endpoints: endpoints
                .into_iter()
                .map(|(name, e)| match e.into() {
                    RpcEndpointType::String(url) => {
                        (name.into(), RpcEndpointConfig { endpoint: url, ..Default::default() })
                    }
                    RpcEndpointType::Config(config) => (name.into(), config),
                })
                .collect(),
        }
    }

    /// Returns `true` if this type doesn't contain any endpoints
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
    type Target = BTreeMap<String, RpcEndpointConfig>;

    fn deref(&self) -> &Self::Target {
        &self.endpoints
    }
}

/// RPC endpoint wrapper type
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RpcEndpointType {
    /// Raw Endpoint url string
    String(RpcEndpoint),
    /// Config object
    Config(RpcEndpointConfig),
}

impl RpcEndpointType {
    /// Returns the string variant
    pub fn as_endpoint_string(&self) -> Option<&RpcEndpoint> {
        match self {
            RpcEndpointType::String(url) => Some(url),
            RpcEndpointType::Config(_) => None,
        }
    }

    /// Returns the config variant
    pub fn as_endpoint_config(&self) -> Option<&RpcEndpointConfig> {
        match self {
            RpcEndpointType::Config(config) => Some(config),
            RpcEndpointType::String(_) => None,
        }
    }

    /// Returns the url or config this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            RpcEndpointType::String(url) => url.resolve(),
            RpcEndpointType::Config(config) => config.endpoint.resolve(),
        }
    }
}

impl fmt::Display for RpcEndpointType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcEndpointType::String(url) => url.fmt(f),
            RpcEndpointType::Config(config) => config.fmt(f),
        }
    }
}

impl TryFrom<RpcEndpointType> for String {
    type Error = UnresolvedEnvVarError;

    fn try_from(value: RpcEndpointType) -> Result<Self, Self::Error> {
        match value {
            RpcEndpointType::String(url) => url.resolve(),
            RpcEndpointType::Config(config) => config.endpoint.resolve(),
        }
    }
}

/// Represents a single endpoint
///
/// This type preserves the value as it's stored in the config. If the value is a reference to an
/// env var, then the `Endpoint::Env` var will hold the reference (`${MAIN_NET}`) and _not_ the
/// value of the env var itself.
/// In other words, this type does not resolve env vars when it's being deserialized
#[derive(Clone, Debug, PartialEq, Eq)]
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

    /// Returns the env variant
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
            RpcEndpoint::Env(val) => interpolate(&val),
        }
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

impl From<RpcEndpoint> for RpcEndpointType {
    fn from(endpoint: RpcEndpoint) -> Self {
        RpcEndpointType::String(endpoint)
    }
}

impl From<RpcEndpoint> for RpcEndpointConfig {
    fn from(endpoint: RpcEndpoint) -> Self {
        RpcEndpointConfig { endpoint, ..Default::default() }
    }
}

/// Rpc endpoint configuration variant
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcEndpointConfig {
    /// endpoint url or env
    pub endpoint: RpcEndpoint,

    /// The number of retries.
    pub retries: Option<u32>,

    /// Initial retry backoff.
    pub retry_backoff: Option<u64>,

    /// The available compute units per second.
    ///
    /// See also <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    pub compute_units_per_second: Option<u64>,
}

impl RpcEndpointConfig {
    /// Returns the url this type holds, see [RpcEndpoints::resolve()]
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        self.endpoint.resolve()
    }
}

impl fmt::Display for RpcEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let RpcEndpointConfig { endpoint, retries, retry_backoff, compute_units_per_second } = self;

        write!(f, "{}", endpoint)?;

        if let Some(retries) = retries {
            write!(f, ", retries={}", retries)?;
        }

        if let Some(retry_backoff) = retry_backoff {
            write!(f, ", retry_backoff={}", retry_backoff)?;
        }

        if let Some(compute_units_per_second) = compute_units_per_second {
            write!(f, ", compute_units_per_second={}", compute_units_per_second)?;
        }

        Ok(())
    }
}

impl Serialize for RpcEndpointConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.retries.is_none() &&
            self.retry_backoff.is_none() &&
            self.compute_units_per_second.is_none()
        {
            // serialize as endpoint if there's no additional config
            self.endpoint.serialize(serializer)
        } else {
            let mut map = serializer.serialize_map(Some(4))?;
            map.serialize_entry("endpoint", &self.endpoint)?;
            map.serialize_entry("retries", &self.retries)?;
            map.serialize_entry("retry_backoff", &self.retry_backoff)?;
            map.serialize_entry("compute_units_per_second", &self.compute_units_per_second)?;
            map.end()
        }
    }
}

impl<'de> Deserialize<'de> for RpcEndpointConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_string() {
            return Ok(Self {
                endpoint: serde_json::from_value(value).map_err(serde::de::Error::custom)?,
                ..Default::default()
            });
        }

        #[derive(Deserialize)]
        struct RpcEndpointConfigInner {
            #[serde(alias = "url")]
            endpoint: RpcEndpoint,
            retries: Option<u32>,
            retry_backoff: Option<u64>,
            compute_units_per_second: Option<u64>,
        }

        let RpcEndpointConfigInner { endpoint, retries, retry_backoff, compute_units_per_second } =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        Ok(RpcEndpointConfig { endpoint, retries, retry_backoff, compute_units_per_second })
    }
}

impl From<RpcEndpointConfig> for RpcEndpointType {
    fn from(config: RpcEndpointConfig) -> Self {
        RpcEndpointType::Config(config)
    }
}

impl Default for RpcEndpointConfig {
    fn default() -> Self {
        Self {
            endpoint: RpcEndpoint::Url("http://localhost:8545".to_string()),
            retries: None,
            retry_backoff: None,
            compute_units_per_second: None,
        }
    }
}

/// Container type for _resolved_ endpoints, see [RpcEndpoints::resolve_all()]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedRpcEndpoints {
    /// contains all named endpoints and their URL or an error if we failed to resolve the env var
    /// alias
    endpoints: BTreeMap<String, Result<String, UnresolvedEnvVarError>>,
}

// === impl ResolvedEndpoints ===

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

impl DerefMut for ResolvedRpcEndpoints {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_rpc_config() {
        let s = r#"{
            "endpoint": "http://localhost:8545",
            "retries": 5,
            "retry_backoff": 250,
            "compute_units_per_second": 100
        }"#;
        let config: RpcEndpointConfig = serde_json::from_str(s).unwrap();
        assert_eq!(
            config,
            RpcEndpointConfig {
                endpoint: RpcEndpoint::Url("http://localhost:8545".to_string()),
                retries: Some(5),
                retry_backoff: Some(250),
                compute_units_per_second: Some(100),
            }
        );

        let s = "\"http://localhost:8545\"";
        let config: RpcEndpointConfig = serde_json::from_str(s).unwrap();
        assert_eq!(
            config,
            RpcEndpointConfig {
                endpoint: RpcEndpoint::Url("http://localhost:8545".to_string()),
                retries: None,
                retry_backoff: None,
                compute_units_per_second: None,
            }
        );
    }
}
