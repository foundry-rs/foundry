use std::collections::BTreeSet;

use crate::Config;
use alloy_primitives::map::HashMap;
use figment::{
    Figment, Profile, Provider,
    value::{Dict, Map, Value},
};
use foundry_compilers::ProjectCompileOutput;
use foundry_evm_networks::NetworkVariant;
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
    /// A legacy Halmos inline annotation could not be translated.
    #[error("invalid @custom:halmos annotation: {0}")]
    InvalidHalmosConfig(String),
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
        if let Some(data) = parse_config_values(natspec, natspec.halmos_config_values()?)? {
            extend_data_map(map, &data);
        }
        if let Some(data) = parse_config_values(natspec, natspec.config_values())? {
            extend_data_map(map, &data);
        }
        Ok(())
    }

    /// Returns a [`figment::Provider`] for this [`InlineConfig`] at the given contract and function
    /// level.
    pub const fn provide<'a>(
        &'a self,
        contract: &'a str,
        function: &'a str,
    ) -> InlineConfigProvider<'a> {
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

    /// Returns the configured [`NetworkVariant`] for a given test, checking function-level first
    /// then contract-level. Returns `None` if no network annotation is present.
    pub fn network_for(
        &self,
        profile: &Profile,
        contract: &str,
        function: &str,
    ) -> Option<NetworkVariant> {
        inline_value_for_profile(
            profile,
            &[self.get_function(contract, function), self.get_contract(contract)],
            |dict| {
                if let Some(Value::Dict(_, networks)) = dict.get("networks")
                    && let Some(Value::String(_, s)) = networks.get("network")
                {
                    return s.parse().ok();
                }
                None
            },
        )
    }

    /// Returns whether contract-level inline config enables symbolic execution.
    pub fn contract_symbolic_enabled(
        &self,
        profile: &Profile,
        contract: &str,
        default: bool,
    ) -> bool {
        inline_value_for_profile(profile, &[self.get_contract(contract)], |dict| {
            if let Some(Value::Dict(_, symbolic)) = dict.get("symbolic")
                && let Some(Value::Bool(_, enabled)) = symbolic.get("enabled")
            {
                return Some(*enabled);
            }
            None
        })
        .unwrap_or(default)
    }

    /// Returns all distinct [`NetworkVariant`]s referenced in any inline config annotation.
    ///
    /// This is used to determine whether a multi-network test pass is needed.
    pub fn referenced_override_networks(&self, profile: &Profile) -> Vec<NetworkVariant> {
        let mut seen = BTreeSet::new();
        for (contract, function) in self.fn_level.keys() {
            if let Some(v) = self.network_for(profile, contract, function) {
                seen.insert(v);
            }
        }
        for contract in self.contract_level.keys() {
            if let Some(v) = self.network_for(profile, contract, "") {
                seen.insert(v);
            }
        }
        seen.into_iter().collect()
    }

    fn get_contract(&self, contract: &str) -> Option<&DataMap> {
        self.contract_level.get(contract)
    }

    fn get_function(&self, contract: &str, function: &str) -> Option<&DataMap> {
        let key = (contract.to_string(), function.to_string());
        self.fn_level.get(&key)
    }
}

fn inline_value_for_profile<T>(
    profile: &Profile,
    levels: &[Option<&DataMap>],
    value_from_dict: impl Fn(&Dict) -> Option<T> + Copy,
) -> Option<T> {
    inline_value_for_exact_profile(profile, levels, value_from_dict).or_else(|| {
        (profile != Profile::Default)
            .then(|| inline_value_for_exact_profile(&Profile::Default, levels, value_from_dict))
            .flatten()
    })
}

fn inline_value_for_exact_profile<T>(
    profile: &Profile,
    levels: &[Option<&DataMap>],
    value_from_dict: impl Fn(&Dict) -> Option<T> + Copy,
) -> Option<T> {
    levels.iter().find_map(|data| data.and_then(|data| data.get(profile)).and_then(value_from_dict))
}

fn parse_config_values<'a>(
    natspec: &NatSpec,
    values: impl IntoIterator<Item = impl std::borrow::Borrow<str> + 'a>,
) -> Result<Option<DataMap>, InlineConfigError> {
    let joined = values
        .into_iter()
        .map(|s| {
            let s = s.borrow();
            // Replace `-` with `_` for backwards compatibility with the old parser.
            if let Some(idx) = s.find('=') {
                s[..idx].replace('-', "_") + &s[idx..]
            } else {
                s.to_string()
            }
        })
        .format("\n")
        .to_string();
    if joined.is_empty() {
        return Ok(None);
    }
    let data = toml::from_str::<DataMap>(&joined).map_err(|e| InlineConfigError {
        location: natspec.location_string(),
        kind: InlineConfigErrorKind::Parse(e),
    })?;
    Ok(Some(data))
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

    fn natspec(docs: &str) -> NatSpec {
        NatSpec {
            contract: "test/Symbolic.t.sol:Symbolic".to_string(),
            function: Some("check".to_string()),
            line: "10:5".to_string(),
            docs: docs.to_string(),
        }
    }

    #[test]
    fn legacy_halmos_array_lengths_feed_symbolic_inline_config() {
        let mut inline = InlineConfig::new();
        inline
            .insert(&natspec(
                "@custom:halmos --array-lengths 2,4 --invariant-depth 12 --width 8 --depth 99",
            ))
            .unwrap();

        let config = Config::default()
            .merge_inline_provider(inline.provide("test/Symbolic.t.sol:Symbolic", "check"))
            .unwrap();

        assert_eq!(config.symbolic.array_lengths, vec![2, 4]);
        assert_eq!(config.symbolic.invariant_depth, 12);
        assert_eq!(config.symbolic.width, Some(8));
        assert_eq!(config.symbolic.depth, Some(99));
    }

    #[test]
    fn legacy_halmos_named_and_default_lengths_feed_symbolic_inline_config() {
        let mut inline = InlineConfig::new();
        inline
            .insert(&natspec(
                "@custom:halmos --array-lengths values={2,4},data=8 --default-array-lengths 0,1 --default-bytes-lengths 0,65",
            ))
            .unwrap();

        let config = Config::default()
            .merge_inline_provider(inline.provide("test/Symbolic.t.sol:Symbolic", "check"))
            .unwrap();

        assert_eq!(
            config.symbolic.dynamic_lengths,
            std::collections::BTreeMap::from([
                ("data".to_string(), vec![8]),
                ("values".to_string(), vec![2, 4]),
            ])
        );
        assert_eq!(config.symbolic.default_array_lengths, vec![0, 1]);
        assert_eq!(config.symbolic.default_bytes_lengths, vec![0, 65]);
    }

    #[test]
    fn native_symbolic_inline_config_overrides_legacy_halmos_translation() {
        let mut inline = InlineConfig::new();
        inline
            .insert(&natspec(
                r#"
@custom:halmos --array-lengths 2
forge-config: default.symbolic.array_lengths = [3]
forge-config: default.symbolic.default_dynamic_length = 4
"#,
            ))
            .unwrap();

        let config = Config::default()
            .merge_inline_provider(inline.provide("test/Symbolic.t.sol:Symbolic", "check"))
            .unwrap();

        assert_eq!(config.symbolic.array_lengths, vec![3]);
        assert_eq!(config.symbolic.default_dynamic_length, 4);
    }

    #[test]
    fn contract_symbolic_enabled_reads_contract_inline_config() {
        let mut inline = InlineConfig::new();
        inline
            .insert(&NatSpec {
                contract: "test/Symbolic.t.sol:Symbolic".to_string(),
                function: None,
                line: "10:5".to_string(),
                docs: r#"
forge-config: default.symbolic.enabled = true
forge-config: ci.symbolic.enabled = false
"#
                .to_string(),
            })
            .unwrap();

        assert!(inline.contract_symbolic_enabled(
            &Profile::new("default"),
            "test/Symbolic.t.sol:Symbolic",
            false,
        ));
        assert!(!inline.contract_symbolic_enabled(
            &Profile::new("ci"),
            "test/Symbolic.t.sol:Symbolic",
            true,
        ));
        assert!(inline.contract_symbolic_enabled(
            &Profile::new("nightly"),
            "test/Symbolic.t.sol:Symbolic",
            false,
        ));
        assert!(!inline.contract_symbolic_enabled(
            &Profile::new("default"),
            "test/Other.t.sol:Other",
            false,
        ));
    }

    #[test]
    fn network_for_preserves_profile_then_level_precedence() {
        let mut inline = InlineConfig::new();
        inline
            .insert(&NatSpec {
                contract: "test/Network.t.sol:Network".to_string(),
                function: None,
                line: "10:5".to_string(),
                docs: r#"
forge-config: default.networks.network = "optimism"
forge-config: ci.networks.network = "tempo"
"#
                .to_string(),
            })
            .unwrap();
        inline
            .insert(&NatSpec {
                contract: "test/Network.t.sol:Network".to_string(),
                function: Some("testNetwork".to_string()),
                line: "20:5".to_string(),
                docs: r#"forge-config: default.networks.network = "ethereum""#.to_string(),
            })
            .unwrap();

        assert_eq!(
            inline.network_for(
                &Profile::new("default"),
                "test/Network.t.sol:Network",
                "testNetwork"
            ),
            Some(NetworkVariant::Ethereum),
        );
        assert_eq!(
            inline.network_for(&Profile::new("ci"), "test/Network.t.sol:Network", "testNetwork"),
            Some(NetworkVariant::Tempo),
        );
    }
}
