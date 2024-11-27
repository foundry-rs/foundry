/// Errors returned when parsing inline config.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InlineConfigParserError {
    /// An invalid profile has been provided
    #[error("'{0}' specifies an invalid profile. Available profiles are: {1}")]
    InvalidProfile(String, String),
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
        let source = InlineConfigParserError::InvalidProfile("key".into(), "a, b, c".into());
        let line = "dir/TestContract.t.sol:FuzzContract".to_string();
        let error = InlineConfigError { line: line.clone(), source };

        let expected = format!("Inline config error detected at {line}");
        assert_eq!(error.to_string(), expected);
    }
}
