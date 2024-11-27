use crate::Config;
use alloy_primitives::map::HashMap;
use figment::{
    value::{Dict, Map, Value},
    Figment, Profile, Provider,
};
use itertools::Itertools;

mod error;
pub use error::*;

mod natspec;
pub use natspec::*;

const INLINE_CONFIG_PREFIX: &str = "forge-config:";

type DataMap = Map<Profile, Dict>;

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to a solidity test.
#[derive(Clone, Debug, Default)]
pub struct InlineConfig {
    /// Contract-level configuration.
    contract_level: HashMap<String, DataMap>,
    /// Function-level configuration.
    fn_level: HashMap<(String, String), DataMap>,
}

impl InlineConfig {
    /// Creates a new, empty [`InlineConfig`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a new [`NatSpec`] into the [`InlineConfig`].
    pub fn insert(&mut self, natspec: &NatSpec) -> eyre::Result<()> {
        let map = if let Some(function) = &natspec.function {
            self.fn_level.entry((natspec.contract.clone(), function.clone())).or_default()
        } else {
            self.contract_level.entry(natspec.contract.clone()).or_default()
        };
        let joined = natspec.config_values().format("\n").to_string();
        extend_data_map(map, &toml::from_str::<DataMap>(&joined)?);
        Ok(())
    }

    /// Returns a [`figment::Provider`] for this [`InlineConfig`] at the given contract and function
    /// level.
    pub fn provide<'a>(&'a self, contract: &'a str, function: &'a str) -> InlineConfigProvider<'a> {
        InlineConfigProvider { inline: self, contract, function }
    }

    /// Merges the inline configuration at the given contract and function level with the provided
    /// base configuration.
    pub fn merge(&self, contract: &str, function: &str, base: &Config) -> Figment {
        Figment::from(base).merge(self.provide(contract, function))
    }

    /// Returns `true` if a configuration is present at the given contract and function level.
    pub fn contains(&self, contract: &str, function: &str) -> bool {
        // Order swapped to avoid allocation in `get_function` since order doesn't matter here.
        self.get_contract(contract)
            .filter(|map| !map.is_empty())
            .or_else(|| self.get_function(contract, function))
            .is_some_and(|map| !map.is_empty())
    }

    fn get_contract(&self, contract: &str) -> Option<&DataMap> {
        self.contract_level.get(contract)
    }

    fn get_function(&self, contract: &str, function: &str) -> Option<&DataMap> {
        let key = (contract.to_string(), function.to_string());
        self.fn_level.get(&key)
    }
}

/// [`figment::Provider`] for [`InlineConfig`] at a given contract and function level.
///
/// Created by [`InlineConfig::provide`].
#[derive(Clone, Debug)]
pub struct InlineConfigProvider<'a> {
    inline: &'a InlineConfig,
    contract: &'a str,
    function: &'a str,
}

impl Provider for InlineConfigProvider<'_> {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("inline config")
    }

    fn data(&self) -> figment::Result<DataMap> {
        let mut map = DataMap::new();
        if let Some(new) = self.inline.get_contract(self.contract) {
            extend_data_map(&mut map, new);
        }
        if let Some(new) = self.inline.get_function(self.contract, self.function) {
            extend_data_map(&mut map, new);
        }
        Ok(map)
    }
}

fn extend_data_map(map: &mut DataMap, new: &DataMap) {
    for (profile, data) in new {
        extend_dict(map.entry(profile.clone()).or_default(), data);
    }
}

fn extend_dict(dict: &mut Dict, new: &Dict) {
    for (k, v) in new {
        match dict.entry(k.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(v.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                extend_value(entry.into_mut(), v);
            }
        }
    }
}

fn extend_value(value: &mut Value, new: &Value) {
    match (value, new) {
        (Value::Dict(tag, dict), Value::Dict(new_tag, new_dict)) => {
            *tag = *new_tag;
            extend_dict(dict, new_dict);
        }
        (value, new) => *value = new.clone(),
    }
}
