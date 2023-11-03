//! Misc serde helpers for foundry crates.

use std::str::FromStr;

use alloy_primitives::U256;
use serde::Deserialize;

/// Helper type to parse both `u64` and `U256`
#[derive(Copy, Clone, Deserialize)]
#[serde(untagged)]
pub enum Numeric {
    /// A [U256] value.
    U256(U256),
    /// A `u64` value.
    Num(u64),
}

impl From<Numeric> for U256 {
    fn from(n: Numeric) -> U256 {
        match n {
            Numeric::U256(n) => n,
            Numeric::Num(n) => U256::from(n),
        }
    }
}

impl FromStr for Numeric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(val) = s.parse::<u128>() {
            Ok(Numeric::U256(U256::from(val)))
        } else if s.starts_with("0x") {
            U256::from_str_radix(s, 16).map(Numeric::U256).map_err(|err| err.to_string())
        } else {
            U256::from_str(s).map(Numeric::U256).map_err(|err| err.to_string())
        }
    }
}
