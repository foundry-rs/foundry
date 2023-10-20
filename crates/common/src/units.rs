use alloy_primitives::{Address, ParseSignedError, I256, U256};
use std::{convert::TryFrom, fmt, str::FromStr};
use thiserror::Error;

/// I256 overflows for numbers wider than 77 units.
const OVERFLOW_I256_UNITS: usize = 77;
/// U256 overflows for numbers wider than 78 units.
const OVERFLOW_U256_UNITS: usize = 78;

/// Common Ethereum unit types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Units {
    /// Wei is equivalent to 1 wei.
    Wei,
    /// Kwei is equivalent to 1e3 wei.
    Kwei,
    /// Mwei is equivalent to 1e6 wei.
    Mwei,
    /// Gwei is equivalent to 1e9 wei.
    Gwei,
    /// Twei is equivalent to 1e12 wei.
    Twei,
    /// Pwei is equivalent to 1e15 wei.
    Pwei,
    /// Ether is equivalent to 1e18 wei.
    Ether,
    /// Other less frequent unit sizes, equivalent to 1e{0} wei.
    Other(u32),
}

impl fmt::Display for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_num().to_string().as_str())
    }
}

impl TryFrom<u32> for Units {
    type Error = ConversionError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Units::Other(value))
    }
}

impl TryFrom<i32> for Units {
    type Error = ConversionError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(Units::Other(value as u32))
    }
}

impl TryFrom<usize> for Units {
    type Error = ConversionError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Units::Other(value as u32))
    }
}

impl TryFrom<String> for Units {
    type Error = ConversionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

impl<'a> TryFrom<&'a String> for Units {
    type Error = ConversionError;

    fn try_from(value: &'a String) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl TryFrom<&str> for Units {
    type Error = ConversionError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl FromStr for Units {
    type Err = ConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "eth" | "ether" => Units::Ether,
            "pwei" | "milli" | "milliether" | "finney" => Units::Pwei,
            "twei" | "micro" | "microether" | "szabo" => Units::Twei,
            "gwei" | "nano" | "nanoether" | "shannon" => Units::Gwei,
            "mwei" | "pico" | "picoether" | "lovelace" => Units::Mwei,
            "kwei" | "femto" | "femtoether" | "babbage" => Units::Kwei,
            "wei" => Units::Wei,
            _ => return Err(ConversionError::UnrecognizedUnits(s.to_string())),
        })
    }
}

impl From<Units> for u32 {
    fn from(units: Units) -> Self {
        units.as_num()
    }
}

impl From<Units> for i32 {
    fn from(units: Units) -> Self {
        units.as_num() as i32
    }
}

impl From<Units> for usize {
    fn from(units: Units) -> Self {
        units.as_num() as usize
    }
}

impl Units {
    /// Converts the ethereum unit to its numeric representation.
    pub fn as_num(&self) -> u32 {
        match self {
            Units::Wei => 0,
            Units::Kwei => 3,
            Units::Mwei => 6,
            Units::Gwei => 9,
            Units::Twei => 12,
            Units::Pwei => 15,
            Units::Ether => 18,
            Units::Other(inner) => *inner,
        }
    }
}

/// This enum holds the numeric types that a possible to be returned by `parse_units` and
/// that are taken by `format_units`.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum ParseUnits {
    /// Unsigned 256-bit integer.
    U256(U256),
    /// Signed 256-bit integer.
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

/// Handles all possible conversion errors.
#[derive(Error, Debug)]
pub enum ConversionError {
    /// The unit is unrecognized.
    #[error("Unknown units: {0}")]
    UnrecognizedUnits(String),
    /// The provided hex string is invalid (too long).
    #[error("bytes32 strings must not exceed 32 bytes in length")]
    TextTooLong,
    /// The provided string cannot be converted from Utf8.
    #[error(transparent)]
    Utf8Error(#[from] std::str::Utf8Error),
    /// Invalid float.
    #[error(transparent)]
    InvalidFloat(#[from] std::num::ParseFloatError),
    /// Could not convert from decimal string.
    #[error("Invalid decimal string: {0}")]
    FromDecStrError(String),
    /// Overflowed while parsing.
    #[error("Overflow parsing string")]
    ParseOverflow,
    /// Could not convert from signed decimal string.
    #[error("Parse Signed Error")]
    ParseI256Error(#[from] ParseSignedError),
    /// Invalid checksum.
    #[error("Invalid address checksum")]
    InvalidAddressChecksum,
    /// Invalid hex.
    #[error(transparent)]
    FromHexError(<Address as std::str::FromStr>::Err),
}

/// Divides the provided amount with 10^{units} provided.
pub fn format_units<T, K>(amount: T, units: K) -> Result<String, ConversionError>
where
    T: Into<ParseUnits>,
    K: TryInto<Units, Error = ConversionError>,
{
    let units: usize = units.try_into()?.into();
    let amount = amount.into();

    match amount {
        // 2**256 ~= 1.16e77
        ParseUnits::U256(_) if units >= OVERFLOW_U256_UNITS => {
            return Err(ConversionError::ParseOverflow)
        }
        // 2**255 ~= 5.79e76
        ParseUnits::I256(_) if units >= OVERFLOW_I256_UNITS => {
            return Err(ConversionError::ParseOverflow)
        }
        _ => {}
    };
    let exp10 = U256::pow(U256::from(10), U256::from(units));

    // `decimals` are formatted twice because U256 does not support alignment (`:0>width`).
    match amount {
        ParseUnits::U256(amount) => {
            let integer = amount / exp10;
            let decimals = (amount % exp10).to_string();
            Ok(format!("{integer}.{decimals:0>units$}"))
        }
        ParseUnits::I256(amount) => {
            let exp10 = I256::from_raw(exp10);
            let sign = if amount.is_negative() { "-" } else { "" };
            let integer = (amount / exp10).twos_complement();
            let decimals = ((amount % exp10).twos_complement()).to_string();
            Ok(format!("{sign}{integer}.{decimals:0>units$}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Units::*;

    #[test]
    fn test_units() {
        assert_eq!(Wei.as_num(), 0);
        assert_eq!(Kwei.as_num(), 3);
        assert_eq!(Mwei.as_num(), 6);
        assert_eq!(Gwei.as_num(), 9);
        assert_eq!(Twei.as_num(), 12);
        assert_eq!(Pwei.as_num(), 15);
        assert_eq!(Ether.as_num(), 18);
        assert_eq!(Other(10).as_num(), 10);
        assert_eq!(Other(20).as_num(), 20);
    }

    #[test]
    fn test_into() {
        assert_eq!(Units::try_from("wei").unwrap(), Wei);
        assert_eq!(Units::try_from("kwei").unwrap(), Kwei);
        assert_eq!(Units::try_from("mwei").unwrap(), Mwei);
        assert_eq!(Units::try_from("gwei").unwrap(), Gwei);
        assert_eq!(Units::try_from("twei").unwrap(), Twei);
        assert_eq!(Units::try_from("pwei").unwrap(), Pwei);
        assert_eq!(Units::try_from("ether").unwrap(), Ether);

        assert_eq!(Units::try_from("wei".to_string()).unwrap(), Wei);
        assert_eq!(Units::try_from("kwei".to_string()).unwrap(), Kwei);
        assert_eq!(Units::try_from("mwei".to_string()).unwrap(), Mwei);
        assert_eq!(Units::try_from("gwei".to_string()).unwrap(), Gwei);
        assert_eq!(Units::try_from("twei".to_string()).unwrap(), Twei);
        assert_eq!(Units::try_from("pwei".to_string()).unwrap(), Pwei);
        assert_eq!(Units::try_from("ether".to_string()).unwrap(), Ether);

        assert_eq!(Units::try_from(&"wei".to_string()).unwrap(), Wei);
        assert_eq!(Units::try_from(&"kwei".to_string()).unwrap(), Kwei);
        assert_eq!(Units::try_from(&"mwei".to_string()).unwrap(), Mwei);
        assert_eq!(Units::try_from(&"gwei".to_string()).unwrap(), Gwei);
        assert_eq!(Units::try_from(&"twei".to_string()).unwrap(), Twei);
        assert_eq!(Units::try_from(&"pwei".to_string()).unwrap(), Pwei);
        assert_eq!(Units::try_from(&"ether".to_string()).unwrap(), Ether);
    }
}
