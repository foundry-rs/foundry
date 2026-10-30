//! Helper for resolving env vars

use regex::Regex;
use std::{
    env,
    env::VarError,
    fmt,
    sync::{LazyLock, OnceLock},
};

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
}

impl UnresolvedEnvVarError {
    /// Tries to resolve a value
    pub fn try_resolve(&self) -> Result<String, Self> {
        interpolate(&self.unresolved)
    }
}

impl fmt::Display for UnresolvedEnvVarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The unresolved input is omitted because it can contain literal credentials.
        write!(f, "environment variable `{}` ", self.var)?;
        f.write_str(match self.source {
            VarError::NotPresent => "not found",
            VarError::NotUnicode(_) => "is not valid unicode",
        })
    }
}

impl std::error::Error for UnresolvedEnvVarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Looks up a value that is not set in the environment, e.g. in an encrypted secret store.
pub type SecretResolver = fn(&str) -> Option<String>;

static SECRET_RESOLVER: OnceLock<SecretResolver> = OnceLock::new();

/// Sets the resolver that [`interpolate`] asks for variables missing from the environment.
///
/// The environment always takes precedence. Only the first call has an effect.
pub fn set_secret_resolver(resolver: SecretResolver) {
    let _ = SECRET_RESOLVER.set(resolver);
}

/// Returns the stored secret `name`, without reading the environment.
///
/// For values that are read from the environment outside of `${NAME}` placeholders, such as
/// `ETH_RPC_URL` and `ETHERSCAN_API_KEY`: callers check the environment first.
pub fn secret(name: &str) -> Option<String> {
    SECRET_RESOLVER.get().and_then(|resolve| resolve(name))
}

/// Returns the value of `var` from the environment, or else from the secret resolver.
fn lookup(var: &str) -> Result<String, VarError> {
    match env::var(var) {
        Err(VarError::NotPresent) => {
            SECRET_RESOLVER.get().and_then(|resolve| resolve(var)).ok_or(VarError::NotPresent)
        }
        res => res,
    }
}

/// Replaces all Env var placeholders in the input string with the values they hold, falling back to
/// the secret resolver for variables that are not set.
pub fn interpolate(input: &str) -> Result<String, UnresolvedEnvVarError> {
    interpolate_with(input, lookup)
}

/// Like [`interpolate`], but reads only the environment and never the secret resolver.
///
/// Used where the result reaches user code, such as the `resolveEnv` cheatcode.
pub fn interpolate_env_only(input: &str) -> Result<String, UnresolvedEnvVarError> {
    interpolate_with(input, |var| env::var(var))
}

fn interpolate_with(
    input: &str,
    lookup: impl Fn(&str) -> Result<String, VarError>,
) -> Result<String, UnresolvedEnvVarError> {
    let mut res = input.to_string();

    // loop over all placeholders in the input and replace them one by one
    for caps in RE_PLACEHOLDER.captures_iter(input) {
        let var = &caps["inner"];
        let value = lookup(var).map_err(|source| UnresolvedEnvVarError {
            unresolved: input.to_string(),
            var: var.to_string(),
            source,
        })?;

        res = res.replacen(&caps["outer"], &value, 1);
    }
    Ok(res)
}

/// Secret resolver shared by tests, since only the first [`set_secret_resolver`] call counts.
///
/// Knows `FOUNDRY_TEST_SECRET_A`/`_B`, and `ETHERSCAN_API_KEY` while
/// `FOUNDRY_TEST_STORE_ETHERSCAN_KEY` is set, so that other tests see no stored key.
#[cfg(test)]
pub(crate) fn test_secret_resolver(var: &str) -> Option<String> {
    match var {
        "FOUNDRY_TEST_SECRET_A" | "FOUNDRY_TEST_SECRET_B" => Some("from-store".to_string()),
        "ETHERSCAN_API_KEY" => env::var("FOUNDRY_TEST_STORE_ETHERSCAN_KEY").ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_resolver_after_environment_and_not_for_env_only() {
        set_secret_resolver(test_secret_resolver);

        // Not in the environment: the store is used.
        assert_eq!(interpolate("x/${FOUNDRY_TEST_SECRET_A}").unwrap(), "x/from-store");
        // The environment wins.
        unsafe { env::set_var("FOUNDRY_TEST_SECRET_B", "from-env") };
        assert_eq!(interpolate("${FOUNDRY_TEST_SECRET_B}").unwrap(), "from-env");
        unsafe { env::remove_var("FOUNDRY_TEST_SECRET_B") };
        // Values that reach user code read only the environment.
        let err = interpolate_env_only("${FOUNDRY_TEST_SECRET_A}").unwrap_err();
        assert_eq!(err.var, "FOUNDRY_TEST_SECRET_A");
        // Names the resolver does not know stay unresolved.
        assert!(interpolate("${FOUNDRY_TEST_SECRET_UNKNOWN}").is_err());
    }

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
        assert_eq!(err.to_string(), "environment variable `FOUNDRY_TEST_UNSET_A` not found");
    }
}
