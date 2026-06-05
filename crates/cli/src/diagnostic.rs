//! Stable, machine-readable diagnostic codes.
//!
//! Codes attach to [`JsonMessage`](crate::json::JsonMessage) entries inside
//! [`JsonEnvelope`](crate::json::JsonEnvelope) `errors[]` and `warnings[]`.
//!
//! See [`docs/agents/diagnostics.md`](../../../docs/agents/diagnostics.md) for
//! the format and registry rules. Codes are namespaced strings of the form
//! `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`.
//!
//! # Implementation choice
//!
//! Codes are exposed as `&'static str` constants, organised in per-domain
//! modules colocated with this crate (or, for adoption PRs, with the owning
//! crate). [`JsonMessage::error`](crate::json::JsonMessage::error) accepts
//! `impl Into<String>`, so call sites pass the constant directly. The
//! [`DiagnosticCode`] newtype is available when callers want a parsed,
//! validated value.

use std::{fmt, sync::OnceLock};

/// Validation error returned by [`DiagnosticCode::new`] / [`validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidDiagnosticCode {
    code: String,
}

impl fmt::Display for InvalidDiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid diagnostic code `{}`: must match `^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)+$`",
            self.code
        )
    }
}

impl std::error::Error for InvalidDiagnosticCode {}

/// Validate that `s` matches the diagnostic-code grammar.
pub fn validate(s: &str) -> Result<(), InvalidDiagnosticCode> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$")
            .expect("diagnostic-code regex compiles")
    });
    if re.is_match(s) { Ok(()) } else { Err(InvalidDiagnosticCode { code: s.to_string() }) }
}

/// Stable, machine-readable diagnostic code attached to a structured message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    /// Create a new code, validating against the registry grammar.
    pub fn new(code: impl Into<String>) -> Result<Self, InvalidDiagnosticCode> {
        let code = code.into();
        validate(&code)?;
        Ok(Self(code))
    }

    /// Borrow the underlying code as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the underlying owned `String`.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for DiagnosticCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<DiagnosticCode> for String {
    fn from(code: DiagnosticCode) -> Self {
        code.0
    }
}

impl std::str::FromStr for DiagnosticCode {
    type Err = InvalidDiagnosticCode;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// CLI-layer diagnostic codes (argument parsing, global flags).
pub mod cli {
    /// Command-line usage was invalid (parse error, missing subcommand,
    /// invalid flag combination).
    pub const USAGE_INVALID: &str = "cli.usage.invalid";
    /// `--help` was requested.
    pub const HELP: &str = "cli.help";
    /// `--version` was requested.
    pub const VERSION: &str = "cli.version";
    /// Process was interrupted (`SIGINT` / `SIGTERM`).
    pub const INTERRUPTED: &str = "cli.interrupted";
    /// Catch-all for failures that do not fit a more specific domain.
    pub const UNKNOWN: &str = "cli.unknown";

    pub(crate) const ALL: &[&str] = &[USAGE_INVALID, HELP, VERSION, INTERRUPTED, UNKNOWN];
}

/// `foundry-config` diagnostic codes.
pub mod config {
    pub const INVALID: &str = "config.invalid";
    pub const MISSING_FIELD: &str = "config.missing_field";

    pub(crate) const ALL: &[&str] = &[INVALID, MISSING_FIELD];
}

/// Compiler diagnostic codes (`foundry-compilers`, `forge`).
pub mod compiler {
    pub const SOLC_ERROR: &str = "compiler.solc.error";
    pub const VYPER_ERROR: &str = "compiler.vyper.error";

    pub(crate) const ALL: &[&str] = &[SOLC_ERROR, VYPER_ERROR];
}

/// Network / RPC diagnostic codes.
pub mod network {
    /// Generic RPC failure (transport, JSON-RPC error, connectivity).
    pub const RPC_ERROR: &str = "network.rpc.error";
    pub const RPC_TIMEOUT: &str = "network.rpc.timeout";
    pub const RPC_UNAUTHORIZED: &str = "network.rpc.unauthorized";

    pub(crate) const ALL: &[&str] = &[RPC_ERROR, RPC_TIMEOUT, RPC_UNAUTHORIZED];
}

/// `foundry-wallets` diagnostic codes.
pub mod wallet {
    pub const KEY_MISSING: &str = "wallet.key.missing";
    pub const SIGNATURE_REJECTED: &str = "wallet.signature.rejected";

    pub(crate) const ALL: &[&str] = &[KEY_MISSING, SIGNATURE_REJECTED];
}

/// `forge test` diagnostic codes.
pub mod test {
    pub const FAILED: &str = "test.failed";
    pub const SETUP_FAILED: &str = "test.setup_failed";
    /// Non-fatal advisory surfaced by a test suite.
    pub const WARNING: &str = "test.warning";

    pub(crate) const ALL: &[&str] = &[FAILED, SETUP_FAILED, WARNING];
}

/// `forge script` diagnostic codes.
pub mod script {
    pub const BROADCAST_FAILED: &str = "script.broadcast_failed";

    pub(crate) const ALL: &[&str] = &[BROADCAST_FAILED];
}

/// `cast` diagnostic codes.
pub mod cast {
    pub const TX_NOT_FOUND: &str = "cast.tx.not_found";

    pub(crate) const ALL: &[&str] = &[TX_NOT_FOUND];
}

/// `anvil` diagnostic codes.
pub mod anvil {
    pub const FORK_UNREACHABLE: &str = "anvil.fork.unreachable";

    pub(crate) const ALL: &[&str] = &[FORK_UNREACHABLE];
}

/// `chisel` diagnostic codes.
pub mod chisel {
    pub const SESSION_INVALID: &str = "chisel.session.invalid";

    pub(crate) const ALL: &[&str] = &[SESSION_INVALID];
}

/// All diagnostic codes declared in this crate.
///
/// Useful for repo-wide validation tests.
pub fn known_codes() -> Vec<&'static str> {
    let groups: &[&[&str]] = &[
        cli::ALL,
        config::ALL,
        compiler::ALL,
        network::ALL,
        wallet::ALL,
        test::ALL,
        script::ALL,
        cast::ALL,
        anvil::ALL,
        chisel::ALL,
    ];
    groups.iter().flat_map(|g| g.iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_codes_round_trip() {
        for c in &["cli.usage.invalid", "config.invalid", "network.rpc.timeout", "a.b.c.d.e"] {
            let code = DiagnosticCode::new(*c).unwrap();
            assert_eq!(code.as_str(), *c);
        }
    }

    #[test]
    fn invalid_codes_rejected() {
        for c in &[
            "",
            "no_dot",
            "Cli.usage",         // uppercase
            ".cli.usage",        // leading dot
            "cli.usage.",        // trailing dot
            "cli..usage",        // empty segment
            "1cli.usage",        // segment must start with [a-z]
            "cli.1usage",        // segment must start with [a-z]
            "cli.usage-invalid", // dash not allowed
            "cli.usage invalid", // space not allowed
        ] {
            assert!(DiagnosticCode::new(*c).is_err(), "expected `{c}` to be rejected");
        }
    }

    #[test]
    fn every_known_code_validates() {
        for c in known_codes() {
            assert!(validate(c).is_ok(), "registered code `{c}` failed validation");
        }
    }

    #[test]
    fn known_codes_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        let mut dups = Vec::new();
        for c in known_codes() {
            if !seen.insert(c) {
                dups.push(c);
            }
        }
        assert!(dups.is_empty(), "duplicate diagnostic codes: {dups:?}");
    }
}
