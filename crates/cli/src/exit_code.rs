//! Canonical process exit codes for the Foundry agent contract.
//!
//! See [`docs/agents/exit-codes.md`](../../../docs/agents/exit-codes.md) for the
//! contract these codes implement.

use std::{fmt, fmt::Write};

/// Canonical exit codes emitted by Foundry binaries.
///
/// Only the variants below are guaranteed by the agent contract. Commands MAY
/// document additional, command-specific codes via
/// [`ExitCodeInfo`](crate::introspect::ExitCodeInfo); those codes MUST NOT
/// collide with this global table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ExitCode {
    /// Command completed successfully.
    Success = 0,
    /// Unclassified failure.
    GenericError = 1,
    /// Argument parse error, missing subcommand, or invalid flag combination.
    Usage = 2,
    /// Foundry config invalid or missing required value.
    Config = 3,
    /// Compilation, linking, or artifact generation failed.
    Build = 4,
    /// Tests ran but at least one failed (distinct from a build/setup failure).
    TestFailure = 5,
    /// RPC, HTTP, or chain-connectivity failure.
    Network = 6,
    /// Authentication, authorization, or wallet/key-related failure.
    User = 7,
    /// Command terminated by `SIGINT` / `SIGTERM`.
    Interrupted = 8,
}

impl ExitCode {
    /// Returns the numeric process exit code.
    pub const fn to_i32(self) -> i32 {
        self as i32
    }

    /// Returns the stable, human-readable variant name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::GenericError => "GenericError",
            Self::Usage => "Usage",
            Self::Config => "Config",
            Self::Build => "Build",
            Self::TestFailure => "TestFailure",
            Self::Network => "Network",
            Self::User => "User",
            Self::Interrupted => "Interrupted",
        }
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name(), self.to_i32())
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code.to_i32()
    }
}

impl From<&eyre::Report> for ExitCode {
    /// Best-effort classification of a report.
    ///
    /// Walks the error chain looking for keywords characteristic of common
    /// failure categories; falls back to [`ExitCode::GenericError`] when no
    /// category matches. The mapping is intentionally conservative — adoption
    /// PRs will tighten it as commands return typed errors.
    fn from(report: &eyre::Report) -> Self {
        let mut buf = String::new();
        for cause in report.chain() {
            let _ = writeln!(buf, "{cause}");
        }
        let lower = buf.to_lowercase();

        // Order matters: classify auth/user signals before network so a 401
        // from an RPC provider doesn't get misfiled as a transient network
        // error.
        if lower.contains("interrupted") || lower.contains("sigint") || lower.contains("sigterm") {
            return Self::Interrupted;
        }

        if lower.contains("foundry.toml")
            || (lower.contains("config")
                && (lower.contains("invalid") || lower.contains("missing")))
        {
            return Self::Config;
        }

        if lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("authentication")
            || lower.contains("authorization")
            || lower.contains("api key")
            || lower.contains("private key")
            || lower.contains("keystore")
            || lower.contains("wallet")
            || lower.contains("signer")
        {
            return Self::User;
        }

        if lower.contains("rpc")
            || lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("connection refused")
            || lower.contains("dns")
        {
            return Self::Network;
        }

        if lower.contains("compil") || lower.contains("solc") || lower.contains("vyper") {
            return Self::Build;
        }

        Self::GenericError
    }
}

impl From<eyre::Report> for ExitCode {
    fn from(report: eyre::Report) -> Self {
        (&report).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_codes_match_spec() {
        assert_eq!(ExitCode::Success.to_i32(), 0);
        assert_eq!(ExitCode::GenericError.to_i32(), 1);
        assert_eq!(ExitCode::Usage.to_i32(), 2);
        assert_eq!(ExitCode::Config.to_i32(), 3);
        assert_eq!(ExitCode::Build.to_i32(), 4);
        assert_eq!(ExitCode::TestFailure.to_i32(), 5);
        assert_eq!(ExitCode::Network.to_i32(), 6);
        assert_eq!(ExitCode::User.to_i32(), 7);
        assert_eq!(ExitCode::Interrupted.to_i32(), 8);
    }

    #[test]
    fn report_classifies_config() {
        let r: eyre::Report = eyre::eyre!("could not parse foundry.toml: invalid value");
        assert_eq!(ExitCode::from(&r), ExitCode::Config);
    }

    #[test]
    fn report_classifies_network() {
        let r: eyre::Report = eyre::eyre!("RPC connection timeout");
        assert_eq!(ExitCode::from(&r), ExitCode::Network);
    }

    #[test]
    fn report_classifies_wallet() {
        let r: eyre::Report = eyre::eyre!("wallet: missing private key");
        assert_eq!(ExitCode::from(&r), ExitCode::User);
    }

    #[test]
    fn report_classifies_compiler() {
        let r: eyre::Report = eyre::eyre!("solc compilation failed");
        assert_eq!(ExitCode::from(&r), ExitCode::Build);
    }

    #[test]
    fn report_falls_back_to_generic() {
        let r: eyre::Report = eyre::eyre!("something unexpected went wrong");
        assert_eq!(ExitCode::from(&r), ExitCode::GenericError);
    }

    #[test]
    fn auth_classified_before_network() {
        let r: eyre::Report = eyre::eyre!("RPC call failed: 401 unauthorized");
        assert_eq!(ExitCode::from(&r), ExitCode::User);
    }

    #[test]
    fn dns_failure_is_network() {
        let r: eyre::Report = eyre::eyre!("dns lookup failed for mainnet.alchemy.com");
        assert_eq!(ExitCode::from(&r), ExitCode::Network);
    }

    #[test]
    fn interrupt_classification() {
        let r: eyre::Report = eyre::eyre!("interrupted by SIGINT");
        assert_eq!(ExitCode::from(&r), ExitCode::Interrupted);
    }
}
