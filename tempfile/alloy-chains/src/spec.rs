//! Specification of Ethereum EIP-155 chains.

use crate::NamedChain;
use strum::IntoEnumIterator;

#[allow(unused_imports)]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
};

/// Ethereum EIP-155 chains.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Chains {
    /// Map of chain IDs to chain definitions.
    pub chains: BTreeMap<u64, Chain>,
}

impl Default for Chains {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Chains {
    /// Constructs an empty set of chains.
    #[inline]
    pub fn empty() -> Self {
        Self { chains: Default::default() }
    }

    /// Returns the default chains.
    pub fn new() -> Self {
        Self { chains: NamedChain::iter().map(|c| (c as u64, Chain::new(c))).collect() }
    }
}

/// Specification for a single chain.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Chain {
    /// The chain's internal ID. This is the Rust enum variant's name.
    pub internal_id: String,
    /// The chain's name. This is used in CLI argument parsing, TOML serialization etc.
    pub name: String,
    /// An optional hint for the average block time, in milliseconds.
    pub average_blocktime_hint: Option<u64>,
    /// Whether the chain is a legacy chain, which does not support EIP-1559.
    pub is_legacy: bool,
    /// Whether the chain supports the Shanghai hardfork.
    pub supports_shanghai: bool,
    /// Whether the chain is a testnet.
    pub is_testnet: bool,
    /// The chain's native currency symbol (e.g. `ETH`).
    pub native_currency_symbol: Option<String>,
    /// The chain's base block explorer API URL (e.g. `https://api.etherscan.io/`).
    pub etherscan_api_url: Option<String>,
    /// The chain's base block explorer base URL (e.g. `https://etherscan.io/`).
    pub etherscan_base_url: Option<String>,
    /// The name of the environment variable that contains the Etherscan API key.
    pub etherscan_api_key_name: Option<String>,
}

impl Chain {
    /// Constructs a new chain specification from the given [`NamedChain`].
    pub fn new(c: NamedChain) -> Self {
        // TODO(MSRV-1.66): Use `Option::unzip`
        let (etherscan_api_url, etherscan_base_url) = match c.etherscan_urls() {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
        Self {
            internal_id: format!("{c:?}"),
            name: c.to_string(),
            average_blocktime_hint: c
                .average_blocktime_hint()
                .map(|d| d.as_millis().try_into().unwrap_or(u64::MAX)),
            is_legacy: c.is_legacy(),
            supports_shanghai: c.supports_shanghai(),
            is_testnet: c.is_testnet(),
            native_currency_symbol: c.native_currency_symbol().map(Into::into),
            etherscan_api_url: etherscan_api_url.map(Into::into),
            etherscan_base_url: etherscan_base_url.map(Into::into),
            etherscan_api_key_name: c.etherscan_api_key_name().map(Into::into),
        }
    }
}

#[cfg(all(test, feature = "std", feature = "serde", feature = "schema"))]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    const JSON_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/chains.json");
    const SCHEMA_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/chains.schema.json");

    fn json_chains() -> String {
        serde_json::to_string_pretty(&Chains::new()).unwrap()
    }

    fn json_schema() -> String {
        serde_json::to_string_pretty(&schemars::schema_for!(Chains)).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "no fs")]
    fn spec_up_to_date() {
        ensure_file_contents(Path::new(JSON_PATH), &json_chains());
    }

    #[test]
    #[cfg_attr(miri, ignore = "no fs")]
    fn schema_up_to_date() {
        ensure_file_contents(Path::new(SCHEMA_PATH), &json_schema());
    }

    /// Checks that the `file` has the specified `contents`. If that is not the
    /// case, updates the file and then fails the test.
    fn ensure_file_contents(file: &Path, contents: &str) {
        if let Ok(old_contents) = fs::read_to_string(file) {
            if normalize_newlines(&old_contents) == normalize_newlines(contents) {
                // File is already up to date.
                return;
            }
        }

        eprintln!("\n\x1b[31;1merror\x1b[0m: {} was not up-to-date, updating\n", file.display());
        if std::env::var("CI").is_ok() {
            eprintln!(
                "    NOTE: run `cargo test --all-features` locally and commit the updated files\n"
            );
        }
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(file, contents).unwrap();
        panic!("some file was not up to date and has been updated, simply re-run the tests");
    }

    fn normalize_newlines(s: &str) -> String {
        s.replace("\r\n", "\n")
    }
}
