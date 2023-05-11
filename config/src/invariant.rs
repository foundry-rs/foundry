//! Configuration for invariant testing

use crate::{
    fuzz::FuzzDictionaryConfig,
    inline::{
        parse_config_bool, parse_config_u32, InlineConfigParser, InlineConfigParserError,
        INLINE_CONFIG_INVARIANT_KEY,
    },
};
use serde::{Deserialize, Serialize};

/// Contains for invariant testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantConfig {
    /// The number of runs that must execute for each invariant test group.
    pub runs: u32,
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Fails the invariant fuzzing if a revert occurs
    pub fail_on_revert: bool,
    /// Allows overriding an unsafe external call when running invariant tests. eg. reentrancy
    /// checks
    pub call_override: bool,
    /// The fuzz dictionary configuration
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
}

impl Default for InvariantConfig {
    fn default() -> Self {
        InvariantConfig {
            runs: 256,
            depth: 15,
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
        }
    }
}

impl InlineConfigParser for InvariantConfig {
    fn config_key() -> String {
        INLINE_CONFIG_INVARIANT_KEY.into()
    }

    fn try_merge(&self, configs: &[String]) -> Result<Option<Self>, InlineConfigParserError> {
        let overrides: Vec<(String, String)> = Self::get_config_overrides(configs);

        if overrides.is_empty() {
            return Ok(None)
        }

        // self is Copy. We clone it with dereference.
        let mut conf_clone = *self;

        for pair in overrides {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "runs" => conf_clone.runs = parse_config_u32(key, value)?,
                "depth" => conf_clone.depth = parse_config_u32(key, value)?,
                "fail-on-revert" => conf_clone.fail_on_revert = parse_config_bool(key, value)?,
                "call-override" => conf_clone.call_override = parse_config_bool(key, value)?,
                _ => Err(InlineConfigParserError::InvalidConfigProperty(key.to_string()))?,
            }
        }
        Ok(Some(conf_clone))
    }
}

#[cfg(test)]
mod tests {
    use crate::{inline::InlineConfigParser, InvariantConfig};

    #[test]
    fn unrecognized_property() {
        let configs = &["forge-config: default.invariant.unknownprop = 200".to_string()];
        let base_config = InvariantConfig::default();
        if let Err(e) = base_config.try_merge(configs) {
            assert_eq!(e.to_string(), "'unknownprop' is an invalid config property");
        } else {
            assert!(false)
        }
    }

    #[test]
    fn successful_merge() {
        let configs = &["forge-config: default.invariant.runs = 42424242".to_string()];
        let base_config = InvariantConfig::default();
        let merged: InvariantConfig = base_config.try_merge(configs).expect("No errors").unwrap();
        assert_eq!(merged.runs, 42424242);
    }

    #[test]
    fn merge_is_none() {
        let empty_config = &[];
        let base_config = InvariantConfig::default();
        let merged = base_config.try_merge(empty_config).expect("No errors");
        assert!(merged.is_none());
    }

    #[test]
    fn can_merge_unrelated_properties_into_config() {
        let unrelated_configs = &["forge-config: default.fuzz.runs = 2".to_string()];
        let base_config = InvariantConfig::default();
        let merged = base_config.try_merge(unrelated_configs).expect("No errors");
        assert!(merged.is_none());
    }

    #[test]
    fn override_detection() {
        let configs = &[
            "forge-config: default.fuzz.runs = 42424242".to_string(),
            "forge-config: ci.fuzz.runs = 666666".to_string(),
            "forge-config: default.invariant.runs = 2".to_string(),
        ];
        let variables = InvariantConfig::get_config_overrides(configs);
        assert_eq!(variables, vec![("runs".into(), "2".into())]);
    }
}
