//! Configuration for fuzz testing

use std::error::Error;

use ethers_core::types::U256;
use serde::{Deserialize, Serialize};

use crate::Config;

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

impl FuzzConfig {
    /// Parses a [`FuzzConfig`] from Solidity function comments. <br>
    /// This is intended to override general fuzzer configs for the current
    /// execution profile (see [`Config`]).
    ///
    /// An example of compatible Solidity comments
    ///
    /// ```solidity
    /// contract MyTest is Test {
    /// // forge-config: default.fuzz.runs = 100
    /// // forge-config: ci.fuzz.runs = 500
    /// function test_SimpleFuzzTest(uint256 x) public {...}
    ///
    /// // forge-config: default.fuzz.runs = 500
    /// // forge-config: ci.fuzz.runs = 10000
    /// function test_ImportantFuzzTest(uint256 x) public {...}
    /// }
    /// ```
    pub fn parse<S: AsRef<str>>(text: S) -> Result<Self, Box<dyn Error>> {
        let profile = Config::selected_profile().to_string();
        let prefix = format!("forge-config:{profile}.fuzz.");

        let mut conf = Self::default();

        // Get all lines containing a `forge-config:` prefix
        let lines = text
            .as_ref()
            .split('\n')
            .map(Self::remove_whitespaces)
            .filter(|l| l.starts_with(&prefix));

        // i.e. line = "forge-config:default.fuzz.runs=500"
        for line in lines {
            // i.e. pair = ["forge-config:default.fuzz.", "runs=500"]
            let pair = line.split(&prefix).collect::<Vec<&str>>();

            // i.e. assignment = "runs=500"
            if let Some(assignment) = pair.last() {
                // i.e. key_value = "['runs', '500']"
                let key_value = assignment.split('=').collect::<Vec<&str>>();
                if let Some(key) = key_value.first() {
                    if let Some(value) = key_value.last() {
                        match key.to_owned() {
                            "runs" => conf.runs = value.parse()?,
                            "max-test-rejects" => conf.max_test_rejects = value.parse()?,
                            "seed" => conf.seed = Some(U256::zero()),
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(conf)
    }

    fn remove_whitespaces(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
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
    use crate::FuzzConfig;

    #[test]
    fn parse_config_default_profile() {
        let conf = "forge-config: default.fuzz.runs = 600 \n forge-config: ci.fuzz.runs = 500 ";
        let parsed = FuzzConfig::parse(conf).expect("Valid config");
        assert_eq!(parsed.runs, 600);
    }

    #[test]
    fn parse_config_white_spaces() {
        let conf = "forge-config:    default.fuzz.runs =     600   ";
        let parsed = FuzzConfig::parse(conf).expect("Valid config");
        assert_eq!(parsed.runs, 600);
    }

    #[test]
    fn parse_config_noisy_text() {
        let conf = "Free text comment forge-config: default.fuzz.runs =     600 ";
        let parsed = FuzzConfig::parse(conf).expect("Valid config");
        let conf = FuzzConfig::default();
        assert_eq!(parsed, conf);
    }

    #[test]
    fn parse_config_error() {
        let conf = "forge-config:default.fuzz.runs = foo \n ";
        let parsed = FuzzConfig::parse(conf);
        assert!(parsed.is_err());
    }

    #[test]
    fn parse_config_ci_profile() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "ci");
            let conf = "forge-config: default.fuzz.runs = 500 \n forge-config: ci.fuzz.runs = 500 ";
            let parsed = FuzzConfig::parse(conf).expect("Valid config");
            assert_eq!(parsed.runs, 500);
            Ok(())
        });
    }
}
