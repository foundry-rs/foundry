use crate::block_number::BlockNumber;
use alloy_primitives::{U256, U64};
use serde::{Deserialize, Deserializer};
use std::str::FromStr;

/// Helper type to parse numeric strings, `u64` and `U256`
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StringifiedNumeric {
    String(String),
    U256(U256),
    Num(serde_json::Number),
}

impl TryFrom<StringifiedNumeric> for U256 {
    type Error = String;

    fn try_from(value: StringifiedNumeric) -> Result<Self, Self::Error> {
        match value {
            StringifiedNumeric::U256(n) => Ok(n),
            StringifiedNumeric::Num(n) => {
                Ok(U256::from_str(&n.to_string()).map_err(|err| err.to_string())?)
            }
            StringifiedNumeric::String(s) => {
                if let Ok(val) = s.parse::<u128>() {
                    Ok(U256::from(val))
                } else if s.starts_with("0x") {
                    U256::from_str_radix(&s, 16).map_err(|err| err.to_string())
                } else {
                    U256::from_str(&s).map_err(|err| err.to_string())
                }
            }
        }
    }
}

impl TryFrom<StringifiedNumeric> for U64 {
    type Error = String;

    fn try_from(value: StringifiedNumeric) -> Result<Self, Self::Error> {
        let value = U256::try_from(value)?;
        Ok(value.wrapping_to::<U64>())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BoolOrU64 {
    #[serde(deserialize_with = "deserialize_stringified_u64")]
    U64(u64),
    Bool(String),
}

/// Supports parsing either a u64 or a boolean (which will then be converted to u64)
///
/// Implemented to binary fields such as "OptimizationUsed" which are formatted either as 0/1 or
/// "true/"false" by different block explorers (e.g. etherscan vs blockscout)
pub fn deserialize_stringified_bool_or_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let num = BoolOrU64::deserialize(deserializer)?;
    match num {
        BoolOrU64::Bool(b) => {
            let b = b.parse::<bool>().map_err(serde::de::Error::custom)?;
            let u = if b { 1 } else { 0 };
            Ok(u)
        }
        BoolOrU64::U64(u) => Ok(u),
    }
}

/// Supports parsing u64
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let num = StringifiedNumeric::deserialize(deserializer)?;
    let num: U256 = num.try_into().map_err(serde::de::Error::custom)?;
    num.try_into().map_err(serde::de::Error::custom)
}

/// Supports parsing numbers as strings
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_numeric<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    let num = StringifiedNumeric::deserialize(deserializer)?;
    num.try_into().map_err(serde::de::Error::custom)
}

/// Supports parsing numbers as strings
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_numeric_opt<'de, D>(
    deserializer: D,
) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    if let Some(num) = Option::<StringifiedNumeric>::deserialize(deserializer)? {
        num.try_into().map(Some).map_err(serde::de::Error::custom)
    } else {
        Ok(None)
    }
}

/// Supports parsing u64
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_u64_opt<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    if let Some(num) = Option::<StringifiedNumeric>::deserialize(deserializer)? {
        let num: U256 = num.try_into().map_err(serde::de::Error::custom)?;
        let num: u64 = num.try_into().map_err(serde::de::Error::custom)?;
        Ok(Some(num))
    } else {
        Ok(None)
    }
}

/// Helper type to parse numeric strings, `u64` and `U256`
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StringifiedBlockNumber {
    Numeric(StringifiedNumeric),
    BlockNumber(BlockNumber),
}

impl TryFrom<StringifiedBlockNumber> for BlockNumber {
    type Error = String;

    fn try_from(value: StringifiedBlockNumber) -> Result<Self, Self::Error> {
        match value {
            StringifiedBlockNumber::Numeric(num) => {
                let num = U256::try_from(num)
                    .and_then(|num| u64::try_from(num).map_err(|e| e.to_string()))?;
                Ok(BlockNumber::Number(U64::from(num)))
            }
            StringifiedBlockNumber::BlockNumber(b) => Ok(b),
        }
    }
}

/// Supports parsing block number as strings
///
/// See <https://github.com/gakonst/ethers-rs/issues/1507>
pub fn deserialize_stringified_block_number<'de, D>(
    deserializer: D,
) -> Result<BlockNumber, D::Error>
where
    D: Deserializer<'de>,
{
    let num = StringifiedBlockNumber::deserialize(deserializer)?;
    num.try_into().map_err(serde::de::Error::custom)
}
