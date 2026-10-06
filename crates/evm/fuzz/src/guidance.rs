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
//! Dictionary entries are sampled through the state-based input strategy. A guided word is selected
//! for a parameter about 20% of the time with the default fuzz dictionary weight of 40, and about
//! 40% with the default invariant dictionary weight of 80. Setting the dictionary weight to 0
//! disables guided words. Selector weights bias invariant target function selection; unlisted
//! functions keep weight 1 and weight 0 excludes a function.

use alloy_dyn_abi::DynSolType;
use alloy_json_abi::Function;
use alloy_primitives::{
    B256, I256, Selector, U256,
    map::{B256IndexSet, HashMap},
};
use eyre::{Result, WrapErr, bail};
use foundry_common::{fs, sh_warn};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

/// Supported guidance file format version.
pub const FUZZ_GUIDANCE_VERSION: u32 = 1;

/// Parsed guidance for fuzz and invariant campaigns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuzzGuidance {
    /// Words sampled as numeric fuzz inputs. They are never removed between runs.
    dictionary: B256IndexSet,
    /// Words sampled as fixed-byte fuzz inputs.
    fixed_bytes_dictionary: B256IndexSet,
    /// Invariant target function weights.
    selector_weights: SelectorWeights,
}

impl FuzzGuidance {
    /// Loads and validates a guidance file.
    pub fn load(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path)?;
        let (guidance, warnings) = Self::parse(&json)
            .wrap_err_with(|| format!("invalid fuzz guidance file {}", path.display()))?;
        for warning in warnings {
            sh_warn!(
                "ignoring fuzz guidance selector weight `{}` in {}: {}",
                warning.key,
                path.display(),
                warning.error
            )?;
        }
        Ok(guidance)
    }

    fn parse(json: &str) -> Result<(Self, Vec<SelectorWeightWarning>)> {
        let raw = serde_json::from_str::<RawFuzzGuidance>(json)?;
        if raw.version != FUZZ_GUIDANCE_VERSION {
            bail!(
                "unsupported fuzz guidance version {}, expected {FUZZ_GUIDANCE_VERSION}",
                raw.version
            );
        }

        let mut dictionary = B256IndexSet::default();
        let mut fixed_bytes_dictionary = B256IndexSet::default();
        for value in raw.dictionary {
            let (word, fixed_bytes) = parse_word(&value)
                .wrap_err_with(|| format!("invalid dictionary value `{value}`"))?;
            dictionary.insert(word);
            fixed_bytes_dictionary.insert(fixed_bytes);
        }

        let mut selector_weights = SelectorWeights::default();
        let mut warnings = Vec::new();
        for (key, weight) in raw.selector_weights {
            match SelectorWeight::parse(&key, weight) {
                Ok(selector_weight) => selector_weights.insert(selector_weight)?,
                Err(error) => {
                    warnings.push(SelectorWeightWarning { key, error: error.to_string() })
                }
            }
        }

        Ok((Self { dictionary, fixed_bytes_dictionary, selector_weights }, warnings))
    }

    #[cfg(test)]
    pub(crate) fn from_json(json: &str) -> Result<Self> {
        Self::parse(json).map(|(guidance, _)| guidance)
    }

    /// Returns whether the guidance has no effect on a campaign.
    pub fn is_empty(&self) -> bool {
        self.dictionary.is_empty() && self.selector_weights.is_empty()
    }

    pub(crate) fn has_dictionary(&self) -> bool {
        !self.dictionary.is_empty()
    }

    pub(crate) const fn dictionary_for(&self, param: &DynSolType) -> &B256IndexSet {
        if matches!(param, DynSolType::FixedBytes(_)) {
            &self.fixed_bytes_dictionary
        } else {
            &self.dictionary
        }
    }

    pub(crate) fn has_selector_weights(&self) -> bool {
        !self.selector_weights.is_empty()
    }

    /// Returns the weight of `function` on the target contract `identifier`, if any key matches.
    ///
    /// `identifier` may be a contract name or a `path:Name` artifact identifier.
    pub(crate) fn selector_weight(&self, identifier: &str, function: &Function) -> Option<u32> {
        self.selector_weights.get(identifier, function.selector())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SelectorWeights {
    contracts: HashMap<String, HashMap<Selector, u32>>,
    unqualified: HashMap<Selector, u32>,
}

impl SelectorWeights {
    fn insert(&mut self, selector_weight: SelectorWeight) -> Result<()> {
        let weights = match selector_weight.contract {
            Some(contract) => self.contracts.entry(contract).or_default(),
            None => &mut self.unqualified,
        };
        if let Some(previous) = weights.insert(selector_weight.selector, selector_weight.weight)
            && previous != selector_weight.weight
        {
            bail!("conflicting weights for selector {}", selector_weight.selector);
        }
        Ok(())
    }

    fn get(&self, identifier: &str, selector: Selector) -> Option<u32> {
        self.contracts
            .get(identifier)
            .and_then(|weights| weights.get(&selector))
            .or_else(|| {
                identifier
                    .rsplit_once(':')
                    .and_then(|(_, name)| self.contracts.get(name))
                    .and_then(|weights| weights.get(&selector))
            })
            .or_else(|| self.unqualified.get(&selector))
            .copied()
    }

    fn is_empty(&self) -> bool {
        self.contracts.is_empty() && self.unqualified.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectorWeight {
    contract: Option<String>,
    selector: Selector,
    weight: u32,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectorWeightWarning {
    key: String,
    error: String,
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

/// Parses a hex, decimal or negative decimal value into numeric and fixed-byte words.
///
/// Numeric hex values are left-padded. The fixed-byte representation keeps the written bytes at
/// the start of the word, as required by ABI `bytesN` values. Negative values use two's
/// complement in both representations.
fn parse_word(value: &str) -> Result<(B256, B256)> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        if hex.is_empty() || hex.len() > 64 {
            bail!("hex values must have between 1 and 64 digits");
        }
        let word = U256::from_str_radix(hex, 16)?.into();
        let bytes = alloy_primitives::hex::decode(if hex.len() % 2 == 0 {
            hex.to_string()
        } else {
            format!("0{hex}")
        })?;
        return Ok((word, B256::right_padding_from(&bytes)));
    }
    if value.is_empty() || !value.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) {
        bail!("expected a hex value or a decimal integer");
    }
    if value.starts_with('-') {
        let word = I256::from_dec_str(value)?.into_raw().into();
        return Ok((word, word));
    }
    let word = U256::from_str_radix(value, 10)?.into();
    Ok((word, word))
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
            guidance.dictionary_for(&DynSolType::Uint(256)).as_slice(),
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
        assert_eq!(
            guidance.dictionary_for(&DynSolType::FixedBytes(4))[0],
            B256::right_padding_from(&[0xde, 0xad, 0xbe, 0xef])
        );
        assert!(!guidance.has_selector_weights());
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
        let (guidance, warnings) = FuzzGuidance::parse(
            r#"{
                "version": 1,
                "selector_weights": {
                    "0x2e1a7d4d": 5,
                    "Vault.withdraw(uint256)": 10,
                    "src/Vault.sol:Vault.withdraw(uint256)": 0,
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

        assert_eq!(warnings.len(), 4);
        assert_eq!(guidance.selector_weight("Vault", &withdraw), Some(10));
        assert_eq!(guidance.selector_weight("src/Vault.sol:Vault", &withdraw), Some(0));
        assert_eq!(guidance.selector_weight("Other", &withdraw), Some(5));
        assert_eq!(guidance.selector_weight("src/Vault.sol:Vault", &deposit), Some(3));
        assert_eq!(guidance.selector_weight("Vault", &deposit), None);
        assert_eq!(guidance.selector_weight("Vault", &pause), Some(0));
        assert_eq!(guidance.selector_weight("Vault", &other), None);
    }

    #[test]
    fn rejects_conflicting_selector_aliases() {
        let err = FuzzGuidance::from_json(
            r#"{
                "version": 1,
                "selector_weights": {
                    "0x2e1a7d4d": 1,
                    "withdraw(uint256)": 2
                }
            }"#,
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "conflicting weights for selector 0x2e1a7d4d");
    }
}
