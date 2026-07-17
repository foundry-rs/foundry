use alloy_primitives::{Bytes, U256, utils::parse_ether};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Represents the options used in `anvil_reorg`
#[derive(Debug, Clone, Deserialize)]
pub struct ReorgOptions {
    // The depth of the reorg
    pub depth: u64,
    // List of transaction requests and blocks pairs to be mined into the new chain
    pub tx_block_pairs: Vec<(TransactionData, u64)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[expect(clippy::large_enum_variant)]
pub enum TransactionData {
    JSON(TransactionRequest),
    Raw(Bytes),
}

/// Options for `anvil_setActivity`, also used as the node's activity simulation config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityOptions {
    /// Number of transactions injected per block.
    pub txs: ActivityRange<u64>,
    /// Percentage of transactions that revert.
    pub reverted: u8,
    /// Percentage of transactions left pending indefinitely.
    pub pending: u8,
    /// Number of events emitted per contract-call transaction.
    pub logs: ActivityRange<u64>,
    /// Transfer value range in wei.
    pub value: ActivityRange<U256>,
    /// Enabled transaction kinds. `None` selects a network-appropriate default.
    pub mix: Option<Vec<ActivityKind>>,
    /// RNG seed for deterministic activity.
    pub seed: Option<u64>,
}

impl Default for ActivityOptions {
    fn default() -> Self {
        Self {
            txs: ActivityRange { min: 3, max: 8 },
            reverted: 10,
            pending: 5,
            logs: ActivityRange { min: 0, max: 3 },
            value: ActivityRange {
                min: parse_ether("0.0001").expect("valid ether"),
                max: parse_ether("0.1").expect("valid ether"),
            },
            mix: None,
            seed: None,
        }
    }
}

/// Inclusive `min..=max` range for activity knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRange<T> {
    pub min: T,
    pub max: T,
}

impl<T: FromStr + Copy + PartialOrd> ActivityRange<T> {
    /// Parses `N` or `N-M`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parse = |s: &str| s.trim().parse::<T>().map_err(|_| format!("invalid value: `{s}`"));
        let (min, max) = match s.split_once('-') {
            Some((min, max)) => (parse(min)?, parse(max)?),
            None => {
                let value = parse(s)?;
                (value, value)
            }
        };
        if min > max {
            return Err(format!("invalid range: `{s}`"));
        }
        Ok(Self { min, max })
    }
}

/// A kind of generated activity transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    /// Plain native transfers between dev accounts.
    Transfer,
    /// Activity contract calls (events, storage churn, gas burn).
    Contract,
    /// Mock ERC20 transfer/approve traffic.
    Erc20,
    /// Storage-write heavy activity contract calls.
    State,
    /// TIP-20 precompile activity (Tempo networks).
    Tip20,
}

impl FromStr for ActivityKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "transfer" => Ok(Self::Transfer),
            "contract" => Ok(Self::Contract),
            "erc20" => Ok(Self::Erc20),
            "state" => Ok(Self::State),
            "tip20" => Ok(Self::Tip20),
            other => Err(format!("unknown activity kind: `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_activity_range() {
        assert_eq!(ActivityRange::<u64>::parse("5").unwrap(), ActivityRange { min: 5, max: 5 });
        assert_eq!(ActivityRange::<u64>::parse("3-8").unwrap(), ActivityRange { min: 3, max: 8 });
        assert!(ActivityRange::<u64>::parse("8-3").is_err());
        assert!(ActivityRange::<u64>::parse("abc").is_err());
    }

    #[test]
    fn activity_options_serde_roundtrip() {
        let options = ActivityOptions { seed: Some(1), ..Default::default() };
        let json = serde_json::to_string(&options).unwrap();
        assert_eq!(serde_json::from_str::<ActivityOptions>(&json).unwrap(), options);
        // Partial objects fall back to defaults.
        let partial: ActivityOptions =
            serde_json::from_str(r#"{"txs":{"min":1,"max":2},"reverted":50}"#).unwrap();
        assert_eq!(partial.txs, ActivityRange { min: 1, max: 2 });
        assert_eq!(partial.reverted, 50);
        assert_eq!(partial.pending, ActivityOptions::default().pending);
    }
}
