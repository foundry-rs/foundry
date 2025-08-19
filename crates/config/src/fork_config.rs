use crate::{
    RpcEndpoint,
    error::ExtractConfigError,
    resolve::{RE_PLACEHOLDER, UnresolvedEnvVarError, interpolate},
};

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Deref};

/// Fork-scoped config for tests and scripts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ForkConfigs(pub HashMap<String, ForkChainConfig>);

impl ForkConfigs {
    /// Resolve environment variables in all fork config fields
    pub fn resolve_env_vars(&mut self) -> Result<(), ExtractConfigError> {
        for (name, fork_config) in &mut self.0 {
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
    type Target = HashMap<String, ForkChainConfig>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Fork-scoped config for tests and scripts.
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
