use regex::Regex;

use crate::{InlineConfigError, NatSpec};

use super::{remove_whitespaces, INLINE_CONFIG_PREFIX};

/// Errors returned by the [`InlineConfigParser`] trait.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InlineConfigParserError {
    /// An invalid configuration property has been provided.
    /// The property cannot be mapped to the configuration object
    #[error("'{0}' is an invalid config property")]
    InvalidConfigProperty(String),
    /// An invalid profile has been provided
    #[error("'{0}' specifies an invalid profile. Available profiles are: {1}")]
    InvalidProfile(String, String),
    /// An error occurred while trying to parse an integer configuration value
    #[error("Invalid config value for key '{0}'. Unable to parse '{1}' into an integer value")]
    ParseInt(String, String),
    /// An error occurred while trying to parse a boolean configuration value
    #[error("Invalid config value for key '{0}'. Unable to parse '{1}' into a boolean value")]
    ParseBool(String, String),
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
pub trait InlineConfigParser
where
    Self: Clone + Default + Sized + 'static,
{
    /// Returns a config key that is common to all valid configuration lines
    /// for the current impl. This helps to extract correct values out of a text.
    ///
    /// An example key would be `fuzz` of `invariant`.
    fn config_key() -> String;

    /// Tries to override `self` properties with values specified in the `configs` parameter.
    ///
    /// Returns
    /// - `Some(Self)` in case some configurations are merged into self.
    /// - `None` in case there are no configurations that can be applied to self.
    /// - `Err(InlineConfigParserError)` in case of wrong configuration.
    fn try_merge(&self, configs: &[String]) -> Result<Option<Self>, InlineConfigParserError>;

    /// Validates all configurations contained in a natspec that apply
    /// to the current configuration key.
    ///
    /// i.e. Given the `invariant` config key and a natspec comment of the form,
    /// ```solidity
    /// /// forge-config: default.invariant.runs = 500
    /// /// forge-config: default.invariant.depth = 500
    /// /// forge-config: ci.invariant.depth = 500
    /// /// forge-config: ci.fuzz.runs = 10
    /// ```
    /// would validate the whole `invariant` configuration.
    fn validate_configs(natspec: &NatSpec) -> Result<(), InlineConfigError> {
        let config_key = Self::config_key();

        let configs =
            natspec.config_lines().filter(|l| l.contains(&config_key)).collect::<Vec<String>>();

        Self::default().try_merge(&configs).map_err(|e| {
            let line = natspec.debug_context();
            InlineConfigError { line, source: e }
        })?;

        Ok(())
    }

    /// Given a list of `config_lines, returns all available pairs (key, value)
    /// matching the current config key
    ///
    /// i.e. Given the `invariant` config key and a vector of config lines
    /// ```rust
    /// let _config_lines = vec![
    ///     "forge-config: default.invariant.runs = 500",
    ///     "forge-config: default.invariant.depth = 500",
    ///     "forge-config: ci.invariant.depth = 500",
    ///     "forge-config: ci.fuzz.runs = 10"
    /// ];
    /// ```
    /// would return the whole set of `invariant` configs.
    /// ```rust
    ///  let _result = vec![
    ///     ("runs", "500"),
    ///     ("depth", "500"),
    ///     ("depth", "500"),
    ///  ];
    /// ```
    fn get_config_overrides(config_lines: &[String]) -> Vec<(String, String)> {
        let mut result: Vec<(String, String)> = vec![];
        let config_key = Self::config_key();
        let profile = ".*";
        let prefix = format!("^{INLINE_CONFIG_PREFIX}:{profile}{config_key}\\.");
        let re = Regex::new(&prefix).unwrap();

        config_lines
            .iter()
            .map(|l| remove_whitespaces(l))
            .filter(|l| re.is_match(l))
            .map(|l| re.replace(&l, "").to_string())
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

/// Checks if all configuration lines specified in `natspec` use a valid profile.
///
/// i.e. Given available profiles
/// ```rust
/// let _profiles = vec!["ci", "default"];
/// ```
/// A configuration like `forge-config: ciii.invariant.depth = 1` would result
/// in an error.
pub fn validate_profiles(natspec: &NatSpec, profiles: &[String]) -> Result<(), InlineConfigError> {
    for config in natspec.config_lines() {
        if !profiles.iter().any(|p| config.starts_with(&format!("{INLINE_CONFIG_PREFIX}:{p}."))) {
            let err_line: String = natspec.debug_context();
            let profiles = format!("{profiles:?}");
            Err(InlineConfigError {
                source: InlineConfigParserError::InvalidProfile(config, profiles),
                line: err_line,
            })?
        }
    }
    Ok(())
}

/// Tries to parse a `u32` from `value`. The `key` argument is used to give details
/// in the case of an error.
pub fn parse_config_u32(key: String, value: String) -> Result<u32, InlineConfigParserError> {
    value.parse().map_err(|_| InlineConfigParserError::ParseInt(key, value))
}

/// Tries to parse a `bool` from `value`. The `key` argument is used to give details
/// in the case of an error.
pub fn parse_config_bool(key: String, value: String) -> Result<bool, InlineConfigParserError> {
    value.parse().map_err(|_| InlineConfigParserError::ParseBool(key, value))
}

#[cfg(test)]
mod tests {
    use crate::{inline::conf_parser::validate_profiles, NatSpec};

    #[test]
    fn can_reject_invalid_profiles() {
        let profiles = ["ci".to_string(), "default".to_string()];
        let natspec = NatSpec {
            contract: Default::default(),
            function: Default::default(),
            line: Default::default(),
            docs: r"
            forge-config: ciii.invariant.depth = 1 
            forge-config: default.invariant.depth = 1
            "
            .into(),
        };

        let result = validate_profiles(&natspec, &profiles);
        assert!(result.is_err());
    }

    #[test]
    fn can_accept_valid_profiles() {
        let profiles = ["ci".to_string(), "default".to_string()];
        let natspec = NatSpec {
            contract: Default::default(),
            function: Default::default(),
            line: Default::default(),
            docs: r"
            forge-config: ci.invariant.depth = 1 
            forge-config: default.invariant.depth = 1
            "
            .into(),
        };

        let result = validate_profiles(&natspec, &profiles);
        assert!(result.is_ok());
    }
}
