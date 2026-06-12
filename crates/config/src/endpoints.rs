//! Support for multiple RPC-endpoints

use crate::resolve::{RE_PLACEHOLDER, UnresolvedEnvVarError, interpolate};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use std::{
    collections::BTreeMap,
    fmt,
    ops::{Deref, DerefMut},
};

/// Container type for API endpoints, like various RPC endpoints
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RpcEndpoints {
    endpoints: BTreeMap<String, RpcEndpoint>,
}

impl RpcEndpoints {
    /// Creates a new list of endpoints
    pub fn new(
        endpoints: impl IntoIterator<Item = (impl Into<String>, impl Into<RpcEndpointType>)>,
    ) -> Self {
        Self {
            endpoints: endpoints
                .into_iter()
                .map(|(name, e)| match e.into() {
                    RpcEndpointType::String(url) => (name.into(), RpcEndpoint::new(url)),
                    RpcEndpointType::Config(config) => (name.into(), config),
                })
                .collect(),
        }
    }

    /// Returns `true` if this type doesn't contain any endpoints
    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// Returns all (alias -> rpc_endpoint) pairs
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

/// RPC endpoint wrapper type
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RpcEndpointType {
    /// Raw Endpoint url string
    String(RpcEndpointUrl),
    /// Config object
    Config(RpcEndpoint),
}

impl RpcEndpointType {
    /// Returns the url or config this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            Self::String(url) => url.resolve(),
            Self::Config(config) => config.endpoint.resolve(),
        }
    }
}

impl fmt::Display for RpcEndpointType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(url) => url.fmt(f),
            Self::Config(config) => config.fmt(f),
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
pub enum RpcEndpointUrl {
    /// A raw Url (ws, http)
    Url(String),
    /// An endpoint that contains at least one `${ENV_VAR}` placeholder
    ///
    /// **Note:** this contains the endpoint as is, like `https://eth-mainnet.alchemyapi.io/v2/${API_KEY}` or `${EPC_ENV_VAR}`
    Env(String),
}

impl RpcEndpointUrl {
    /// Returns the url variant
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Self::Url(url) => Some(url),
            Self::Env(_) => None,
        }
    }

    /// Returns the url this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            Self::Url(url) => Ok(url),
            Self::Env(val) => interpolate(&val),
        }
    }
}

impl fmt::Display for RpcEndpointUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => url.fmt(f),
            Self::Env(var) => var.fmt(f),
        }
    }
}

impl TryFrom<RpcEndpointUrl> for String {
    type Error = UnresolvedEnvVarError;

    fn try_from(value: RpcEndpointUrl) -> Result<Self, Self::Error> {
        value.resolve()
    }
}

impl Serialize for RpcEndpointUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RpcEndpointUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = String::deserialize(deserializer)?;
        let endpoint = if RE_PLACEHOLDER.is_match(&val) { Self::Env(val) } else { Self::Url(val) };

        Ok(endpoint)
    }
}

impl From<RpcEndpointUrl> for RpcEndpointType {
    fn from(endpoint: RpcEndpointUrl) -> Self {
        Self::String(endpoint)
    }
}

impl From<RpcEndpointUrl> for RpcEndpoint {
    fn from(endpoint: RpcEndpointUrl) -> Self {
        Self { endpoint, ..Default::default() }
    }
}

/// The auth token to be used for RPC endpoints
/// It works in the same way as the `RpcEndpoint` type, where it can be a raw string or a reference
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcAuth {
    Raw(String),
    Env(String),
}

impl RpcAuth {
    /// Returns the auth token this type holds
    ///
    /// # Error
    ///
    /// Returns an error if the type holds a reference to an env var and the env var is not set
    pub fn resolve(self) -> Result<String, UnresolvedEnvVarError> {
        match self {
            Self::Raw(raw_auth) => Ok(raw_auth),
            Self::Env(var) => interpolate(&var),
        }
    }
}

impl fmt::Display for RpcAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw(url) => url.fmt(f),
            Self::Env(var) => var.fmt(f),
        }
    }
}

impl Serialize for RpcAuth {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RpcAuth {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = String::deserialize(deserializer)?;
        let auth = if RE_PLACEHOLDER.is_match(&val) { Self::Env(val) } else { Self::Raw(val) };

        Ok(auth)
    }
}

// Rpc endpoint configuration
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RpcEndpointConfig {
    /// The number of retries.
    pub retries: Option<u32>,

    /// Initial retry backoff.
    pub retry_backoff: Option<u64>,

    /// The available compute units per second.
    ///
    /// See also <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    pub compute_units_per_second: Option<u64>,
}

impl fmt::Display for RpcEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { retries, retry_backoff, compute_units_per_second } = self;

        if let Some(retries) = retries {
            write!(f, ", retries={retries}")?;
        }

        if let Some(retry_backoff) = retry_backoff {
            write!(f, ", retry_backoff={retry_backoff}")?;
        }

        if let Some(compute_units_per_second) = compute_units_per_second {
            write!(f, ", compute_units_per_second={compute_units_per_second}")?;
        }

        Ok(())
    }
}

/// Rpc endpoint configuration variant
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcEndpoint {
    /// endpoint url or env
    pub endpoint: RpcEndpointUrl,

    /// Additional fallback endpoints for load-balanced multi-endpoint forking.
    /// When set, requests are distributed across all endpoints (primary + extra)
    /// with automatic failover.
    pub extra_endpoints: Vec<RpcEndpointUrl>,

    /// Token to be used as authentication
    pub auth: Option<RpcAuth>,

    /// additional configuration
    pub config: RpcEndpointConfig,
}

impl RpcEndpoint {
    pub fn new(endpoint: RpcEndpointUrl) -> Self {
        Self { endpoint, ..Default::default() }
    }

    /// Resolves environment variables in fields into their raw values
    pub fn resolve(self) -> ResolvedRpcEndpoint {
        ResolvedRpcEndpoint {
            endpoint: self.endpoint.resolve(),
            extra_endpoints: self.extra_endpoints.into_iter().map(|e| e.resolve()).collect(),
            auth: self.auth.map(|auth| auth.resolve()),
            config: self.config,
        }
    }
}

impl fmt::Display for RpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { endpoint, auth, config, .. } = self;
        write!(f, "{endpoint}")?;
        write!(f, "{config}")?;
        if let Some(auth) = auth {
            write!(f, ", auth={auth}")?;
        }
        Ok(())
    }
}

impl Serialize for RpcEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let has_config = self.config.retries.is_some()
            || self.config.retry_backoff.is_some()
            || self.config.compute_units_per_second.is_some()
            || self.auth.is_some();

        if !has_config && self.extra_endpoints.is_empty() {
            // serialize as plain endpoint string if there's no additional config
            self.endpoint.serialize(serializer)
        } else {
            let mut map = serializer.serialize_map(None)?;
            if self.extra_endpoints.is_empty() {
                map.serialize_entry("endpoint", &self.endpoint)?;
            } else {
                // Serialize all endpoints as an array under "endpoints"
                let all: Vec<&RpcEndpointUrl> =
                    std::iter::once(&self.endpoint).chain(&self.extra_endpoints).collect();
                map.serialize_entry("endpoints", &all)?;
            }
            map.serialize_entry("retries", &self.config.retries)?;
            map.serialize_entry("retry_backoff", &self.config.retry_backoff)?;
            map.serialize_entry("compute_units_per_second", &self.config.compute_units_per_second)?;
            map.serialize_entry("auth", &self.auth)?;
            map.end()
        }
    }
}

impl<'de> Deserialize<'de> for RpcEndpoint {
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

        // Support both single "endpoint" and array "endpoints" for backwards compatibility
        #[derive(Deserialize)]
        struct RpcEndpointConfigInner {
            #[serde(alias = "url")]
            endpoint: Option<RpcEndpointUrl>,
            /// Array of endpoint URLs for multi-endpoint load balancing
            endpoints: Option<Vec<RpcEndpointUrl>>,
            retries: Option<u32>,
            retry_backoff: Option<u64>,
            compute_units_per_second: Option<u64>,
            auth: Option<RpcAuth>,
        }

        let RpcEndpointConfigInner {
            endpoint,
            endpoints,
            retries,
            retry_backoff,
            compute_units_per_second,
            auth,
        } = serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        let (primary, extra) = match (endpoint, endpoints) {
            // Single endpoint: endpoint = "..."
            (Some(ep), None) => (ep, vec![]),
            // Array of endpoints: endpoints = ["...", "..."]
            (None, Some(mut eps)) => {
                if eps.is_empty() {
                    return Err(serde::de::Error::custom(
                        "endpoints array must contain at least one URL",
                    ));
                }
                let primary = eps.remove(0);
                (primary, eps)
            }
            // Both provided — error
            (Some(_), Some(_)) => {
                return Err(serde::de::Error::custom(
                    "cannot specify both `endpoint` and `endpoints`",
                ));
            }
            // Neither provided — error
            (None, None) => {
                return Err(serde::de::Error::custom(
                    "must specify either `endpoint` or `endpoints`",
                ));
            }
        };

        Ok(Self {
            endpoint: primary,
            extra_endpoints: extra,
            auth,
            config: RpcEndpointConfig { retries, retry_backoff, compute_units_per_second },
        })
    }
}

impl From<RpcEndpoint> for RpcEndpointType {
    fn from(config: RpcEndpoint) -> Self {
        Self::Config(config)
    }
}

impl Default for RpcEndpoint {
    fn default() -> Self {
        Self {
            endpoint: RpcEndpointUrl::Url("http://localhost:8545".to_string()),
            extra_endpoints: vec![],
            config: RpcEndpointConfig::default(),
            auth: None,
        }
    }
}

/// Rpc endpoint with environment variables resolved to values, see [`RpcEndpoint::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRpcEndpoint {
    pub endpoint: Result<String, UnresolvedEnvVarError>,
    /// Additional resolved endpoints for multi-endpoint load balancing.
    pub extra_endpoints: Vec<Result<String, UnresolvedEnvVarError>>,
    pub auth: Option<Result<String, UnresolvedEnvVarError>>,
    pub config: RpcEndpointConfig,
}

impl ResolvedRpcEndpoint {
    /// Returns the primary url this type holds, see [`RpcEndpoint::resolve`]
    pub fn url(&self) -> Result<String, UnresolvedEnvVarError> {
        self.endpoint.clone()
    }

    /// Returns all resolved URLs (primary + extra) for multi-endpoint configurations.
    /// Returns an empty vec if no extra endpoints are configured.
    pub fn all_urls(&self) -> Result<Vec<String>, UnresolvedEnvVarError> {
        let primary = self.endpoint.clone()?;
        if self.extra_endpoints.is_empty() {
            return Ok(vec![primary]);
        }
        let mut urls = vec![primary];
        for ep in &self.extra_endpoints {
            urls.push(ep.clone()?);
        }
        Ok(urls)
    }

    // Returns true if all environment variables are resolved successfully
    pub fn is_unresolved(&self) -> bool {
        let endpoint_err = self.endpoint.is_err();
        let extra_err = self.extra_endpoints.iter().any(|e| e.is_err());
        let auth_err = self.auth.as_ref().map(|auth| auth.is_err()).unwrap_or(false);
        endpoint_err || extra_err || auth_err
    }

    // Attempts to resolve unresolved environment variables into a new instance
    pub fn try_resolve(mut self) -> Self {
        if !self.is_unresolved() {
            return self;
        }
        if let Err(err) = self.endpoint {
            self.endpoint = err.try_resolve()
        }
        for ep in &mut self.extra_endpoints {
            if let Err(err) = std::mem::replace(ep, Ok(String::new())) {
                *ep = err.try_resolve();
            }
        }
        if let Some(Err(err)) = self.auth {
            self.auth = Some(err.try_resolve())
        }
        self
    }
}

/// Container type for _resolved_ endpoints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedRpcEndpoints {
    endpoints: BTreeMap<String, ResolvedRpcEndpoint>,
}

impl ResolvedRpcEndpoints {
    /// Returns true if there's an endpoint that couldn't be resolved
    pub fn has_unresolved(&self) -> bool {
        self.endpoints.values().any(|e| e.is_unresolved())
    }
}

impl Deref for ResolvedRpcEndpoints {
    type Target = BTreeMap<String, ResolvedRpcEndpoint>;

    fn deref(&self) -> &Self::Target {
        &self.endpoints
    }
}

impl DerefMut for ResolvedRpcEndpoints {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoints
    }
}

/// Returns the URL for a built-in RPC alias, if one exists.
///
/// Built-in aliases act as fallbacks: they are only used when the alias has **not** been
/// defined by the user in `[rpc_endpoints]` or resolved via MESC.
pub fn builtin_rpc_url(alias: &str) -> Option<&'static str> {
    match alias {
        "tempo" => Some("https://rpc.mpp.tempo.xyz"),
        "moderato" => Some("https://rpc.mpp.moderato.tempo.xyz"),
        _ => None,
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
            "compute_units_per_second": 100,
            "auth": "Bearer 123"
        }"#;
        let config: RpcEndpoint = serde_json::from_str(s).unwrap();
        assert_eq!(
            config,
            RpcEndpoint {
                endpoint: RpcEndpointUrl::Url("http://localhost:8545".to_string()),
                extra_endpoints: vec![],
                config: RpcEndpointConfig {
                    retries: Some(5),
                    retry_backoff: Some(250),
                    compute_units_per_second: Some(100),
                },
                auth: Some(RpcAuth::Raw("Bearer 123".to_string())),
            }
        );

        let s = "\"http://localhost:8545\"";
        let config: RpcEndpoint = serde_json::from_str(s).unwrap();
        assert_eq!(
            config,
            RpcEndpoint {
                endpoint: RpcEndpointUrl::Url("http://localhost:8545".to_string()),
                extra_endpoints: vec![],
                config: RpcEndpointConfig {
                    retries: None,
                    retry_backoff: None,
                    compute_units_per_second: None,
                },
                auth: None,
            }
        );
    }

    #[test]
    fn serde_rpc_config_multi_endpoints() {
        // Array of endpoints via "endpoints" key
        let s = r#"{
            "endpoints": ["https://rpc1.example.com", "https://rpc2.example.com", "https://rpc3.example.com"],
            "retries": 5,
            "retry_backoff": 1000
        }"#;
        let config: RpcEndpoint = serde_json::from_str(s).unwrap();
        assert_eq!(
            config,
            RpcEndpoint {
                endpoint: RpcEndpointUrl::Url("https://rpc1.example.com".to_string()),
                extra_endpoints: vec![
                    RpcEndpointUrl::Url("https://rpc2.example.com".to_string()),
                    RpcEndpointUrl::Url("https://rpc3.example.com".to_string()),
                ],
                config: RpcEndpointConfig {
                    retries: Some(5),
                    retry_backoff: Some(1000),
                    compute_units_per_second: None,
                },
                auth: None,
            }
        );

        // Resolved URLs
        let resolved = config.resolve();
        let all_urls = resolved.all_urls().unwrap();
        assert_eq!(
            all_urls,
            vec![
                "https://rpc1.example.com".to_string(),
                "https://rpc2.example.com".to_string(),
                "https://rpc3.example.com".to_string(),
            ]
        );
    }

    #[test]
    fn serde_rpc_config_rejects_both_endpoint_and_endpoints() {
        let s = r#"{
            "endpoint": "https://rpc1.example.com",
            "endpoints": ["https://rpc2.example.com"]
        }"#;
        let result: Result<RpcEndpoint, _> = serde_json::from_str(s);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot specify both"));
    }

    #[test]
    fn serde_rpc_config_rejects_empty_endpoints() {
        let s = r#"{ "endpoints": [] }"#;
        let result: Result<RpcEndpoint, _> = serde_json::from_str(s);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one URL"));
    }
}
