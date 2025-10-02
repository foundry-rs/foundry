use crate::Config;
use alloy_primitives::map::HashMap;
use figment::{
    Figment, Profile, Provider,
    value::{Dict, Map, Value},
};
use foundry_compilers::ProjectCompileOutput;
use itertools::Itertools;

mod natspec;
pub use natspec::*;

const INLINE_CONFIG_PREFIX: &str = "forge-config:";

type DataMap = Map<Profile, Dict>;

/// Errors returned when parsing inline config.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InlineConfigErrorKind {
    /// Failed to parse inline config as TOML.
    #[error(transparent)]
    Parse(#[from] toml::de::Error),
    /// An invalid profile has been provided.
    #[error("invalid profile `{0}`; valid profiles: {1}")]
    InvalidProfile(String, String),
}

/// Wrapper error struct that catches config parsing errors, enriching them with context information
/// reporting the misconfigured line.
#[derive(Debug, thiserror::Error)]
#[error("Inline config error at {location}: {kind}")]
pub struct InlineConfigError {
    /// The span of the error in the format:
    /// `dir/TestContract.t.sol:FuzzContract:10:12:111`
    pub location: String,
    /// The inner error
    pub kind: InlineConfigErrorKind,
}

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

    /// Tries to create a new instance by detecting inline configurations from the project compile
    /// output.
    pub fn new_parsed(output: &ProjectCompileOutput, config: &Config) -> eyre::Result<Self> {
        let natspecs: Vec<NatSpec> = NatSpec::parse(output, &config.root);
        let profiles = &config.profiles;
        let mut inline = Self::new();
        for natspec in &natspecs {
            inline.insert(natspec)?;
            // Validate after parsing as TOML.
            natspec.validate_profiles(profiles)?;
        }
        Ok(inline)
    }

    /// Inserts a new [`NatSpec`] into the [`InlineConfig`].
    pub fn insert(&mut self, natspec: &NatSpec) -> Result<(), InlineConfigError> {
        let map = if let Some(function) = &natspec.function {
            self.fn_level.entry((natspec.contract.clone(), function.clone())).or_default()
        } else {
            self.contract_level.entry(natspec.contract.clone()).or_default()
        };
        let joined = natspec
            .config_values()
            .map(|s| {
                // Replace `-` with `_` for backwards compatibility with the old parser.
                if let Some(idx) = s.find('=') {
                    s[..idx].replace('-', "_") + &s[idx..]
                } else {
                    s.to_string()
                }
            })
            .format("\n")
            .to_string();
        let data = toml::from_str::<DataMap>(&joined).map_err(|e| InlineConfigError {
            location: natspec.location_string(),
            kind: InlineConfigErrorKind::Parse(e),
        })?;
        extend_data_map(map, &data);
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

    /// Returns `true` if a configuration is present at the given contract level.
    pub fn contains_contract(&self, contract: &str) -> bool {
        self.get_contract(contract).is_some_and(|map| !map.is_empty())
    }

    /// Returns `true` if a configuration is present at the function level.
    ///
    /// Does not include contract-level configurations.
    pub fn contains_function(&self, contract: &str, function: &str) -> bool {
        self.get_function(contract, function).is_some_and(|map| !map.is_empty())
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
