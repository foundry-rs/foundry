//! Helper for resolving env vars

use regex::Regex;
use std::{env, env::VarError, fmt, sync::LazyLock};

/// A regex that matches `${val}` placeholders
pub static RE_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(?P<outer>\$\{\s*(?P<inner>.*?)\s*})").unwrap());

/// Error when we failed to resolve an env var
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedEnvVarError {
    /// The unresolved input string
    pub unresolved: String,
    /// Var that couldn't be resolved
    pub var: String,
    /// the `env::var` error
    pub source: VarError,
    /// The config key that references the variable, e.g. `rpc_endpoints.sepolia`.
    pub context: Option<String>,
}

impl UnresolvedEnvVarError {
    /// Tries to resolve a value
    pub fn try_resolve(&self) -> Result<String, Self> {
        interpolate(&self.unresolved).map_err(|err| Self { context: self.context.clone(), ..err })
    }

    /// Sets the config key that references the variable.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

impl fmt::Display for UnresolvedEnvVarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The unresolved input is omitted because it can contain literal credentials.
        write!(f, "environment variable `{}` ", self.var)?;
        f.write_str(match self.source {
            VarError::NotPresent => "not found",
            VarError::NotUnicode(_) => "is not valid unicode",
        })?;
        if let Some(context) = &self.context {
            write!(f, " (used by `{context}` in foundry.toml)")?;
        }
        if matches!(self.source, VarError::NotPresent) {
            f.write_str("; set it in your environment or in the project's `.env` file")?;
        }
        Ok(())
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
            context: None,
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

    #[test]
    fn unresolved_error_omits_input() {
        let err = interpolate(
            "https://user:synthetic-password@rpc.invalid/${FOUNDRY_TEST_UNSET_A}/${FOUNDRY_TEST_UNSET_B}",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "environment variable `FOUNDRY_TEST_UNSET_A` not found; set it in your environment or in the project's `.env` file"
        );
        assert_eq!(
            err.with_context("rpc_endpoints.sepolia").to_string(),
            "environment variable `FOUNDRY_TEST_UNSET_A` not found (used by `rpc_endpoints.sepolia` in foundry.toml); set it in your environment or in the project's `.env` file"
        );
    }
}
