//! External guidance for fuzz and invariant campaigns.
//!
//! A guidance file lets external tooling, such as a test generator that has read the source, steer
//! a campaign without editing the tests. The file is JSON with a mandatory format version:
//!
//! ```json
//! {
//!   "version": 1,
//!   "dictionary": ["0xdeadbeef", "1000000000000000000", "-1"],
//!   "selector_weights": { "Vault.withdraw(uint256)": 10, "0x2e1a7d4d": 5, "pause()": 0 }
//! }
//! ```
//!
//! Dictionary entries are sampled as fuzz inputs for the whole campaign. Selector weights bias
//! invariant target function selection; unlisted functions keep weight 1 and weight 0 excludes a
//! function.

use alloy_json_abi::Function;
use alloy_primitives::{B256, I256, Selector, U256, map::B256IndexSet};
use eyre::{Result, WrapErr, bail};
use foundry_common::{fs, sh_warn};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

/// Supported guidance file format version.
pub const FUZZ_GUIDANCE_VERSION: u32 = 1;

/// Parsed guidance for fuzz and invariant campaigns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuzzGuidance {
    /// Words sampled as fuzz inputs. They are never removed between runs.
    dictionary: B256IndexSet,
    /// Invariant target function weights, contract-qualified entries first.
    selector_weights: Vec<SelectorWeight>,
}

impl FuzzGuidance {
    /// Loads and validates a guidance file.
    pub fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)?;
        Self::from_json(&json)
            .wrap_err_with(|| format!("invalid fuzz guidance file {}", path.display()))
    }

    /// Parses and validates guidance JSON.
    ///
    /// Invalid versions and dictionary values are errors. Malformed selector weight keys are
    /// reported as warnings and ignored.
    pub fn from_json(json: &str) -> Result<Self> {
        let raw = serde_json::from_str::<RawFuzzGuidance>(json)?;
        if raw.version != FUZZ_GUIDANCE_VERSION {
            bail!(
                "unsupported fuzz guidance version {}, expected {FUZZ_GUIDANCE_VERSION}",
                raw.version
            );
        }

        let dictionary = raw
            .dictionary
            .iter()
            .map(|value| {
                parse_word(value).wrap_err_with(|| format!("invalid dictionary value `{value}`"))
            })
            .collect::<Result<_>>()?;

        let mut selector_weights = Vec::new();
        for (key, weight) in raw.selector_weights {
            match SelectorWeight::parse(&key, weight) {
                Ok(selector_weight) => selector_weights.push(selector_weight),
                Err(err) => {
                    let _ = sh_warn!("ignoring fuzz guidance selector weight `{key}`: {err}");
                }
            }
        }
        // Contract-qualified keys take precedence over bare signatures and selectors.
        selector_weights.sort_by_key(|selector_weight| selector_weight.contract.is_none());

        Ok(Self { dictionary, selector_weights })
    }

    /// Returns whether the guidance has no effect on a campaign.
    pub fn is_empty(&self) -> bool {
        self.dictionary.is_empty() && self.selector_weights.is_empty()
    }

    /// Returns the guidance dictionary words.
    pub const fn dictionary(&self) -> &B256IndexSet {
        &self.dictionary
    }

    /// Returns the parsed selector weights.
    pub fn selector_weights(&self) -> &[SelectorWeight] {
        &self.selector_weights
    }

    /// Returns the weight of `function` on the target contract `identifier`, if any key matches.
    ///
    /// `identifier` may be a contract name or a `path:Name` artifact identifier.
    pub fn selector_weight(&self, identifier: &str, function: &Function) -> Option<u32> {
        let selector = function.selector();
        self.selector_weights
            .iter()
            .find(|selector_weight| {
                selector_weight.selector == selector && selector_weight.matches_contract(identifier)
            })
            .map(|selector_weight| selector_weight.weight)
    }
}

/// Invariant target function weight parsed from a guidance selector key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorWeight {
    /// Contract name or `path:Name` identifier the weight is restricted to.
    pub contract: Option<String>,
    /// Function selector.
    pub selector: Selector,
    /// Relative selection weight.
    pub weight: u32,
}

impl SelectorWeight {
    /// Parses a `0x12345678`, `signature(types)` or `Contract.signature(types)` key.
    fn parse(key: &str, weight: u32) -> Result<Self> {
        let key = key.trim();
        let Some(paren) = key.find('(') else {
            let Some(hex) = key.strip_prefix("0x") else {
                bail!(
                    "expected a 4-byte selector, `signature(types)` or `Contract.signature(types)`"
                );
            };
            let selector = hex.parse::<Selector>().wrap_err("invalid 4-byte selector")?;
            return Ok(Self { contract: None, selector, weight });
        };

        let (contract, signature) = match key[..paren].rsplit_once('.') {
            Some(("", _)) => bail!("empty contract name"),
            Some((contract, _)) => (Some(contract.to_string()), &key[contract.len() + 1..]),
            None => (None, key),
        };
        let selector =
            Function::parse(signature).wrap_err("invalid function signature")?.selector();
        Ok(Self { contract, selector, weight })
    }

    fn matches_contract(&self, identifier: &str) -> bool {
        self.contract.as_deref().is_none_or(|contract| {
            contract == identifier || identifier.rsplit(':').next() == Some(contract)
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFuzzGuidance {
    version: u32,
    #[serde(default)]
    dictionary: Vec<String>,
    #[serde(default)]
    selector_weights: BTreeMap<String, u32>,
}

/// Parses a hex, decimal or negative decimal value into a 32-byte word.
///
/// Hex values are left-padded and negative values use two's complement.
fn parse_word(value: &str) -> Result<B256> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        if hex.is_empty() || hex.len() > 64 {
            bail!("hex values must have between 1 and 64 digits");
        }
        return Ok(U256::from_str_radix(hex, 16)?.into());
    }
    if value.is_empty() || !value.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) {
        bail!("expected a hex value or a decimal integer");
    }
    if value.starts_with('-') {
        return Ok(I256::from_dec_str(value)?.into_raw().into());
    }
    Ok(U256::from_str_radix(value, 10)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(value: U256) -> B256 {
        value.into()
    }

    #[test]
    fn parses_dictionary_values() {
        let guidance = FuzzGuidance::from_json(
            r#"{
                "version": 1,
                "dictionary": [
                    "0xdeadbeef",
                    "0X0000000000000000000000000000000000000000000000000000000000000001",
                    "1000000000000000000",
                    "-1",
                    "-2",
                    "0x1234567890123456789012345678901234567890"
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            guidance.dictionary().as_slice(),
            &[
                word(U256::from(0xdeadbeef_u64)),
                word(U256::from(1)),
                word(U256::from(10).pow(U256::from(18))),
                B256::repeat_byte(0xff),
                word(U256::MAX - U256::from(1)),
                "0x1234567890123456789012345678901234567890"
                    .parse::<alloy_primitives::Address>()
                    .unwrap()
                    .into_word(),
            ]
        );
        assert!(guidance.selector_weights().is_empty());
    }

    #[test]
    fn rejects_invalid_files() {
        for (json, expected) in [
            (r#"{"version": 2}"#, "unsupported fuzz guidance version 2, expected 1"),
            (r#"{"dictionary": []}"#, "missing field `version` at line 1 column 18"),
            (
                r#"{"version": 1, "senders": []}"#,
                "unknown field `senders`, expected one of `version`, `dictionary`, `selector_weights` at line 1 column 24",
            ),
            (r#"{"version": 1, "dictionary": ["0x"]}"#, "invalid dictionary value `0x`"),
            (r#"{"version": 1, "dictionary": ["1.5"]}"#, "invalid dictionary value `1.5`"),
            (r#"{"version": 1, "dictionary": ["abc"]}"#, "invalid dictionary value `abc`"),
            (
                r#"{"version": 1, "dictionary": ["0x10000000000000000000000000000000000000000000000000000000000000000"]}"#,
                "invalid dictionary value `0x10000000000000000000000000000000000000000000000000000000000000000`",
            ),
            (
                r#"{"version": 1, "dictionary": ["115792089237316195423570985008687907853269984665640564039457584007913129639936"]}"#,
                "invalid dictionary value `115792089237316195423570985008687907853269984665640564039457584007913129639936`",
            ),
        ] {
            assert_eq!(FuzzGuidance::from_json(json).unwrap_err().to_string(), expected);
        }
    }

    #[test]
    fn parses_selector_weights() {
        let guidance = FuzzGuidance::from_json(
            r#"{
                "version": 1,
                "selector_weights": {
                    "0x2e1a7d4d": 5,
                    "Vault.withdraw(uint256)": 10,
                    "src/Vault.sol:Vault.deposit(uint)": 3,
                    "pause()": 0,
                    "withdraw": 7,
                    "0x1234": 7,
                    ".pause()": 7,
                    "Vault.withdraw(uint256": 7
                }
            }"#,
        )
        .unwrap();

        let withdraw = Function::parse("withdraw(uint256)").unwrap();
        let deposit = Function::parse("deposit(uint256)").unwrap();
        let pause = Function::parse("pause()").unwrap();
        let other = Function::parse("other()").unwrap();

        assert_eq!(guidance.selector_weights().len(), 4);
        assert_eq!(guidance.selector_weight("Vault", &withdraw), Some(10));
        assert_eq!(guidance.selector_weight("src/Vault.sol:Vault", &withdraw), Some(10));
        assert_eq!(guidance.selector_weight("Other", &withdraw), Some(5));
        assert_eq!(guidance.selector_weight("src/Vault.sol:Vault", &deposit), Some(3));
        assert_eq!(guidance.selector_weight("Vault", &deposit), None);
        assert_eq!(guidance.selector_weight("Vault", &pause), Some(0));
        assert_eq!(guidance.selector_weight("Vault", &other), None);
    }
}
