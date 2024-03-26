use crate::Config;
pub use conf_parser::{
    parse_config_bool, parse_config_u32, validate_inline_config_type, InlineConfigParser,
    InlineConfigType,
};
pub use error::{InlineConfigError, InlineConfigParserError};
pub use natspec::NatSpec;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};

mod conf_parser;
mod error;
mod natspec;

pub const INLINE_CONFIG_FUZZ_KEY: &str = "fuzz";
pub const INLINE_CONFIG_INVARIANT_KEY: &str = "invariant";
pub const INLINE_CONFIG_FIXTURE_KEY: &str = "fixture";
const INLINE_CONFIG_PREFIX: &str = "forge-config";

static INLINE_CONFIG_PREFIX_SELECTED_PROFILE: Lazy<String> = Lazy::new(|| {
    let selected_profile = Config::selected_profile().to_string();
    format!("{INLINE_CONFIG_PREFIX}:{selected_profile}.")
});

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to a solidity test.
#[derive(Clone, Debug, Default)]
pub struct InlineConfig<T> {
    /// Maps a (test-contract, test-function) pair
    /// to a specific configuration provided by the user.
    configs: HashMap<(String, String), T>,
}

impl<T> InlineConfig<T> {
    /// Returns an inline configuration, if any, for a test function.
    /// Configuration is identified by the pair "contract", "function".
    pub fn get(&self, contract_id: &str, fn_name: &str) -> Option<&T> {
        let key = (contract_id.to_string(), fn_name.to_string());
        self.configs.get(&key)
    }

    /// Inserts an inline configuration, for a test function.
    /// Configuration is identified by the pair "contract", "function".    
    pub fn insert<C, F>(&mut self, contract_id: C, fn_name: F, config: T)
    where
        C: Into<String>,
        F: Into<String>,
    {
        let key = (contract_id.into(), fn_name.into());
        self.configs.insert(key, config);
    }
}

/// Represents per-test fixtures, declared inline
/// as structured comments in Solidity test files. This allows
/// setting data sets for specific fuzzed parameters in a solidity test.
#[derive(Clone, Debug, Default)]
pub struct InlineFixturesConfig {
    /// Maps a test-contract to a set of test-fixtures.
    configs: HashMap<String, HashSet<String>>,
}

impl InlineFixturesConfig {
    /// Records a function to be used as fixture for given contract.
    /// The name of function should be the same as the name of fuzzed parameter.
    pub fn add_fixture(&mut self, contract: String, fixture: String) {
        self.configs.entry(contract).or_default().insert(fixture);
    }

    /// Returns functions to be used as fixtures for given contract.
    pub fn get_fixtures(&mut self, contract: String) -> Option<&HashSet<String>> {
        self.configs.get(&contract)
    }
}

pub(crate) fn remove_whitespaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
