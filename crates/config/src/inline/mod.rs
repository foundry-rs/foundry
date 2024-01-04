use crate::Config;
pub use conf_parser::{parse_config_bool, parse_config_u32, validate_profiles, InlineConfigParser};
pub use error::{InlineConfigError, InlineConfigParserError};
pub use natspec::NatSpec;
use once_cell::sync::Lazy;
use std::{borrow::Cow, collections::HashMap};

mod conf_parser;
mod error;
mod natspec;

pub const INLINE_CONFIG_FUZZ_KEY: &str = "fuzz";
pub const INLINE_CONFIG_INVARIANT_KEY: &str = "invariant";
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
    configs: HashMap<InlineConfigKey<'static>, T>,
}

impl<T> InlineConfig<T> {
    /// Returns an inline configuration, if any, for a test function.
    /// Configuration is identified by the pair "contract", "function".
    pub fn get<C, F>(&self, contract_id: C, fn_name: F) -> Option<&T>
    where
        C: Into<String>,
        F: Into<String>,
    {
        // TODO use borrow
        let key = InlineConfigKey {
            contract: Cow::Owned(contract_id.into()),
            function: Cow::Owned(fn_name.into()),
        };
        self.configs.get(&key)
    }

    /// Inserts an inline configuration, for a test function.
    /// Configuration is identified by the pair "contract", "function".    
    pub fn insert<C, F>(&mut self, contract_id: C, fn_name: F, config: T)
    where
        C: Into<String>,
        F: Into<String>,
    {
        let key = InlineConfigKey {
            contract: Cow::Owned(contract_id.into()),
            function: Cow::Owned(fn_name.into()),
        };
        self.configs.insert(key, config);
    }
}

/// Represents a (test-contract, test-function) pair
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineConfigKey<'a> {
    contract: Cow<'a, str>,
    function: Cow<'a, str>,
}

pub(crate) fn remove_whitespaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
