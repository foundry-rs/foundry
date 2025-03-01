use crate::{contract::SourceCodeMetadata, units::Units, EtherscanError, Result};
use alloy_primitives::{Address, ParseSignedError, I256, U256};
use semver::Version;
use serde::{Deserialize, Deserializer};
use std::{fmt, str::FromStr};
use thiserror::Error;

static SOLC_BIN_LIST_URL: &str = "https://binaries.soliditylang.org/bin/list.txt";

/// Given a Solc [Version], lookup the build metadata and return the full SemVer.
/// e.g. `0.8.13` -> `0.8.13+commit.abaa5c0e`
pub async fn lookup_compiler_version(version: &Version) -> Result<Version> {
    let response = reqwest::get(SOLC_BIN_LIST_URL).await?.text().await?;
    // Ignore extra metadata (`pre` or `build`)
    let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let v = response
        .lines()
        .find(|l| !l.contains("nightly") && l.contains(&version))
        .map(|l| l.trim_start_matches("soljson-v").trim_end_matches(".js"))
        .ok_or_else(|| EtherscanError::MissingSolcVersion(version))?;

    Ok(v.parse().expect("failed to parse semver"))
}

/// Return None if empty, otherwise parse as [Address].
pub fn deserialize_address_opt<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Address>, D::Error> {
    match Option::<String>::deserialize(deserializer)? {
        None => Ok(None),
        Some(s) => match s.is_empty() {
            true => Ok(None),
            _ => Ok(Some(s.parse().map_err(serde::de::Error::custom)?)),
        },
    }
}

/// Deserializes as JSON either:
///
/// - Object: `{ "SourceCode": { language: "Solidity", .. }, ..}`
/// - Stringified JSON object:
///     - `{ "SourceCode": "{{\r\n  \"language\": \"Solidity\", ..}}", ..}`
///     - `{ "SourceCode": "{ \"file.sol\": \"...\" }", ... }`
/// - Normal source code string: `{ "SourceCode": "// SPDX-License-Identifier: ...", .. }`
pub fn deserialize_source_code<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<SourceCodeMetadata, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SourceCode {
        String(String), // this must come first
        Obj(SourceCodeMetadata),
    }
    let s = SourceCode::deserialize(deserializer)?;
    match s {
        SourceCode::String(s) => {
            if s.starts_with('{') && s.ends_with('}') {
                let mut s = s.as_str();
                // skip double braces
                if s.starts_with("{{") && s.ends_with("}}") {
                    s = &s[1..s.len() - 1];
                }
                serde_json::from_str(s).map_err(serde::de::Error::custom)
            } else {
                Ok(SourceCodeMetadata::SourceCode(s))
            }
        }
        SourceCode::Obj(obj) => Ok(obj),
    }
}

/// This enum holds the numeric types that a possible to be returned by `parse_units` and
/// that are taken by `format_units`.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum ParseUnits {
    U256(U256),
    I256(I256),
}

impl From<ParseUnits> for U256 {
    fn from(n: ParseUnits) -> Self {
        match n {
            ParseUnits::U256(n) => n,
            ParseUnits::I256(n) => n.into_raw(),
        }
    }
}

impl From<ParseUnits> for I256 {
    fn from(n: ParseUnits) -> Self {
        match n {
            ParseUnits::I256(n) => n,
            ParseUnits::U256(n) => I256::from_raw(n),
        }
    }
}

impl From<alloy_primitives::Signed<256, 4>> for ParseUnits {
    fn from(n: alloy_primitives::Signed<256, 4>) -> Self {
        Self::I256(n)
    }
}

impl fmt::Display for ParseUnits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseUnits::U256(val) => val.fmt(f),
            ParseUnits::I256(val) => val.fmt(f),
        }
    }
}

macro_rules! construct_format_units_from {
    ($( $t:ty[$convert:ident] ),*) => {
        $(
            impl From<$t> for ParseUnits {
                fn from(num: $t) -> Self {
                    Self::$convert(U256::from(num))
                }
            }
        )*
    }
}

macro_rules! construct_signed_format_units_from {
    ($( $t:ty[$convert:ident] ),*) => {
        $(
            impl From<$t> for ParseUnits {
                fn from(num: $t) -> Self {
                    Self::$convert(I256::from_raw(U256::from(num)))
                }
            }
        )*
    }
}

// Generate the From<T> code for the given numeric types below.
construct_format_units_from! {
    u8[U256], u16[U256], u32[U256], u64[U256], u128[U256], U256[U256], usize[U256]
}

construct_signed_format_units_from! {
    i8[I256], i16[I256], i32[I256], i64[I256], i128[I256], isize[I256]
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Unknown units: {0}")]
    UnrecognizedUnits(String),
    #[error("bytes32 strings must not exceed 32 bytes in length")]
    TextTooLong,
    #[error(transparent)]
    Utf8Error(#[from] std::str::Utf8Error),
    #[error(transparent)]
    InvalidFloat(#[from] std::num::ParseFloatError),
    #[error("Invalid decimal string: {0}")]
    FromDecStrError(String),
    #[error("Overflow parsing string")]
    ParseOverflow,
    #[error("Parse Signed Error")]
    ParseI256Error(#[from] ParseSignedError),
    #[error("Invalid address checksum")]
    InvalidAddressChecksum,
    #[error(transparent)]
    FromHexError(<Address as std::str::FromStr>::Err),
}

/// Multiplies the provided amount with 10^{units} provided.
pub fn parse_units<K, S>(amount: S, units: K) -> Result<ParseUnits, ConversionError>
where
    S: ToString,
    K: TryInto<Units, Error = ConversionError> + Copy,
{
    let exponent: u32 = units.try_into()?.as_num();
    let mut amount_str = amount.to_string().replace('_', "");
    let negative = amount_str.chars().next().unwrap_or_default() == '-';
    let dec_len = if let Some(di) = amount_str.find('.') {
        amount_str.remove(di);
        amount_str[di..].len() as u32
    } else {
        0
    };

    if dec_len > exponent {
        // Truncate the decimal part if it is longer than the exponent
        let amount_str = &amount_str[..(amount_str.len() - (dec_len - exponent) as usize)];
        if negative {
            // Edge case: We have removed the entire number and only the negative sign is left.
            //            Return 0 as a I256 given the input was signed.
            if amount_str == "-" {
                Ok(ParseUnits::I256(I256::ZERO))
            } else {
                Ok(ParseUnits::I256(
                    I256::from_dec_str(amount_str)
                        .map_err(|e| ConversionError::FromDecStrError(e.to_string()))?,
                ))
            }
        } else {
            Ok(ParseUnits::U256(
                U256::from_str(amount_str)
                    .map_err(|e| ConversionError::FromDecStrError(e.to_string()))?,
            ))
        }
    } else if negative {
        // Edge case: Only a negative sign was given, return 0 as a I256 given the input was signed.
        if amount_str == "-" {
            Ok(ParseUnits::I256(I256::ZERO))
        } else {
            let _fi = U256::from(10_i64);
            let mut n = I256::from_str(&amount_str)?;
            n *= I256::from_raw(U256::from(10))
                .checked_pow(U256::from(exponent) - U256::from(dec_len))
                .ok_or(ConversionError::ParseOverflow)?;
            Ok(ParseUnits::I256(n))
        }
    } else {
        let mut a_uint = U256::from_str(&amount_str)
            .map_err(|e| ConversionError::FromDecStrError(e.to_string()))?;
        a_uint *= U256::from(10)
            .checked_pow(U256::from(exponent - dec_len))
            .ok_or(ConversionError::ParseOverflow)?;
        Ok(ParseUnits::U256(a_uint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::SourceCodeLanguage;

    #[test]
    fn can_deserialize_address_opt() {
        #[derive(serde::Serialize, Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_address_opt")]
            address: Option<Address>,
        }

        // https://api.etherscan.io/api?module=contract&action=getsourcecode&address=0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413
        let json = r#"{"address":""}"#;
        let de: Test = serde_json::from_str(json).unwrap();
        assert_eq!(de.address, None);

        // Round-trip the above
        let json = serde_json::to_string(&de).unwrap();
        let de: Test = serde_json::from_str(&json).unwrap();
        assert_eq!(de.address, None);

        // https://api.etherscan.io/api?module=contract&action=getsourcecode&address=0xDef1C0ded9bec7F1a1670819833240f027b25EfF
        let json = r#"{"address":"0x4af649ffde640ceb34b1afaba3e0bb8e9698cb01"}"#;
        let de: Test = serde_json::from_str(json).unwrap();
        let expected = "0x4af649ffde640ceb34b1afaba3e0bb8e9698cb01".parse().unwrap();
        assert_eq!(de.address, Some(expected));
    }

    #[test]
    fn can_deserialize_source_code() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "deserialize_source_code")]
            source_code: SourceCodeMetadata,
        }

        let src = "source code text";

        // Normal JSON
        let json = r#"{
            "source_code": { "language": "Solidity", "sources": { "Contract": { "content": "source code text" } } }
        }"#;
        let de: Test = serde_json::from_str(json).unwrap();
        assert!(matches!(de.source_code.language().unwrap(), SourceCodeLanguage::Solidity));
        assert_eq!(de.source_code.sources().len(), 1);
        assert_eq!(de.source_code.sources().get("Contract").unwrap().content, src);
        #[cfg(feature = "foundry-compilers")]
        assert!(de.source_code.settings().unwrap().is_none());

        // Stringified JSON
        let json = r#"{
            "source_code": "{{ \"language\": \"Solidity\", \"sources\": { \"Contract\": { \"content\": \"source code text\" } } }}"
        }"#;
        let de: Test = serde_json::from_str(json).unwrap();
        assert!(matches!(de.source_code.language().unwrap(), SourceCodeLanguage::Solidity));
        assert_eq!(de.source_code.sources().len(), 1);
        assert_eq!(de.source_code.sources().get("Contract").unwrap().content, src);
        #[cfg(feature = "foundry-compilers")]
        assert!(de.source_code.settings().unwrap().is_none());

        let json = r#"{"source_code": "source code text"}"#;
        let de: Test = serde_json::from_str(json).unwrap();
        assert_eq!(de.source_code.source_code(), src);
    }
}
