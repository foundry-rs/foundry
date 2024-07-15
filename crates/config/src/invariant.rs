//! Configuration for invariant testing

use crate::{
    fuzz::FuzzDictionaryConfig,
    inline::{
        parse_config_bool, parse_config_u32, InlineConfigParser, InlineConfigParserError,
        INLINE_CONFIG_INVARIANT_KEY,
    },
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains for invariant testing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The maximum number of attempts to shrink the sequence
    pub shrink_run_limit: u32,
    /// The maximum number of rejects via `vm.assume` which can be encountered during a single
    /// invariant run.
    pub max_assume_rejects: u32,
    /// Number of runs to execute and include in the gas report.
    pub gas_report_samples: u32,
    /// Path where invariant failures are recorded and replayed.
    pub failure_persist_dir: Option<PathBuf>,
}

impl Default for InvariantConfig {
    fn default() -> Self {
        Self {
            runs: 256,
            depth: 500,
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
            shrink_run_limit: 5000,
            max_assume_rejects: 65536,
            gas_report_samples: 256,
            failure_persist_dir: None,
        }
    }
}

impl InvariantConfig {
    /// Creates invariant configuration to write failures in `{PROJECT_ROOT}/cache/fuzz` dir.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            runs: 256,
            depth: 500,
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
            shrink_run_limit: 5000,
            max_assume_rejects: 65536,
            gas_report_samples: 256,
            failure_persist_dir: Some(cache_dir),
        }
    }

    /// Returns path to failure dir of given invariant test contract.
    pub fn failure_dir(self, contract_name: &str) -> PathBuf {
        self.failure_persist_dir
            .unwrap()
            .join("failures")
            .join(contract_name.split(':').last().unwrap())
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

        let mut conf_clone = self.clone();

        for pair in overrides {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "runs" => conf_clone.runs = parse_config_u32(key, value)?,
                "depth" => conf_clone.depth = parse_config_u32(key, value)?,
                "fail-on-revert" => conf_clone.fail_on_revert = parse_config_bool(key, value)?,
                "call-override" => conf_clone.call_override = parse_config_bool(key, value)?,
                "failure-persist-dir" => {
                    conf_clone.failure_persist_dir = Some(PathBuf::from(value))
                }
                "shrink-run-limit" => conf_clone.shrink_run_limit = parse_config_u32(key, value)?,
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
            unreachable!()
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
