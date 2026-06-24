//! Stable, machine-readable diagnostic codes emitted by `cast`.
//!
//! See [`docs/agents/diagnostics.md`](../../../docs/agents/diagnostics.md)
//! for the format and registry rules. Most codes re-export per-domain
//! constants from [`foundry_cli::diagnostic`].

/// Diagnostic codes for read-only RPC commands like `cast call`, `cast tx`.
pub mod rpc {
    pub use foundry_cli::diagnostic::network::{RPC_ERROR, RPC_TIMEOUT, RPC_UNAUTHORIZED};

    pub(crate) const ALL: &[&str] = &[RPC_ERROR, RPC_TIMEOUT, RPC_UNAUTHORIZED];
}

/// All diagnostic codes declared by `cast`.
pub fn known_codes() -> Vec<&'static str> {
    let groups: &[&[&str]] = &[rpc::ALL];
    groups.iter().flat_map(|g| g.iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli::diagnostic::validate;

    #[test]
    fn every_known_code_validates() {
        for c in known_codes() {
            assert!(validate(c).is_ok(), "registered code `{c}` failed validation");
        }
    }
}
