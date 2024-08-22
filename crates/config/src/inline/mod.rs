use crate::Config;
use std::{collections::HashMap, sync::LazyLock};

mod conf_parser;
pub use conf_parser::*;

mod error;
pub use error::*;

mod natspec;
pub use natspec::*;

pub const INLINE_CONFIG_FUZZ_KEY: &str = "fuzz";
pub const INLINE_CONFIG_INVARIANT_KEY: &str = "invariant";
const INLINE_CONFIG_PREFIX: &str = "forge-config";

static INLINE_CONFIG_PREFIX_SELECTED_PROFILE: LazyLock<String> = LazyLock::new(|| {
    let selected_profile = Config::selected_profile().to_string();
    format!("{INLINE_CONFIG_PREFIX}:{selected_profile}.")
});

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to a solidity test.
#[derive(Clone, Debug, Default)]
pub struct InlineConfig<T> {
    /// Contract-level configurations, used for functions that do not have a specific
    /// configuration.
    contract_level: HashMap<String, T>,
    /// Maps a (test-contract, test-function) pair
    /// to a specific configuration provided by the user.
    fn_level: HashMap<(String, String), T>,
}

impl<T> InlineConfig<T> {
    /// Returns an inline configuration, if any, for a test function.
    /// Configuration is identified by the pair "contract", "function".
    pub fn get(&self, contract_id: &str, fn_name: &str) -> Option<&T> {
        let key = (contract_id.to_string(), fn_name.to_string());
        self.fn_level.get(&key).or_else(|| self.contract_level.get(contract_id))
    }

    pub fn insert_contract(&mut self, contract_id: impl Into<String>, config: T) {
        self.contract_level.insert(contract_id.into(), config);
    }

    /// Inserts an inline configuration, for a test function.
    /// Configuration is identified by the pair "contract", "function".
    pub fn insert_fn<C, F>(&mut self, contract_id: C, fn_name: F, config: T)
    where
        C: Into<String>,
        F: Into<String>,
    {
        let key = (contract_id.into(), fn_name.into());
        self.fn_level.insert(key, config);
    }
}

pub(crate) fn remove_whitespaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
