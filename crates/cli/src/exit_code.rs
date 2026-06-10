//! Canonical process exit codes for the Foundry agent contract.
//!
//! See [`docs/agents/exit-codes.md`](../../../docs/agents/exit-codes.md) for the
//! contract these codes implement.

use std::fmt;

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
}
