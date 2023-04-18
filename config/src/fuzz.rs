//! Configuration for fuzz testing

use ethers_core::types::U256;
use serde::{Deserialize, Serialize};

use crate::{
    inline::{ConfParser, ConfParserError},
    Config,
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
    #[serde(
        deserialize_with = "ethers_core::types::serde_helpers::deserialize_stringified_numeric_opt"
    )]
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

impl ConfParser for FuzzConfig {
    fn config_prefix() -> String {
        let profile = Config::selected_profile().to_string();
        format!("forge-config:{profile}.fuzz.")
    }

    fn try_merge<S: AsRef<str>>(&self, text: S) -> Result<Self, ConfParserError>
    where
        Self: Sized + 'static,
    {
        let vars: Vec<(String, String)> = Self::config_variables::<S>(text);
        if vars.is_empty() {
            return Ok(*self)
        }

        let mut conf = *self;

        for pair in vars {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "runs" => conf.runs = value.parse()?,
                "max-test-rejects" => conf.max_test_rejects = value.parse()?,
                _ => Err(ConfParserError::InvalidConfigProperty(key.to_string()))?,
            }
        }
        Ok(conf)
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
    use crate::{inline::ConfParser, FuzzConfig};
    use solang_parser::pt::Comment;

    #[test]
    fn parse_config_default_profile() -> eyre::Result<()> {
        let conf = "forge-config: default.fuzz.runs = 1024";
        let base_conf: FuzzConfig = FuzzConfig::default();
        let parsed: FuzzConfig = base_conf.try_merge(conf).expect("Valid config");
        assert_eq!(parsed.runs, 1024);
        Ok(())
    }

    #[test]
    fn parse_config_ci_profile() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "ci");
            let conf = r#"
                forge-config: default.fuzz.runs = 1024
                forge-config: ci.fuzz.runs = 2048"#;

            let base_conf: FuzzConfig = FuzzConfig::default();
            let parsed: FuzzConfig = base_conf.try_merge(conf).expect("Valid config");
            assert_eq!(parsed.runs, 2048);
            Ok(())
        });
    }

    #[test]
    fn unrecognized_property() {
        let conf = "forge-config: default.fuzz.unknownprop = 200";
        let base_config = FuzzConfig::default();
        if let Err(e) = base_config.try_merge(conf) {
            assert_eq!(e.to_string(), "'unknownprop' is not a valid config property");
        } else {
            assert!(false)
        }
    }

    #[test]
    fn e2e() -> eyre::Result<()> {
        use solang_parser::parse;
        let code = r#"
            contract FuzzTestContract {
                /**
                 * forge-config: default.fuzz.runs = 1023
                 * forge-config: default.fuzz.max-test-rejects = 521
                 */
                function testFuzz(string name) public returns (string) {
                    return name;
                }
            }
        "#;

        let (_, comments) = parse(code, 0).expect("Valid code");
        let comm = &comments[0];
        match comm {
            Comment::DocBlock(_, text) => {
                let base_config = FuzzConfig::default();
                let config: FuzzConfig = base_config.try_merge(text).expect("Valid config");
                assert_eq!(config.runs, 1023);
                assert_eq!(config.max_test_rejects, 521);
            }
            _ => {
                assert!(false); // Force test to fail
            }
        }
        Ok(())
    }
}
