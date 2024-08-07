/// Errors returned by the [`InlineConfigParser`](crate::InlineConfigParser) trait.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
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

/// Wrapper error struct that catches config parsing errors, enriching them with context information
/// reporting the misconfigured line.
#[derive(Debug, thiserror::Error)]
#[error("Inline config error detected at {line}")]
pub struct InlineConfigError {
    /// Specifies the misconfigured line. This is something of the form
    /// `dir/TestContract.t.sol:FuzzContract:10:12:111`
    pub line: String,
    /// The inner error
    pub source: InlineConfigParserError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_format_inline_config_errors() {
        let source = InlineConfigParserError::ParseBool("key".into(), "invalid-bool-value".into());
        let line = "dir/TestContract.t.sol:FuzzContract".to_string();
        let error = InlineConfigError { line: line.clone(), source };

        let expected = format!("Inline config error detected at {line}");
        assert_eq!(error.to_string(), expected);
    }
}
