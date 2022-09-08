use ethers_core::{
    abi::ethereum_types::{FromStrRadixErr, FromStrRadixErrKind},
    types::{Sign, I256, U256},
};
use eyre::{Context, ContextCompat, Result};
use std::{
    convert::TryFrom,
    fmt::{Debug, Display, Formatter, Result as FmtResult},
    iter::FromIterator,
    str::FromStr,
};

/* -------------------------------------------- Base -------------------------------------------- */

// TODO: UpperHex and LowerHex
/// Represents a number's [radix] or base. Currently it supports the same bases that [std::fmt]
/// supports.
///
/// [Radix] = (https://en.wikipedia.org/wiki/Radix)
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Base {
    Binary = 2,
    Octal = 8,
    #[default]
    Decimal = 10,
    Hexadecimal = 16,
}

impl Display for Base {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Display::fmt(&(*self as u32), f)
    }
}

impl FromStr for Base {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "2" | "b" | "bin" | "binary" => Ok(Self::Binary),
            "8" | "o" | "oct" | "octal" => Ok(Self::Octal),
            "10" | "d" | "dec" | "decimal" => Ok(Self::Decimal),
            "16" | "h" | "hex" | "hexadecimal " => Ok(Self::Hexadecimal),
            _ => Err(eyre::eyre!(
                r#"Invalid base. Possible options:
2, b, bin, binary
8, o, oct, octal
10, d, dec, decimal
16, h, hex, hexadecimal
                "#
            )),
        }
    }
}

impl TryFrom<String> for Base {
    type Error = eyre::Report;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl TryFrom<u32> for Base {
    type Error = eyre::Report;

    fn try_from(n: u32) -> Result<Self, Self::Error> {
        match n {
            2 => Ok(Self::Binary),
            8 => Ok(Self::Octal),
            10 => Ok(Self::Decimal),
            16 => Ok(Self::Hexadecimal),
            _ => Err(eyre::eyre!("Invalid base. Possible options: 2, 8, 10, 16")),
        }
    }
}

impl TryFrom<I256> for Base {
    type Error = eyre::Report;

    fn try_from(n: I256) -> Result<Self, Self::Error> {
        Self::try_from(n.low_u32())
    }
}

impl TryFrom<U256> for Base {
    type Error = eyre::Report;

    fn try_from(n: U256) -> Result<Self, Self::Error> {
        Self::try_from(n.low_u32())
    }
}

impl From<Base> for u32 {
    fn from(b: Base) -> Self {
        b as u32
    }
}

impl From<Base> for I256 {
    fn from(b: Base) -> Self {
        Self::from(b as u32)
    }
}

impl From<Base> for U256 {
    fn from(b: Base) -> Self {
        Self::from(b as u32)
    }
}

impl From<Base> for String {
    fn from(b: Base) -> Self {
        b.to_string()
    }
}

impl Base {
    pub fn unwrap_or_detect(base: Option<Self>, s: impl AsRef<str>) -> Result<Self> {
        base.map_or_else(|| Self::detect(s), |base| Ok(base))
    }

    /// Try parsing a number's base from a string.
    pub fn detect(s: impl AsRef<str>) -> Result<Self> {
        let s = s.as_ref();
        match s {
            _ if s.starts_with("0b") => Ok(Self::Binary),
            _ if s.starts_with("0o") => Ok(Self::Octal),
            _ if s.starts_with("0x") => Ok(Self::Hexadecimal),
            // No prefix => first try parsing as decimal
            _ => match U256::from_str_radix(s, 10) {
                Ok(_) => {
                    match U256::from_str_radix(s, 16) {
                        // Can be both, ambiguous
                        Ok(_) => Err(eyre::eyre!("Could not autodetect base: input could be decimal or hexadecimal. Please prepend with 0x if the input is hexadecimal, or specify a --base-in parameter.")),
                        // Can only be decimal
                        Err(_) => Ok(Self::Decimal),
                    }
                }
                Err(_) => match U256::from_str_radix(s, 16) {
                    Ok(_) => Ok(Self::Hexadecimal),
                    Err(e) => Err(eyre::eyre!(
                        "Could not autodetect base neither as decimal nor as hexadecimal: {}",
                        e
                    )),
                },
            },
        }
    }

    /// Returns the Rust standard prefix for a base
    pub const fn prefix(&self) -> &str {
        match self {
            Base::Binary => "0b",
            Base::Octal => "0o",
            Base::Hexadecimal => "0x",
            _ => "",
        }
    }
}

/* --------------------------------------- NumberWithBase --------------------------------------- */

#[derive(Clone, Copy)]
pub struct NumberWithBase {
    number: U256,
    is_positive: bool,
    base: Base,
}

impl Debug for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Display::fmt(self, f)
    }
}

impl Display for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let prefix = self.base.prefix();
        if self.number.is_zero() {
            f.pad_integral(true, prefix, "0")
        } else {
            f.pad_integral(self.is_positive, prefix, &self.format(false))
        }
    }
}

impl FromStr for NumberWithBase {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_int(s, None)
    }
}

impl From<I256> for NumberWithBase {
    fn from(number: I256) -> Self {
        // both is_positive and is_negative return false for 0
        Self::new(number.into_raw(), !number.is_negative(), Base::default())
    }
}

impl From<U256> for NumberWithBase {
    fn from(number: U256) -> Self {
        Self::new(number, true, Base::default())
    }
}

impl From<NumberWithBase> for I256 {
    fn from(n: NumberWithBase) -> Self {
        I256::from_raw(n.number)
    }
}

impl From<NumberWithBase> for U256 {
    fn from(n: NumberWithBase) -> Self {
        n.number
    }
}

impl From<NumberWithBase> for String {
    /// Formats the number into the specified base with its prefix
    fn from(n: NumberWithBase) -> Self {
        n.format(true)
    }
}

impl NumberWithBase {
    pub fn new(number: impl Into<U256>, is_positive: bool, base: impl Into<Base>) -> Self {
        Self { number: number.into(), is_positive, base: base.into() }
    }

    /// Parses a string slice into a signed integer. If base is None then it tries to determine base
    /// from the prefix, otherwise defaults to Decimal.
    pub fn parse_int(s: &str, base: Option<Base>) -> Result<Self> {
        let base = Base::unwrap_or_detect(base, s)/*.unwrap_or_default()*/?;
        let (number, is_positive) = Self::_parse_int(s, base)?;
        Ok(Self { number, is_positive, base })
    }

    /// Parses a string slice into an unsigned integer. If base is None then it tries to determine
    /// base from the prefix, otherwise defaults to Decimal.
    pub fn parse_uint(s: &str, base: Option<Base>) -> Result<Self> {
        let base = Base::unwrap_or_detect(base, s)/*.unwrap_or_default()*/?;
        let number = Self::_parse_uint(s, base)?;
        Ok(Self { number, is_positive: true, base })
    }

    /// Returns the underlying number as an unsigned integer. If the value is negative then the
    /// two's complement of its absolute value will be returned.
    pub fn number(&self) -> U256 {
        self.number
    }

    /// Returns whether the underlying number is to be treated as a signed integer.
    pub fn is_positive(&self) -> bool {
        self.is_positive
    }

    /// Returns the underlying base. Defaults to [`Decimal`][Base].
    pub fn base(&self) -> Base {
        self.base
    }

    /// Returns the Rust standard prefix for the base.
    pub const fn prefix(&self) -> &str {
        self.base.prefix()
    }

    /// Sets the number's base to format to.
    pub fn set_base(&mut self, base: Base) -> &mut Self {
        self.base = base;
        self
    }

    /// Formats the number into the specified base.
    pub fn format(&self, add_prefix: bool) -> String {
        let prefix = if add_prefix { self.prefix() } else { "" };
        let s = match self.base {
            // Binary and Octal traits are not implemented for primitive-types types, so we're using
            // a custom formatter
            Base::Binary | Base::Octal => self.format_radix(),
            Base::Decimal => {
                if self.is_positive {
                    self.number.to_string()
                } else {
                    I256::from_raw(self.number).to_string()
                }
            }
            Base::Hexadecimal => format!("{:x}", self.number),
        };
        format!("{}{}", prefix, s)
    }

    /// Iterates over every digit and calls [std::char::from_digit] to create a String.
    ///
    /// Modified from: https://stackoverflow.com/a/50278316
    fn format_radix(&self) -> String {
        let mut x = self.number;
        let radix = self.base as u32;
        let r = U256::from(radix);

        let mut buf = ['\0'; 256];
        let mut i = 255;
        loop {
            let m = (x % r).low_u64() as u32;
            // radix is always less than 37 so from_digit cannot panic
            // m is always in the radix's range so unwrap cannot panic
            buf[i] = char::from_digit(m, radix).unwrap();
            x /= r;
            if x.is_zero() {
                break
            }
            i -= 1;
        }
        String::from_iter(&buf[i..])
    }

    fn _parse_int(s: &str, base: Base) -> Result<(U256, bool)> {
        let (s, sign) = get_sign(s);
        let mut n = Self::_parse_uint(s, base)?;

        let is_neg = matches!(sign, Sign::Negative);
        if is_neg {
            n = (!n).overflowing_add(U256::one()).0;
        }

        Ok((n, !is_neg))
    }

    fn _parse_uint(s: &str, base: Base) -> Result<U256> {
        // TODO: Parse from binary or octal str into U256
        U256::from_str_radix(s, base as u32).map_err(|e| {
            if matches!(e.kind(), FromStrRadixErrKind::UnsupportedRadix) {
                eyre::eyre!("numbers in base {} are currently not supported as input", base)
            } else {
                eyre::eyre!(e)
            }
        })
    }
}

/* ------------------------------------------- ToBase ------------------------------------------- */

/// Facilitates formatting an integer into a [Base].
pub trait ToBase {
    fn to_base(&self, base: Base, add_prefix: bool) -> String;
}

impl ToBase for I256 {
    fn to_base(&self, base: Base, add_prefix: bool) -> String {
        NumberWithBase::from(*self).set_base(base).format(add_prefix)
    }
}

impl ToBase for U256 {
    fn to_base(&self, base: Base, add_prefix: bool) -> String {
        NumberWithBase::from(*self).set_base(base).format(add_prefix)
    }
}

fn get_sign(s: &str) -> (&str, Sign) {
    match s.as_bytes().first() {
        Some(b'+') => (&s[1..], Sign::Positive),
        Some(b'-') => (&s[1..], Sign::Negative),
        _ => (s, Sign::Positive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Base::*;

    const POS_NUM: [i32; 14] = [1, 2, 8, 10, 16, 32, 64, 100, 128, 200, 500, 1000, 10000, i32::MAX];

    const NEG_NUM: [i32; 14] =
        [-1, -2, -8, -10, -16, -32, -64, -100, -128, -200, -500, -1000, -10000, i32::MIN];

    #[test]
    fn test_defaults() {
        let def: Base = Default::default();
        assert!(matches!(def, Decimal));

        let n: NumberWithBase = U256::zero().into();
        assert!(matches!(n.base, Decimal));
        let n: NumberWithBase = I256::zero().into();
        assert!(matches!(n.base, Decimal));
    }

    #[test]
    fn test_detect() {
        assert_eq!(Base::detect("0b100").unwrap(), Binary);
        assert_eq!(Base::detect("0o100").unwrap(), Octal);
        assert_eq!(Base::detect("100").unwrap(), Decimal);
        assert_eq!(Base::detect("0x100").unwrap(), Hexadecimal);

        Base::detect("0b234abc").unwrap_err();
        Base::detect("0o89cba").unwrap_err();
        Base::detect("123456abcdef").unwrap_err();
        Base::detect("0x123abclpmk").unwrap_err();
        Base::detect("hello world").unwrap_err();
    }

    #[test]
    fn test_fmt_pos() {
        let expected_2: Vec<_> = POS_NUM.iter().map(|n| format!("{:b}", n)).collect();
        let expected_2_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{:#b}", n)).collect();
        let expected_8: Vec<_> = POS_NUM.iter().map(|n| format!("{:o}", n)).collect();
        let expected_8_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{:#o}", n)).collect();
        let expected_10: Vec<_> = POS_NUM.iter().map(|n| format!("{:}", n)).collect();
        let expected_10_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{:#}", n)).collect();
        let expected_16: Vec<_> = POS_NUM.iter().map(|n| format!("{:x}", n)).collect();
        let expected_16_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{:#x}", n)).collect();

        let mut alt = false;
        for (i, n) in POS_NUM.into_iter().enumerate() {
            let mut num: NumberWithBase = I256::from(n).into();

            alt = false;
            assert_eq!(num.set_base(Binary).format(alt), expected_2[i]);
            assert_eq!(num.set_base(Octal).format(alt), expected_8[i]);
            assert_eq!(num.set_base(Decimal).format(alt), expected_10[i]);
            assert_eq!(num.set_base(Hexadecimal).format(alt), expected_16[i]);

            alt = true;
            assert_eq!(num.set_base(Binary).format(alt), expected_2_alt[i]);
            assert_eq!(num.set_base(Octal).format(alt), expected_8_alt[i]);
            assert_eq!(num.set_base(Decimal).format(alt), expected_10_alt[i]);
            assert_eq!(num.set_base(Hexadecimal).format(alt), expected_16_alt[i]);
        }
    }

    #[test]
    fn test_fmt_neg() {
        // underlying is 256 bits so we have to pad left manually
        let expected_2: Vec<_> = NEG_NUM.iter().map(|n| format!("{:1>256b}", n)).collect();
        let expected_2_alt: Vec<_> = NEG_NUM.iter().map(|n| format!("0b{:1>256b}", n)).collect();
        // TODO: create expected for octal
        // let expected_8: Vec<_> = NEG_NUM.iter().map(|n| format!("1{:7>85o}", n)).collect();
        // let expected_8_alt: Vec<_> = NEG_NUM.iter().map(|n| format!("0o1{:7>85o}", n)).collect();
        let expected_10: Vec<_> = NEG_NUM.iter().map(|n| format!("{:}", n)).collect();
        let expected_10_alt: Vec<_> = NEG_NUM.iter().map(|n| format!("{:#}", n)).collect();
        let expected_16: Vec<_> = NEG_NUM.iter().map(|n| format!("{:f>64x}", n)).collect();
        let expected_16_alt: Vec<_> = NEG_NUM.iter().map(|n| format!("0x{:f>64x}", n)).collect();

        let mut alt = false;
        for (i, n) in NEG_NUM.into_iter().enumerate() {
            let mut num: NumberWithBase = I256::from(n).into();

            alt = false;
            assert_eq!(num.set_base(Binary).format(alt), expected_2[i]);
            // assert_eq!(num.set_base(Octal).format(alt), expected_8[i]);
            assert_eq!(num.set_base(Decimal).format(alt), expected_10[i]);
            assert_eq!(num.set_base(Hexadecimal).format(alt), expected_16[i]);

            alt = true;
            assert_eq!(num.set_base(Binary).format(alt), expected_2_alt[i]);
            // assert_eq!(num.set_base(Octal).format(alt), expected_8_alt[i]);
            assert_eq!(num.set_base(Decimal).format(alt), expected_10_alt[i]);
            assert_eq!(num.set_base(Hexadecimal).format(alt), expected_16_alt[i]);
        }
    }
}
