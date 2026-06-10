use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;
use foundry_evm_core::evm::FoundryEvmNetwork;
use semver::Version;
use std::cmp::Ordering;

impl Cheatcode for foundryVersionCmpCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { version } = self;
        foundry_version_cmp(version).map(|cmp| (cmp as i8).abi_encode())
    }
}

impl Cheatcode for foundryVersionAtLeastCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, _state: &mut Cheatcodes<FEN>) -> Result {
        let Self { version } = self;
        foundry_version_cmp(version).map(|cmp| cmp.is_ge().abi_encode())
    }
}

fn foundry_version_cmp(version: &str) -> Result<Ordering> {
    version_cmp(strip_semver_metadata(SEMVER_VERSION), version)
}

/// Strips pre-release (e.g. `-nightly`, `-dev`) and build metadata
/// (e.g. `+<sha_short>.<unix_timestamp>.<profile>`) from a version string
/// so we compare on `MAJOR.MINOR.PATCH` only.
fn strip_semver_metadata(version: &str) -> &str {
    version.split(['-', '+']).next().unwrap()
}

fn version_cmp(version_a: &str, version_b: &str) -> Result<Ordering> {
    let version_a = parse_version(version_a)?;
    let version_b = parse_version(version_b)?;
    Ok(version_a.cmp(&version_b))
}

fn parse_version(version: &str) -> Result<Version> {
    let version =
        Version::parse(version).map_err(|e| fmt_err!("invalid version `{version}`: {e}"))?;
    if !version.pre.is_empty() {
        return Err(fmt_err!(
            "invalid version `{version}`: pre-release versions are not supported"
        ));
    }
    if !version.build.is_empty() {
        return Err(fmt_err!("invalid version `{version}`: build metadata is not supported"));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_build_metadata_only() {
        // Tagged release: `1.7.1+<sha>.<ts>.<profile>`
        assert_eq!(strip_semver_metadata("1.7.1+abc1234567.1737036656.release"), "1.7.1");
    }

    #[test]
    fn strips_pre_release_and_build_metadata() {
        // Nightly: `1.7.1-nightly+<sha>.<ts>.<profile>`
        assert_eq!(strip_semver_metadata("1.7.1-nightly+abc1234567.1737036656.release"), "1.7.1");
        // Dev: `1.7.1-dev+<sha>.<ts>.<profile>`
        assert_eq!(strip_semver_metadata("1.7.1-dev+abc1234567.1737036656.debug"), "1.7.1");
    }

    #[test]
    fn strips_plain_version() {
        assert_eq!(strip_semver_metadata("1.7.1"), "1.7.1");
    }

    #[test]
    fn version_cmp_orders_correctly() {
        assert_eq!(version_cmp("1.7.1", "1.7.1").unwrap(), Ordering::Equal);
        assert_eq!(version_cmp("1.7.1", "1.7.0").unwrap(), Ordering::Greater);
        assert_eq!(version_cmp("1.7.1", "1.7.2").unwrap(), Ordering::Less);
        assert_eq!(version_cmp("1.7.1", "0.0.1").unwrap(), Ordering::Greater);
        assert_eq!(version_cmp("1.7.1", "99.0.0").unwrap(), Ordering::Less);
    }

    #[test]
    fn parse_version_rejects_pre_release_and_build_metadata() {
        // User-supplied versions must be plain `MAJOR.MINOR.PATCH`.
        assert!(parse_version("1.7.1-nightly").is_err());
        assert!(parse_version("1.7.1+abc").is_err());
        assert!(parse_version("not-a-version").is_err());
        assert!(parse_version("1.7.1").is_ok());
    }

    #[test]
    fn cmp_works_against_full_semver_version_strings() {
        // Simulate comparing each shape of `SEMVER_VERSION` against a user-supplied version.
        for current in [
            "1.7.1+abc1234567.1737036656.release",
            "1.7.1-nightly+abc1234567.1737036656.release",
            "1.7.1-dev+abc1234567.1737036656.debug",
            "1.7.1",
        ] {
            let stripped = strip_semver_metadata(current);
            assert_eq!(version_cmp(stripped, "1.7.1").unwrap(), Ordering::Equal);
            assert_eq!(version_cmp(stripped, "1.7.0").unwrap(), Ordering::Greater);
            assert_eq!(version_cmp(stripped, "1.7.2").unwrap(), Ordering::Less);
        }
    }
}
