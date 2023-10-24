//! Configuration for fuzz testing

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

use crate::inline::{
    parse_config_u32, InlineConfigParser, InlineConfigParserError, INLINE_CONFIG_FUZZ_KEY,
};

/// Contains for fuzz testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzConfig {
    /// The number of test cases that must execute for each property test
    pub runs: u32,
    /// The maximum number of test case rejections allowed by proptest, to be
    /// encountered during usage of `vm.assume` cheatcode. This will be used
    /// to set the `max_global_rejects` value in proptest test runner config.
    /// `max_local_rejects` option isn't exposed here since we're not using
    /// `prop_filter`.
    pub max_test_rejects: u32,
    /// Optional seed for the fuzzing RNG algorithm
    pub seed: Option<U256>,
    /// The fuzz dictionary configuration
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            runs: 256,
            max_test_rejects: 65536,
            seed: None,
            dictionary: FuzzDictionaryConfig::default(),
        }
    }
}

impl InlineConfigParser for FuzzConfig {
    fn config_key() -> String {
        INLINE_CONFIG_FUZZ_KEY.into()
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
                "max-test-rejects" => conf_clone.max_test_rejects = parse_config_u32(key, value)?,
                "dictionary-weight" => {
                    conf_clone.dictionary.dictionary_weight = parse_config_u32(key, value)?
                }
                _ => Err(InlineConfigParserError::InvalidConfigProperty(key))?,
            }
        }
        Ok(Some(conf_clone))
    }
}

/// Contains for fuzz testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzDictionaryConfig {
    /// The weight of the dictionary
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub dictionary_weight: u32,
    /// The flag indicating whether to include values from storage
    pub include_storage: bool,
    /// The flag indicating whether to include push bytes values
    pub include_push_bytes: bool,
    /// How many addresses to record at most.
    /// Once the fuzzer exceeds this limit, it will start evicting random entries
    ///
    /// This limit is put in place to prevent memory blowup.
    #[serde(deserialize_with = "crate::deserialize_usize_or_max")]
    pub max_fuzz_dictionary_addresses: usize,
    /// How many values to record at most.
    /// Once the fuzzer exceeds this limit, it will start evicting random entries
    #[serde(deserialize_with = "crate::deserialize_usize_or_max")]
    pub max_fuzz_dictionary_values: usize,
}

impl Default for FuzzDictionaryConfig {
    fn default() -> Self {
        FuzzDictionaryConfig {
            dictionary_weight: 40,
            include_storage: true,
            include_push_bytes: true,
            // limit this to 300MB
            max_fuzz_dictionary_addresses: (300 * 1024 * 1024) / 20,
            // limit this to 200MB
            max_fuzz_dictionary_values: (200 * 1024 * 1024) / 32,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{inline::InlineConfigParser, FuzzConfig};

    #[test]
    fn unrecognized_property() {
        let configs = &["forge-config: default.fuzz.unknownprop = 200".to_string()];
        let base_config = FuzzConfig::default();
        if let Err(e) = base_config.try_merge(configs) {
            assert_eq!(e.to_string(), "'unknownprop' is an invalid config property");
        } else {
            unreachable!()
        }
    }

    #[test]
    fn successful_merge() {
        let configs = &[
            "forge-config: default.fuzz.runs = 42424242".to_string(),
            "forge-config: default.fuzz.dictionary-weight = 42".to_string(),
        ];
        let base_config = FuzzConfig::default();
        let merged: FuzzConfig = base_config.try_merge(configs).expect("No errors").unwrap();
        assert_eq!(merged.runs, 42424242);
        assert_eq!(merged.dictionary.dictionary_weight, 42);
    }

    #[test]
    fn merge_is_none() {
        let empty_config = &[];
        let base_config = FuzzConfig::default();
        let merged = base_config.try_merge(empty_config).expect("No errors");
        assert!(merged.is_none());
    }

    #[test]
    fn merge_is_none_unrelated_property() {
        let unrelated_configs = &["forge-config: default.invariant.runs = 2".to_string()];
        let base_config = FuzzConfig::default();
        let merged = base_config.try_merge(unrelated_configs).expect("No errors");
        assert!(merged.is_none());
    }

    #[test]
    fn override_detection() {
        let configs = &[
            "forge-config: default.fuzz.runs = 42424242".to_string(),
            "forge-config: ci.fuzz.runs = 666666".to_string(),
            "forge-config: default.invariant.runs = 2".to_string(),
            "forge-config: default.fuzz.dictionary-weight = 42".to_string(),
        ];
        let variables = FuzzConfig::get_config_overrides(configs);
        assert_eq!(
            variables,
            vec![
                ("runs".into(), "42424242".into()),
                ("runs".into(), "666666".into()),
                ("dictionary-weight".into(), "42".into())
            ]
        );
    }
}
