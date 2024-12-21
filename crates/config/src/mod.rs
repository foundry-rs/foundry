#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_ignored_errors_patterns() {
        let config = Config {
            ignored_error_codes_from: vec![
                ErrorIgnorePattern {
                    pattern: "test/**/*.sol".to_string(),
                    codes: vec!["C001".to_string()],
                },
            ],
            ..Default::default()
        };

        assert!(config.get_ignored_errors_for_path(Path::new("test/Contract.sol")).contains(&"C001".to_string()));
        assert!(config.get_ignored_errors_for_path(Path::new("src/Contract.sol")).is_empty());
    }
}
