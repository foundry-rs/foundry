use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorIgnorePattern {
    pub pattern: String,
    pub codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Patterns for Ignoring Specific Error Codes
    #[serde(default)]
    pub ignored_error_codes_from: Vec<ErrorIgnorePattern>,
}

impl Config {
    /// Retrieves a list of ignored error codes for the specified path
    pub fn get_ignored_errors_for_path(&self, path: &Path) -> Vec<String> {
        self.ignored_error_codes_from
            .iter()
            .filter(|pattern| glob::Pattern::new(&pattern.pattern)
                .map(|glob| glob.matches(path.to_str().unwrap_or_default()))
                .unwrap_or(false))
            .flat_map(|pattern| pattern.codes.clone())
            .collect()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ignored_error_codes_from: Vec::new(),
        }
    }
}

impl Default for ErrorIgnorePattern {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            codes: Vec::new(),
        }
    }
}
