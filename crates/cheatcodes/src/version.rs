use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;
use foundry_common::version::SEMVER_VERSION;
use semver::Version;
use std::cmp::Ordering;

impl Cheatcode for foundryVersionCmpCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { version } = self;

        let parsed_version = Version::parse(version)
            .map_err(|e| fmt_err!("Invalid semver format '{}': {}", version, e))?;

        let current_version = Version::parse(SEMVER_VERSION)
            .map_err(|e| fmt_err!("Invalid current version: {}", e))?;
        // Compare the current Foundry version (SEMVER_VERSION) against the parsed version.
        // Note: returns -1 if current < provided, 0 if equal, 1 if current > provided.
        let cmp_result = match current_version.cmp_precedence(&parsed_version) {
            Ordering::Less => -1i32,
            Ordering::Equal => 0i32,
            Ordering::Greater => 1i32,
        };
        Ok(cmp_result.abi_encode())
    }
}

impl Cheatcode for foundryVersionAtLeastCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { version } = self;

        let parsed_version = Version::parse(version)
            .map_err(|e| fmt_err!("Invalid semver format '{}': {}", version, e))?;

        let current_version = Version::parse(SEMVER_VERSION)
            .map_err(|e| fmt_err!("Invalid current version: {}", e))?;

        let at_least = current_version.cmp_precedence(&parsed_version) != Ordering::Less;
        Ok(at_least.abi_encode())
    }
}
