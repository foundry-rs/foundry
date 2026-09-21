//! Fe compiler configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Settings for compiling Fe contracts with an installed Fe executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeConfig {
    /// Path to Fe 26.3.0 or newer; defaults to `fe` on PATH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Fe optimization level: `0`, `1`, `2`, or `s`.
    pub optimize: String,
}

impl Default for FeConfig {
    fn default() -> Self {
        Self { path: None, optimize: "1".into() }
    }
}

impl FeConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn defaults_and_overrides() {
        let defaults: FeConfig = toml::from_str("").unwrap();
        assert_eq!(defaults, FeConfig::default());
        let config: FeConfig = toml::from_str("path = '/opt/fe'\noptimize = 's'").unwrap();
        assert_eq!(config.path, Some(PathBuf::from("/opt/fe")));
        assert_eq!(config.optimize, "s");
    }
    #[test]
    fn nested_configuration_and_validation() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                "[profile.default.fe]\npath = './toolchain/fe'\noptimize = '2'\n",
            )?;
            let config = Config::load().unwrap();
            assert_eq!(config.fe.path, Some(PathBuf::from("./toolchain/fe")));
            assert_eq!(config.fe.optimize, "2");
            assert!(config.warnings.is_empty(), "{:?}", config.warnings);
            Ok(())
        });
        let mut config = Config::default();
        config.fe.optimize = "3".into();
        assert!(config.fe_settings().unwrap_err().to_string().contains("optimizer"));
    }
}
