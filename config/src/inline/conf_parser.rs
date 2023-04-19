use std::{num::ParseIntError, str::ParseBoolError};

use regex::Regex;

/// Errors returned by the [`ConfParser`] trait.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfParserError {
    /// An invalid configuration property has been provided.
    /// The property cannot be mapped to the configuration object
    #[error("'{0}' is not a valid config property")]
    InvalidConfigProperty(String),
    /// An error occurred while trying to parse an integer configuration value
    #[error(transparent)]
    ParseIntError(#[from] ParseIntError),
    /// An error occurred while trying to parse a boolean configuration value
    #[error(transparent)]
    ParseBoolError(#[from] ParseBoolError),
}

/// This trait is intended to parse configurations from
/// structured text. Foundry users can annotate Solidity test functions,
/// providing special configs just for the execution of a specific test.
///
/// An example:
///
/// ```solidity
/// contract MyTest is Test {
/// /// forge-config: default.fuzz.runs = 100
/// /// forge-config: ci.fuzz.runs = 500
/// function test_SimpleFuzzTest(uint256 x) public {...}
///
/// /// forge-config: default.fuzz.runs = 500
/// /// forge-config: ci.fuzz.runs = 10000
/// function test_ImportantFuzzTest(uint256 x) public {...}
/// }
/// ```
pub trait ConfParser {
    /// Returns a prefix that is common to all valid configuration lines.
    /// That helps the parser to extract correct values out of a text.
    fn config_prefix() -> String;

    /// Returns
    /// - `Some(Self)` in case some configurations are merged into self.
    /// - `None` in case there are no configurations that can be applied to self.
    /// - `Err(ConfParserError)` in case of wrong configuration.
    fn try_merge<S: AsRef<str>>(&self, text: S) -> Result<Option<Self>, ConfParserError>
    where
        Self: Sized + 'static;

    /// Given a configuration `text` returns all available pairs (key, value)
    /// matching the `config_prefix`
    fn config_variables<S: AsRef<str>>(text: S) -> Vec<(String, String)> {
        let mut result: Vec<(String, String)> = vec![];
        let prefix = Self::config_prefix();

        text.as_ref()
            .split('\n')
            .map(remove_whitespaces)
            .filter(|l| l.contains(&prefix))
            .map(|l| {
                let pattern = format!("^.*{prefix}");
                let re = Regex::new(&pattern).unwrap();
                re.replace(&l, "").to_string()
            })
            .for_each(|line| {
                let key_value = line.split('=').collect::<Vec<&str>>(); // i.e. "['runs', '500']"
                if let Some(key) = key_value.first() {
                    if let Some(value) = key_value.last() {
                        result.push((key.to_string(), value.to_string()));
                    }
                }
            });

        result
    }
}

fn remove_whitespaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::{ConfParser, ConfParserError};

    #[test]
    fn config_variables() {
        let text = r#"
        forge-config: default.fuzz.runs = 600
        forge-config: default.fuzz.foo = 700
        forge-config: default.fuzz.bar = 800
        invalid-prefix
        "#;

        let vars = TestParser::config_variables(text);
        assert_eq!(
            vec![
                ("runs".to_string(), "600".to_string()),
                ("foo".to_string(), "700".to_string()),
                ("bar".to_string(), "800".to_string())
            ],
            vars
        );
    }

    #[test]
    fn white_spaces_are_ignored() {
        let text = "forge-config:        default.     fuzz.runs   = 600";
        let vars = TestParser::config_variables(text);
        assert_eq!(vec![("runs".to_string(), "600".to_string())], vars);
    }

    #[derive(Default)]
    struct TestParser;
    impl ConfParser for TestParser {
        fn config_prefix() -> String {
            "forge-config:default.fuzz.".to_string()
        }

        fn try_merge<S: AsRef<str>>(&self, _text: S) -> Result<Option<Self>, ConfParserError>
        where
            Self: Sized + 'static,
        {
            Ok(Some(Self::default()))
        }
    }
}
