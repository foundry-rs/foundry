use crate::{
    RpcEndpoint,
    error::ExtractConfigError,
    resolve::{RE_PLACEHOLDER, UnresolvedEnvVarError, interpolate},
};

use alloy_chains::Chain;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashMap, fmt, ops::Deref, str::FromStr};

/// Fork-related cheatcode configuration for both, tests and scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ForkConfigs {
    // Whether the `writeFork..` cheatcodes can be used to modify the `.toml` files or not.
    // By default, limited to read-only access.
    pub access: ForkConfigPermission,
    // Individual configs for each chain.
    #[serde(flatten)]
    pub chain_configs: HashMap<Chain, ForkChainConfig>,
}

impl ForkConfigs {
    /// Resolve environment variables in all fork config fields
    pub fn resolve_env_vars(&mut self) -> Result<(), ExtractConfigError> {
        for (name, fork_config) in &mut self.chain_configs {
            // Take temporary ownership of the config, so that it can be consumed.
            let config = std::mem::take(fork_config);

            // Resolve the env vars and place it back into the map.
            *fork_config = config.resolved().map_err(|e| {
                let msg = if !e.var.is_empty() {
                    format!("environment variable `{}` not found", e.var)
                } else {
                    e.to_string()
                };
                ExtractConfigError::new(figment::Error::from(format!(
                    "Failed to resolve fork config [forks.{name}]: {msg}"
                )))
            })?;
        }

        Ok(())
    }
}

impl Deref for ForkConfigs {
    type Target = HashMap<Chain, ForkChainConfig>;

    fn deref(&self) -> &Self::Target {
        &self.chain_configs
    }
}

/// Chain-scoped configuration for fork-related cheatcodes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ForkChainConfig {
    // Optional RPC endpoint for the fork.
    //
    // If uninformed, it will attempt to load one from `[rpc_endpoints]` with a matching alias
    // for the name of the forked chain.
    pub rpc_endpoint: Option<RpcEndpoint>,
    // Any arbitrary key-value pair of variables.
    pub vars: HashMap<String, toml::Value>,
}

impl ForkChainConfig {
    /// Resolves environment variables in the fork configuration.
    /// Returns a new ForkConfig with all environment variables resolved.
    pub fn resolved(self) -> Result<Self, UnresolvedEnvVarError> {
        let mut resolved_vars = HashMap::new();
        for (key, value) in self.vars {
            resolved_vars.insert(key, resolve_toml_value(value)?);
        }

        Ok(Self { rpc_endpoint: self.rpc_endpoint, vars: resolved_vars })
    }
}

/// Recursively resolves environment variables in a toml::Value
fn resolve_toml_value(value: toml::Value) -> Result<toml::Value, UnresolvedEnvVarError> {
    match value {
        toml::Value::String(s) => {
            // Check if the string contains environment variable placeholders
            if RE_PLACEHOLDER.is_match(&s) {
                // Resolve the environment variables
                let resolved = interpolate(&s)?;
                Ok(toml::Value::String(resolved))
            } else {
                Ok(toml::Value::String(s))
            }
        }
        toml::Value::Array(arr) => {
            // Recursively resolve each element in the array
            let resolved_arr: Result<Vec<_>, _> = arr.into_iter().map(resolve_toml_value).collect();
            Ok(toml::Value::Array(resolved_arr?))
        }
        toml::Value::Table(table) => {
            // Recursively resolve each value in the table
            let mut resolved_table = toml::map::Map::new();
            for (k, v) in table {
                resolved_table.insert(k, resolve_toml_value(v)?);
            }
            Ok(toml::Value::Table(resolved_table))
        }
        // Other types don't need resolution
        other => Ok(other),
    }
}

/// Determines the status of file system access
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForkConfigPermission {
    /// Only reading is allowed
    #[default]
    Read,
    /// Writing is also allowed
    ReadWrite,
}

impl ForkConfigPermission {
    /// Returns true if write access is allowed
    pub fn can_write(&self) -> bool {
        match self {
            Self::ReadWrite => true,
            Self::Read => false,
        }
    }
}

impl FromStr for ForkConfigPermission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "true" | "read-write" | "readwrite" | "write" => Ok(Self::ReadWrite),
            "false" | "read" => Ok(Self::Read),
            _ => Err(format!("Unknown variant {s}")),
        }
    }
}

impl fmt::Display for ForkConfigPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadWrite => f.write_str("read-write"),
            Self::Read => f.write_str("read"),
        }
    }
}

impl Serialize for ForkConfigPermission {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::ReadWrite => serializer.serialize_bool(true),
            Self::Read => serializer.serialize_bool(false),
        }
    }
}

impl<'de> Deserialize<'de> for ForkConfigPermission {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Status {
            Bool(bool),
            String(String),
        }
        match Status::deserialize(deserializer)? {
            Status::Bool(enabled) => {
                let status = if enabled { Self::ReadWrite } else { Self::Read };
                Ok(status)
            }
            Status::String(val) => val.parse().map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_permission() {
        assert_eq!(ForkConfigPermission::ReadWrite, "true".parse().unwrap());
        assert_eq!(ForkConfigPermission::ReadWrite, "readwrite".parse().unwrap());
        assert_eq!(ForkConfigPermission::ReadWrite, "read-write".parse().unwrap());
        assert_eq!(ForkConfigPermission::ReadWrite, "write".parse().unwrap());
        assert_eq!(ForkConfigPermission::Read, "false".parse().unwrap());
        assert_eq!(ForkConfigPermission::Read, "read".parse().unwrap());
    }
}
