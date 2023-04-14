//! Configuration for invariant testing

use crate::{
    conf_parser::{ConfParser, ConfParserError},
    fuzz::FuzzDictionaryConfig,
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

impl ConfParser for InvariantConfig {
    fn config_prefix() -> String {
        let profile = Config::selected_profile().to_string();
        format!("forge-config:{profile}.invariant.")
    }

    fn parse<S: AsRef<str>>(text: S) -> Result<Option<Self>, ConfParserError>
    where
        Self: Sized + 'static,
    {
        let vars: Vec<(String, String)> = Self::config_variables::<S>(text);
        if vars.is_empty() {
            return Ok(None)
        }

        let mut conf = Self::default();
        for pair in vars {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "runs" => conf.runs = value.parse()?,
                "depth" => conf.depth = value.parse()?,
                "fail-on-revert" => conf.fail_on_revert = value.parse()?,
                "call-override" => conf.call_override = value.parse()?,
                _ => Err(ConfParserError::InvalidConfigProperty(key.to_string()))?,
            }
        }
        Ok(Some(conf))
    }
}

#[cfg(test)]
mod tests {
    use crate::{conf_parser::ConfParser, InvariantConfig};

    #[test]
    fn parse_config_default_profile() -> eyre::Result<()> {
        let conf = r#"
            forge-config: default.invariant.runs = 1024
            forge-config: default.invariant.depth = 30
            forge-config: default.invariant.fail-on-revert = true
            forge-config: default.invariant.call-override = false
        "#;

        let parsed = InvariantConfig::parse(conf)?.expect("Parsed config exists");
        assert_eq!(parsed.runs, 1024);
        assert_eq!(parsed.depth, 30);
        assert_eq!(parsed.fail_on_revert, true);
        assert_eq!(parsed.call_override, false);

        Ok(())
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

            let parsed = InvariantConfig::parse(conf).unwrap().expect("Parsed config exists");
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
        if let Err(e) = InvariantConfig::parse(conf) {
            assert_eq!(e.to_string(), "'unknownprop' is not a valid config property");
        } else {
            assert!(false)
        }
    }
}
