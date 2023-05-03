mod conf_parser;
pub use conf_parser::{InlineConfigParser, InlineConfigParserError};
use std::collections::HashMap;

mod natspec;
pub use natspec::NatSpec;

const INLINE_CONFIG_PREFIX: &str = "forge-config";
pub const INLINE_CONFIG_FUZZ_KEY: &str = "fuzz";
pub const INLINE_CONFIG_INVARIANT_KEY: &str = "invariant";

/// Wrapper error struct that catches config parsing
/// errors [`InlineConfigParserError`], enriching them with context information
/// reporting the misconfigured line.
#[derive(thiserror::Error, Debug)]
#[error("Inline config Error detected at {line} {source}")]
pub struct InlineConfigError {
    /// Specifies the misconfigured line. This is something of the form
    /// `dir/TestContract.t.sol:FuzzContract:10:12:111`
    pub line: String,
    /// The inner error
    pub source: InlineConfigParserError,
}

/// Represents a (test-contract, test-function) pair
type InlineConfigKey = (String, String);

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to a solidity test.
#[derive(Default, Debug, Clone)]
pub struct InlineConfig<T: 'static> {
    /// Maps a (test-contract, test-function) pair
    /// to a specific configuration provided by the user.
    configs: HashMap<InlineConfigKey, T>,
}

impl<T> InlineConfig<T> {
    /// Returns an inline configuration, if any, for a test function.
    /// Configuration is identified by the pair "contract", "function".
    pub fn get<S: Into<String>>(&self, contract_id: S, fn_name: S) -> Option<&T> {
        self.configs.get(&(contract_id.into(), fn_name.into()))
    }

    /// Inserts an inline configuration, for a test function.
    /// Configuration is identified by the pair "contract", "function".    
    pub fn insert<S: Into<String>>(&mut self, contract_id: S, fn_name: S, config: T) {
        self.configs.insert((contract_id.into(), fn_name.into()), config);
    }
}

fn remove_whitespaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::InlineConfigParserError;
    use crate::InlineConfigError;

    #[test]
    fn inline_config_error() {
        let source =
            InlineConfigParserError::ParseBoolError("key".into(), "invalid-bool-value".into());
        let line = "dir/TestContract.t.sol:FuzzContract:10:12:111".to_string();
        let error = InlineConfigError { line: line.clone(), source: source.clone() };

        let expected = format!("Inline config Error detected at {line} {source}");
        assert_eq!(error.to_string(), expected);
    }
}
