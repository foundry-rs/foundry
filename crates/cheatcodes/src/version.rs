use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;
use semver::Version;
use std::cmp::Ordering;

impl Cheatcode for foundryVersionCmpCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { version } = self;
        foundry_version_cmp(version).map(|cmp| (cmp as i8).abi_encode())
    }
}

impl Cheatcode for foundryVersionAtLeastCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { version } = self;
        foundry_version_cmp(version).map(|cmp| cmp.is_ge().abi_encode())
    }
}

fn foundry_version_cmp(version: &str) -> Result<Ordering> {
    version_cmp(SEMVER_VERSION.split('-').next().unwrap(), version)
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
        return Err(fmt_err!("invalid version `{version}`: pre-release versions are not supported"));
    }
    if !version.build.is_empty() {
        return Err(fmt_err!("invalid version `{version}`: build metadata is not supported"));
    }
    Ok(version)
}
