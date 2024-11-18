use std::str::FromStr;

use foundry_compilers::artifacts::EvmVersion;

use crate::{inline::InlineConfigParserError, InlineConfigParser};

/// Test configuration
///
/// Used to parse InlineConfig.
#[derive(Clone, Debug, Default)]
pub struct TestConfig {
    pub evm_version: EvmVersion,
}

impl InlineConfigParser for TestConfig {
    fn config_key() -> String {
        "test".into()
    }

    fn try_merge(
        &self,
        configs: &[String],
    ) -> Result<Option<Self>, crate::inline::InlineConfigParserError> {
        let overrides: Vec<(String, String)> = Self::get_config_overrides(configs);

        if overrides.is_empty() {
            return Ok(None)
        }

        let mut conf_clone = self.clone();

        for pair in overrides {
            let key = pair.0;
            let value = pair.1;
            match key.as_str() {
                "evm-version" => {
                    conf_clone.evm_version = EvmVersion::from_str(value.as_str()).map_err(|_| {
                        InlineConfigParserError::InvalidConfigProperty(format!(
                            "evm-version {}",
                            value
                        ))
                    })?
                }
                _ => Err(InlineConfigParserError::InvalidConfigProperty(key))?,
            }
        }
        Ok(Some(conf_clone))
    }
}
