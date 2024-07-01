//! Helper for resolving env vars

use once_cell::sync::Lazy;
use regex::Regex;
use std::{env, env::VarError, fmt};

/// A regex that matches `${val}` placeholders
pub static RE_PLACEHOLDER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)(?P<outer>\$\{\s*(?P<inner>.*?)\s*})").unwrap());

/// Error when we failed to resolve an env var
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedEnvVarError {
    /// The unresolved input string
    pub unresolved: String,
    /// Var that couldn't be resolved
    pub var: String,
    /// the `env::var` error
    pub source: VarError,
}

impl UnresolvedEnvVarError {
    /// Tries to resolve a value
    pub fn try_resolve(&self) -> Result<String, Self> {
        interpolate(&self.unresolved)
    }
}

impl fmt::Display for UnresolvedEnvVarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Failed to resolve env var `{}` in `{}`: {}",
            self.var, self.unresolved, self.source
        )
    }
}

impl std::error::Error for UnresolvedEnvVarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Replaces all Env var placeholders in the input string with the values they hold
pub fn interpolate(input: &str) -> Result<String, UnresolvedEnvVarError> {
    let mut res = input.to_string();

    // loop over all placeholders in the input and replace them one by one
    for caps in RE_PLACEHOLDER.captures_iter(input) {
        let var = &caps["inner"];
        let value = env::var(var).map_err(|source| UnresolvedEnvVarError {
            unresolved: input.to_string(),
            var: var.to_string(),
            source,
        })?;

        res = res.replacen(&caps["outer"], &value, 1);
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_find_placeholder() {
        let val = "https://eth-mainnet.alchemyapi.io/v2/346273846238426342";
        assert!(!RE_PLACEHOLDER.is_match(val));

        let val = "${RPC_ENV}";
        assert!(RE_PLACEHOLDER.is_match(val));

        let val = "https://eth-mainnet.alchemyapi.io/v2/${API_KEY}";
        assert!(RE_PLACEHOLDER.is_match(val));

        let cap = RE_PLACEHOLDER.captures(val).unwrap();
        assert_eq!(cap.name("outer").unwrap().as_str(), "${API_KEY}");
        assert_eq!(cap.name("inner").unwrap().as_str(), "API_KEY");
    }
}
