//! EIP-1559 fee estimation configuration.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Controls how EIP-1559 fees are estimated, analogous to wallet UIs that offer
/// "low" / "market" / "aggressive" gas options.
///
/// Used both as a CLI value (`--estimate <preset>`) and as a TOML
/// configuration value (`eip1559_fee_estimate = "market"`).
///
/// The estimate controls two parameters:
/// - the `eth_feeHistory` reward percentile used to derive the priority fee, and
/// - the multiplier applied to the base fee when building `maxFeePerGas`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Eip1559FeeEstimatePreset {
    /// Lower priority-fee percentile (10th) and a tighter base-fee buffer
    /// (`base_fee * 1.5`). This only lowers the tip estimate and the max-fee cap,
    /// not the base fee that is actually paid, and risks the transaction stalling
    /// if the base fee rises.
    Low,
    /// Default: `base_fee * 2` plus a 20th-percentile priority fee.
    #[default]
    Market,
    /// Higher priority-fee percentile (50th) to bid a larger tip for faster
    /// inclusion.
    Aggressive,
}

impl Eip1559FeeEstimatePreset {
    /// The reward percentile sampled from `eth_feeHistory` to estimate the
    /// priority fee.
    pub const fn reward_percentile(&self) -> f64 {
        match self {
            Self::Low => 10.0,
            Self::Market => 20.0,
            Self::Aggressive => 50.0,
        }
    }

    /// The base-fee multiplier, expressed as `(numerator, denominator)`, applied
    /// when building `maxFeePerGas` so the transaction stays includable if the
    /// base fee rises before it is mined.
    pub const fn base_fee_multiplier(&self) -> (u128, u128) {
        match self {
            Self::Low => (3, 2),
            Self::Market => (2, 1),
            Self::Aggressive => (2, 1),
        }
    }
}

impl fmt::Display for Eip1559FeeEstimatePreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Low => "low",
            Self::Market => "market",
            Self::Aggressive => "aggressive",
        };
        f.write_str(s)
    }
}

impl FromStr for Eip1559FeeEstimatePreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "market" => Ok(Self::Market),
            "aggressive" => Ok(Self::Aggressive),
            other => Err(format!(
                "invalid EIP-1559 fee estimate preset: {other} (expected one of: low, market, aggressive)"
            )),
        }
    }
}
