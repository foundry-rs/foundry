//! Misc Serde helpers for foundry crates.

use alloy_primitives::U256;
use serde::{de, Deserialize, Deserializer};
use std::str::FromStr;

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
    fn from(n: Numeric) -> Self {
        match n {
            Numeric::U256(n) => n,
            Numeric::Num(n) => Self::from(n),
        }
    }
}

impl FromStr for Numeric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(val) = s.parse::<u128>() {
            Ok(Self::U256(U256::from(val)))
        } else if s.starts_with("0x") {
            U256::from_str_radix(s, 16).map(Numeric::U256).map_err(|err| err.to_string())
        } else {
            U256::from_str(s).map(Numeric::U256).map_err(|err| err.to_string())
        }
    }
}

/// Deserializes the input into an `Option<U256>`, using [`from_int_or_hex`] to deserialize the
/// inner value.
pub fn from_int_or_hex_opt<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<NumberOrHexU256>::deserialize(deserializer)? {
        Some(val) => val.try_into_u256().map(Some),
        None => Ok(None),
    }
}

/// An enum that represents either a [serde_json::Number] integer, or a hex [U256].
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NumberOrHexU256 {
    /// An integer
    Int(serde_json::Number),
    /// A hex U256
    Hex(U256),
}

impl NumberOrHexU256 {
    /// Tries to convert this into a [U256]].
    pub fn try_into_u256<E: de::Error>(self) -> Result<U256, E> {
        match self {
            Self::Int(num) => U256::from_str(num.to_string().as_str()).map_err(E::custom),
            Self::Hex(val) => Ok(val),
        }
    }
}

/// Deserializes the input into a U256, accepting both 0x-prefixed hex and decimal strings with
/// arbitrary precision, defined by serde_json's [`Number`](serde_json::Number).
pub fn from_int_or_hex<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    NumberOrHexU256::deserialize(deserializer)?.try_into_u256()
}

/// Helper type to deserialize sequence of numbers
#[derive(Deserialize)]
#[serde(untagged)]
pub enum NumericSeq {
    /// Single parameter sequence (e.g `[1]`).
    Seq([Numeric; 1]),
    /// `U256`.
    U256(U256),
    /// Native `u64`.
    Num(u64),
}

/// Deserializes a number from hex or int
pub fn deserialize_number<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    Numeric::deserialize(deserializer).map(Into::into)
}

/// Deserializes a number from hex or int, but optionally
pub fn deserialize_number_opt<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    let num = match Option::<Numeric>::deserialize(deserializer)? {
        Some(Numeric::U256(n)) => Some(n),
        Some(Numeric::Num(n)) => Some(U256::from(n)),
        _ => None,
    };

    Ok(num)
}

/// Deserializes single integer params: `1, [1], ["0x01"]`
pub fn deserialize_number_seq<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let num = match NumericSeq::deserialize(deserializer)? {
        NumericSeq::Seq(seq) => seq[0].into(),
        NumericSeq::U256(n) => n,
        NumericSeq::Num(n) => U256::from(n),
    };

    Ok(num)
}
