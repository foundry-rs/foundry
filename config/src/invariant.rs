//! Configuration for invariant testing

use crate::{
    fuzz::FuzzDictionaryConfig,
    inline::{InlineConfigParser, InlineConfigParserError},
    Config,
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
    fn config_prefix() -> String {
        let profile = Config::selected_profile().to_string();
        format!("forge-config:{profile}.invariant.")
    }

    fn try_merge<S: AsRef<str>>(&self, text: S) -> Result<Option<Self>, InlineConfigParserError>
    where
        Self: Sized + 'static,
    {
        let vars: Vec<(String, String)> = Self::config_variables::<S>(text);
        if vars.is_empty() {
            return Ok(None)
        }

        let mut conf = *self;

        for pair in vars {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "runs" => {
                    conf.runs = value
                        .parse()
                        .map_err(|_| InlineConfigParserError::ParseIntError(key, value))?
                }
                "depth" => {
                    conf.depth = value
                        .parse()
                        .map_err(|_| InlineConfigParserError::ParseIntError(key, value))?
                }
                "fail-on-revert" => {
                    conf.fail_on_revert = value
                        .parse()
                        .map_err(|_| InlineConfigParserError::ParseBoolError(key, value))?
                }
                "call-override" => {
                    conf.call_override = value
                        .parse()
                        .map_err(|_| InlineConfigParserError::ParseBoolError(key, value))?
                }
                _ => Err(InlineConfigParserError::InvalidConfigProperty(key.to_string()))?,
            }
        }
        Ok(Some(conf))
    }
}

#[cfg(test)]
mod tests {
    use crate::{inline::InlineConfigParser, InvariantConfig};

    #[test]
    fn parse_config_default_profile() {
        let conf = r#"
            forge-config: default.invariant.runs = 1024
            forge-config: default.invariant.depth = 30
            forge-config: default.invariant.fail-on-revert = true
            forge-config: default.invariant.call-override = false
        "#;
        let base_conf: InvariantConfig = InvariantConfig::default();
        let parsed: InvariantConfig = base_conf.try_merge(conf).unwrap().expect("Valid config");
        assert_eq!(parsed.runs, 1024);
        assert_eq!(parsed.depth, 30);
        assert_eq!(parsed.fail_on_revert, true);
        assert_eq!(parsed.call_override, false);
    }

    #[test]
    fn parse_config_ci_profile() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "ci");
            let conf = r#"
                forge-config: ci.invariant.runs = 1024
                forge-config: ci.invariant.depth = 30
                forge-config: ci.invariant.fail-on-revert = true
                forge-config: ci.invariant.call-override = false
            "#;

            let base_conf: InvariantConfig = InvariantConfig::default();
            let parsed: InvariantConfig = base_conf.try_merge(conf).unwrap().expect("Valid config");
            assert_eq!(parsed.runs, 1024);
            assert_eq!(parsed.depth, 30);
            assert_eq!(parsed.fail_on_revert, true);
            assert_eq!(parsed.call_override, false);
            Ok(())
        });
    }

    #[test]
    fn unrecognized_property() {
        let conf = "forge-config: default.invariant.unknownprop = 200";
        let base_conf: InvariantConfig = InvariantConfig::default();
        if let Err(e) = base_conf.try_merge(conf) {
            assert_eq!(e.to_string(), "'unknownprop' is not a valid config property");
        } else {
            assert!(false)
        }
    }
}
