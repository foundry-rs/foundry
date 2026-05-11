//! Tempo utilities: fee token parsing and named nonce lanes (2D nonces).
//!
//! A "lane" is a friendly alias for a Tempo `nonce_key` (a [`U256`]). Lanes are defined in a
//! shared TOML file (default `tempo.lanes.toml` at the project root) so a team can reserve
//! independent sequential nonce streams for parallel scripts without coordinating on raw
//! `U256` selectors.
//!
//! Example `tempo.lanes.toml`:
//!
//! ```toml
//! deploy   = 1
//! ops      = 2
//! payments = 3
//! ```
//!
//! ```bash
//! cast erc20 transfer ... --tempo.lane payments
//! ```

use crate::opts::TempoOpts;
use alloy_primitives::{Address, U256};
use eyre::{Result, eyre};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};
use tempo_primitives::TempoAddressExt;

/// Default name of the lanes file at the project root.
pub const DEFAULT_LANES_FILE: &str = "tempo.lanes.toml";

/// Result of resolving a `--tempo.lane <name>` argument against a lanes file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLane {
    /// The lane name as provided on the CLI.
    pub name: String,
    /// The `nonce_key` the lane resolved to.
    pub nonce_key: U256,
}

/// Parses a fee token address.
pub fn parse_fee_token_address(address_or_id: &str) -> eyre::Result<Address> {
    Address::from_str(address_or_id).or_else(|_| Ok(token_id_to_address(address_or_id.parse()?)))
}

fn token_id_to_address(token_id: u64) -> Address {
    let mut address_bytes = [0u8; 20];
    address_bytes[..12].copy_from_slice(&Address::TIP20_PREFIX);
    address_bytes[12..20].copy_from_slice(&token_id.to_be_bytes());
    Address::from(address_bytes)
}

/// Loads a TOML lanes file from `path`.
///
/// Each top-level key is a lane name, and the value is the `nonce_key` (an integer or a
/// decimal/hex string parsed as [`U256`]).
pub fn load_lanes(path: &Path) -> Result<BTreeMap<String, U256>> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| eyre!("failed to read tempo lanes file {}: {}", path.display(), e))?;
    parse_lanes(&contents)
        .map_err(|e| eyre!("failed to parse tempo lanes file {}: {}", path.display(), e))
}

fn parse_lanes(contents: &str) -> Result<BTreeMap<String, U256>> {
    let raw: BTreeMap<String, toml::Value> = toml::from_str(contents)?;
    let mut out = BTreeMap::new();
    for (name, value) in raw {
        let nonce_key = match value {
            toml::Value::Integer(n) => {
                if n < 0 {
                    return Err(eyre!("invalid nonce_key for lane '{name}': must be non-negative"));
                }
                U256::from(n as u64)
            }
            toml::Value::String(s) => U256::from_str(s.trim())
                .map_err(|e| eyre!("invalid nonce_key for lane '{name}': {e}"))?,
            other => {
                return Err(eyre!(
                    "invalid nonce_key for lane '{name}': expected integer or string, got {}",
                    other.type_str(),
                ));
            }
        };
        out.insert(name, nonce_key);
    }
    Ok(out)
}

/// Resolves `opts.lane` against a lanes file and writes the resulting `nonce_key` to
/// `opts.nonce_key`. Returns the resolved lane (or `None` if no `--tempo.lane` was set).
///
/// `root` is the project root used to locate the default lanes file
/// (`<root>/tempo.lanes.toml`) when `--tempo.lanes-file` was not provided.
pub fn resolve_lane(opts: &mut TempoOpts, root: &Path) -> Result<Option<ResolvedLane>> {
    let Some(lane_name) = opts.lane.clone() else { return Ok(None) };

    let path: PathBuf = opts.lanes_file.clone().unwrap_or_else(|| root.join(DEFAULT_LANES_FILE));

    if !path.exists() {
        return Err(eyre!(
            "tempo lanes file not found at {}\n\
             create it with `name = <nonce_key>` entries, e.g.:\n  \
             deploy   = 1\n  \
             ops      = 2\n  \
             payments = 3",
            path.display(),
        ));
    }

    let lanes = load_lanes(&path)?;

    let nonce_key = lanes.get(&lane_name).copied().ok_or_else(|| {
        let mut known: Vec<&str> = lanes.keys().map(String::as_str).collect();
        known.sort_unstable();
        eyre!(
            "lane '{lane_name}' not found in {} (known lanes: {})",
            path.display(),
            if known.is_empty() { "<none>".to_string() } else { known.join(", ") },
        )
    })?;

    opts.nonce_key = Some(nonce_key);
    Ok(Some(ResolvedLane { name: lane_name, nonce_key }))
}

/// Prints `lane: <name> (nonce_key=<key>, nonce=<n>)` to stderr (so it doesn't pollute
/// stdout for commands like `cast mktx` whose stdout is meant to be piped), giving
/// visibility into which 2D nonce lane was used.
pub fn maybe_print_resolved_lane(resolved: Option<&ResolvedLane>, nonce: u64) -> Result<()> {
    if let Some(lane) = resolved {
        sh_eprintln!("lane: {} (nonce_key={}, nonce={})", lane.name, lane.nonce_key, nonce)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_int_and_string_lane_values() {
        let toml = r#"
deploy   = 1
ops      = 2
payments = "3"
big      = "115792089237316195423570985008687907853269984665640564039457584007913129639935"
"#;
        let lanes = parse_lanes(toml).unwrap();
        assert_eq!(lanes.get("deploy"), Some(&U256::from(1u64)));
        assert_eq!(lanes.get("ops"), Some(&U256::from(2u64)));
        assert_eq!(lanes.get("payments"), Some(&U256::from(3u64)));
        assert_eq!(lanes.get("big"), Some(&U256::MAX));
    }

    #[test]
    fn parse_lanes_rejects_invalid_string() {
        let toml = "broken = \"not-a-number\"";
        let err = parse_lanes(toml).unwrap_err();
        assert!(err.to_string().contains("invalid nonce_key for lane 'broken'"));
    }

    #[test]
    fn resolve_lane_sets_nonce_key_and_returns_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEFAULT_LANES_FILE);
        std::fs::write(&path, "deploy = 7\npayments = 42\n").unwrap();

        let mut opts = TempoOpts { lane: Some("payments".to_string()), ..Default::default() };
        let resolved = resolve_lane(&mut opts, dir.path()).unwrap().unwrap();
        assert_eq!(resolved.name, "payments");
        assert_eq!(resolved.nonce_key, U256::from(42u64));
        assert_eq!(opts.nonce_key, Some(U256::from(42u64)));
    }

    #[test]
    fn resolve_lane_returns_none_when_no_lane() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = TempoOpts::default();
        let resolved = resolve_lane(&mut opts, dir.path()).unwrap();
        assert!(resolved.is_none());
        assert!(opts.nonce_key.is_none());
    }

    #[test]
    fn resolve_lane_errors_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = TempoOpts { lane: Some("deploy".to_string()), ..Default::default() };
        let err = resolve_lane(&mut opts, dir.path()).unwrap_err();
        assert!(err.to_string().contains("tempo lanes file not found"));
    }

    #[test]
    fn resolve_lane_errors_when_lane_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEFAULT_LANES_FILE);
        std::fs::write(&path, "deploy = 1\nops = 2\n").unwrap();

        let mut opts = TempoOpts { lane: Some("payments".to_string()), ..Default::default() };
        let err = resolve_lane(&mut opts, dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("lane 'payments' not found"));
        assert!(msg.contains("deploy, ops"));
    }
}
