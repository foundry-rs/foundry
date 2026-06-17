//! Stable, machine-readable diagnostic codes emitted by `forge`.
//!
//! See [`docs/agents/diagnostics.md`](../../../docs/agents/diagnostics.md)
//! for the format and registry rules. Most codes re-export per-domain
//! constants from [`foundry_cli::diagnostic`].

/// Build-time diagnostic codes (compilation, linking, artifact generation).
pub mod build {
    pub use foundry_cli::diagnostic::compiler::SOLC_ERROR;

    pub(crate) const ALL: &[&str] = &[SOLC_ERROR];
}

/// `forge test` diagnostic codes.
pub mod test {
    pub use foundry_cli::diagnostic::test::{FAILED, SETUP_FAILED, WARNING};

    pub(crate) const ALL: &[&str] = &[FAILED, SETUP_FAILED, WARNING];
}

/// All diagnostic codes declared by `forge`.
pub fn known_codes() -> Vec<&'static str> {
    let groups: &[&[&str]] = &[build::ALL, test::ALL];
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
