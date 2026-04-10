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

/// A compiler-agnostic inline configuration entry.
///
/// This type mirrors `foundry_compilers::InlineConfigEntry` and serves as the
/// bridge between compiler-provided config overrides and Foundry's internal
/// NatSpec-based `InlineConfig`.
#[derive(Clone, Debug)]
pub struct InlineConfigEntry {
    /// The contract identifier, in the form `path:ContractName`.
    pub contract: String,
    /// The function name, if this is a function-level override.
    pub function: Option<String>,
    /// The location in source for error reporting, e.g. `"10:5"`.
    pub line: String,
    /// Raw configuration lines. Each string must include the `forge-config:` prefix,
    /// e.g. `"forge-config: default.fuzz.runs = 1024"`.
    pub config_values: Vec<String>,
}

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

    /// Creates a new [`InlineConfig`] from pre-parsed [`NatSpec`] entries.
    ///
    /// This allows alternative compilers to provide inline config without going
    /// through solc/solar AST parsing.
    pub fn from_natspecs(natspecs: &[NatSpec], profiles: &[Profile]) -> eyre::Result<Self> {
        let mut inline = Self::new();
        for natspec in natspecs {
            inline.insert(natspec)?;
            natspec.validate_profiles(profiles)?;
        }
        Ok(inline)
    }

    /// Creates a new [`InlineConfig`] from [`InlineConfigEntry`] items.
    ///
    /// This bridges the compiler-agnostic [`InlineConfigEntry`] type to
    /// Foundry's internal [`NatSpec`]-based inline config, enabling non-Solidity
    /// compilers to provide per-test configuration overrides.
    pub fn from_entries(
        entries: impl IntoIterator<Item = InlineConfigEntry>,
        profiles: &[Profile],
    ) -> eyre::Result<Self> {
        let natspecs: Vec<NatSpec> = entries
            .into_iter()
            .map(|entry| NatSpec {
                contract: entry.contract,
                function: entry.function,
                line: entry.line,
                docs: entry.config_values.join("\n"),
            })
            .collect();
        Self::from_natspecs(&natspecs, profiles)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_entries_empty() {
        let config = InlineConfig::from_entries(vec![], &[Profile::Default]).unwrap();
        assert!(!config.contains_contract("Foo"));
    }

    #[test]
    fn test_from_entries_function_level() {
        let entry = InlineConfigEntry {
            contract: "src/Test.sol:TestContract".to_string(),
            function: Some("testFoo".to_string()),
            line: "10:5".to_string(),
            config_values: vec!["forge-config: default.fuzz.runs = 512".to_string()],
        };
        let config = InlineConfig::from_entries(vec![entry], &[Profile::Default]).unwrap();
        assert!(config.contains_function("src/Test.sol:TestContract", "testFoo"));
    }

    #[test]
    fn test_from_entries_contract_level() {
        let entry = InlineConfigEntry {
            contract: "src/Test.sol:TestContract".to_string(),
            function: None,
            line: "5:1".to_string(),
            config_values: vec!["forge-config: default.fuzz.runs = 256".to_string()],
        };
        let config = InlineConfig::from_entries(vec![entry], &[Profile::Default]).unwrap();
        assert!(config.contains_contract("src/Test.sol:TestContract"));
    }

    #[test]
    fn test_from_entries_invalid_profile() {
        let entry = InlineConfigEntry {
            contract: "src/Test.sol:TestContract".to_string(),
            function: Some("testBar".to_string()),
            line: "10:5".to_string(),
            config_values: vec!["forge-config: nonexistent.fuzz.runs = 100".to_string()],
        };
        let result = InlineConfig::from_entries(vec![entry], &[Profile::Default]);
        assert!(result.is_err(), "Expected error for invalid profile");
    }
}
