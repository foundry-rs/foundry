//! Misc Serde helpers for foundry crates.

use alloy_primitives::{U64, U256};
use serde::{Deserialize, Deserializer, de};
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

/// Helper type to parse a `u64` from a JSON number or a hex/decimal string.
#[derive(Copy, Clone, Deserialize)]
#[serde(untagged)]
pub enum Numeric64 {
    /// A JSON number.
    Num(u64),
    /// A hex or decimal string.
    U64(U64),
}

impl From<Numeric64> for u64 {
    fn from(n: Numeric64) -> Self {
        match n {
            Numeric64::Num(n) => n,
            Numeric64::U64(n) => n.to::<Self>(),
        }
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
            Self::Int(num) => U256::from_str(&num.to_string()).map_err(E::custom),
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

/// Helper type to deserialize a single `u64` from either a direct value or a one-element sequence.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Numeric64ValueOrSeq {
    /// Single parameter sequence (e.g `[1]`).
    Seq([Numeric64; 1]),
    /// Single value.
    Value(Numeric64),
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

/// Deserializes single `u64` params: `1, [1], ["0x01"]`.
pub fn deserialize_u64_seq<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let num = match Numeric64ValueOrSeq::deserialize(deserializer)? {
        Numeric64ValueOrSeq::Seq(seq) => seq[0].into(),
        Numeric64ValueOrSeq::Value(num) => num.into(),
    };

    Ok(num)
}

/// Deserializes an optional integer from a single-element params sequence.
/// Accepts `[]`, `[null]`, `[n]`, `["0x.."]`.
pub fn deserialize_u64_seq_opt<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let seq = Vec::<Option<Numeric64>>::deserialize(deserializer)?;
    if seq.len() > 1 {
        return Err(de::Error::custom(format!(
            "expected params sequence with length 0 or 1 but got {}",
            seq.len()
        )));
    }
    Ok(seq.into_iter().next().flatten().map(Into::into))
}

pub mod duration {
    use serde::{Deserialize, Deserializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let d = jiff::SignedDuration::try_from(*duration).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&format!("{d:#}"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let d = s.parse::<jiff::SignedDuration>().map_err(serde::de::Error::custom)?;
        d.try_into().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::IntoDeserializer;
    use serde_json::json;

    fn parse_u64_param(value: serde_json::Value) -> Result<u64, serde_json::Error> {
        deserialize_u64_seq(value.into_deserializer())
    }

    fn parse_optional_u64_param(
        value: serde_json::Value,
    ) -> Result<Option<u64>, serde_json::Error> {
        deserialize_u64_seq_opt(value.into_deserializer())
    }

    #[test]
    fn deserialize_u64_seq_accepts_single_param_sequence_and_direct_value() {
        let valid_values = [
            json!([100]),
            json!(100),
            json!(["0x64"]),
            json!("0x64"),
            json!(["100"]),
            json!("100"),
        ];
        for value in valid_values {
            assert_eq!(parse_u64_param(value).unwrap(), 100);
        }

        for value in [json!([u64::MAX]), json!(u64::MAX)] {
            assert_eq!(parse_u64_param(value).unwrap(), u64::MAX);
        }
    }

    #[test]
    fn deserialize_u64_seq_rejects_invalid_shape_and_overflow() {
        for value in [
            json!([]),
            json!([1, 2]),
            json!([null]),
            json!(null),
            json!(["0x10000000000000000"]),
            json!("0x10000000000000000"),
            json!(["18446744073709551616"]),
            json!("18446744073709551616"),
        ] {
            assert!(parse_u64_param(value).is_err());
        }
    }

    #[test]
    fn deserialize_u64_seq_opt_accepts_empty_null_and_single_param_sequence() {
        for value in [json!([]), json!([null])] {
            assert_eq!(parse_optional_u64_param(value).unwrap(), None);
        }

        for value in [json!([100]), json!(["0x64"]), json!(["100"])] {
            assert_eq!(parse_optional_u64_param(value).unwrap(), Some(100));
        }

        assert_eq!(parse_optional_u64_param(json!([u64::MAX])).unwrap(), Some(u64::MAX));
    }

    #[test]
    fn deserialize_u64_seq_opt_rejects_invalid_shape_and_overflow() {
        for value in [
            json!([1, 2]),
            json!(100),
            json!("0x64"),
            json!([["0x64"]]),
            json!(["0x10000000000000000"]),
            json!(["18446744073709551616"]),
        ] {
            assert!(parse_optional_u64_param(value).is_err());
        }
    }
}
