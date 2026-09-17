//! Number base parsing and formatting for the `cast` conversion commands.

use alloy_primitives::{I256, U256};
use eyre::Result;
use std::{
    fmt::{Debug, Display, Formatter, LowerHex, Result as FmtResult},
    num::IntErrorKind,
    str::FromStr,
};

/// Represents a number's [radix] or base. Supports the same bases that [`std::fmt`] supports.
///
/// [radix]: https://en.wikipedia.org/wiki/Radix
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Base {
    Binary = 2,
    Octal = 8,
    #[default]
    Decimal = 10,
    Hexadecimal = 16,
}

impl FromStr for Base {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "2" | "b" | "bin" | "binary" => Ok(Self::Binary),
            "8" | "o" | "oct" | "octal" => Ok(Self::Octal),
            "10" | "d" | "dec" | "decimal" => Ok(Self::Decimal),
            "16" | "h" | "hex" | "hexadecimal" => Ok(Self::Hexadecimal),
            s => Err(eyre::eyre!(
                "\
Invalid base \"{s}\". Possible values:
 2, b, bin, binary
 8, o, oct, octal
10, d, dec, decimal
16, h, hex, hexadecimal"
            )),
        }
    }
}

impl Base {
    /// Parses `base` when given, otherwise detects the base of `s` from its prefix.
    pub fn unwrap_or_detect(base: Option<&str>, s: &str) -> Result<Self> {
        match base {
            Some(base) => base.parse(),
            None => Self::detect(s),
        }
    }

    /// Detects a number's base from its prefix, defaulting to decimal and then hexadecimal for
    /// unprefixed values.
    pub fn detect(s: &str) -> Result<Self> {
        let s = s.strip_prefix(['+', '-']).unwrap_or(s);
        let prefix = s.get(..2).map(str::to_ascii_lowercase);
        match prefix.as_deref() {
            Some("0b") => Self::detect_prefixed(s, Self::Binary, "binary"),
            Some("0o") => Self::detect_prefixed(s, Self::Octal, "octal"),
            Some("0x") => Self::detect_prefixed(s, Self::Hexadecimal, "hexadecimal"),
            // Unprefixed digits are ambiguous; prefer decimal.
            _ if U256::from_str_radix(s, 10).is_ok() => Ok(Self::Decimal),
            _ => U256::from_str_radix(s, 16).map(|_| Self::Hexadecimal).map_err(|e| {
                eyre::eyre!("could not autodetect base as neither decimal or hexadecimal: {e}")
            }),
        }
    }

    /// Validates the digits after a 2-char prefix. `PosOverflow` is accepted since the digits are
    /// correct for the base; only the base is being detected here.
    fn detect_prefixed(s: &str, base: Self, label: &str) -> Result<Self> {
        match u64::from_str_radix(&s[2..], base as u32) {
            Ok(_) => Ok(base),
            Err(e) if *e.kind() == IntErrorKind::PosOverflow => Ok(base),
            Err(e) => Err(eyre::eyre!("could not parse {label} value: {e}")),
        }
    }

    /// Returns the Rust standard prefix for a base.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Binary => "0b",
            Self::Octal => "0o",
            Self::Decimal => "",
            Self::Hexadecimal => "0x",
        }
    }
}

/// A parsed number together with the [`Base`] it is formatted in.
///
/// [`Debug`] formats the number in its base, [`Display`] in decimal and [`LowerHex`] in
/// hexadecimal; the alternate flag (`#`) prepends the base prefix.
///
/// # Example
///
/// ```
/// use alloy_primitives::U256;
/// use cast::base::{Base, NumberWithBase};
///
/// let number = NumberWithBase::from(U256::from(12345));
/// assert_eq!(format!("{number}"), "12345");
/// assert_eq!(format!("{number:x}"), "3039");
/// assert_eq!(format!("{number:#x}"), "0x3039");
/// assert_eq!(format!("{:#?}", number.with_base(Base::Binary)), "0b11000000111001");
/// assert_eq!(format!("{:#?}", number.with_base(Base::Octal)), "0o30071");
/// ```
#[derive(Clone, Copy)]
pub struct NumberWithBase {
    /// The number, as two's complement when negative.
    number: U256,
    /// Whether the number is positive or zero.
    is_nonnegative: bool,
    /// The base to format to.
    base: Base,
}

impl Debug for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let prefix = self.base.prefix();
        if self.number.is_zero() {
            return f.pad_integral(true, prefix, "0");
        }
        // Only decimal output carries a sign; the other bases show the two's complement.
        let is_nonnegative = self.base != Base::Decimal || self.is_nonnegative;
        f.pad_integral(is_nonnegative, prefix, &self.format())
    }
}

impl Display for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(&self.with_base(Base::Decimal), f)
    }
}

impl LowerHex for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(&self.with_base(Base::Hexadecimal), f)
    }
}

impl From<I256> for NumberWithBase {
    fn from(number: I256) -> Self {
        Self {
            number: number.into_raw(),
            is_nonnegative: !number.is_negative(),
            base: Base::default(),
        }
    }
}

impl From<U256> for NumberWithBase {
    fn from(number: U256) -> Self {
        Self { number, is_nonnegative: true, base: Base::default() }
    }
}

impl NumberWithBase {
    /// Parses a signed integer, detecting the base from the prefix when `base` is `None`.
    pub fn parse_int(s: &str, base: Option<&str>) -> Result<Self> {
        Self::parse_int_in(s, Base::unwrap_or_detect(base, s)?)
    }

    /// Parses a signed integer in `base`.
    pub fn parse_int_in(s: &str, base: Base) -> Result<Self> {
        let (s, is_nonnegative) = match s.strip_prefix('-') {
            Some(s) => (s, false),
            None => (s.strip_prefix('+').unwrap_or(s), true),
        };
        let mut number = Self::parse_digits(s, base)?;
        if !is_nonnegative {
            number = number.wrapping_neg();
        }
        Ok(Self { number, is_nonnegative, base })
    }

    /// Parses an unsigned integer, detecting the base from the prefix when `base` is `None`.
    pub fn parse_uint(s: &str, base: Option<&str>) -> Result<Self> {
        let base = Base::unwrap_or_detect(base, s)?;
        Ok(Self { number: Self::parse_digits(s, base)?, is_nonnegative: true, base })
    }

    fn parse_digits(s: &str, base: Base) -> Result<U256> {
        let s = match s.get(..2) {
            Some("0x" | "0X" | "0o" | "0O" | "0b" | "0B") => &s[2..],
            _ => s,
        };
        U256::from_str_radix(s, base as u64).map_err(Into::into)
    }

    /// Returns the number as an unsigned integer (two's complement when negative).
    pub const fn number(&self) -> U256 {
        self.number
    }

    /// Returns whether the number is positive or zero.
    pub const fn is_nonnegative(&self) -> bool {
        self.is_nonnegative
    }

    /// Returns a copy of the number formatted in `base`.
    pub const fn with_base(self, base: Base) -> Self {
        Self { base, ..self }
    }

    /// Formats the number's digits in its base, without any prefix, sign or padding.
    fn format(&self) -> String {
        match self.base {
            Base::Binary => format!("{:b}", self.number),
            Base::Octal => format!("{:o}", self.number),
            Base::Decimal if self.is_nonnegative => self.number.to_string(),
            Base::Decimal => I256::from_raw(self.number).to_string().trim_start_matches('-').into(),
            Base::Hexadecimal => format!("{:x}", self.number),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Base::*;

    const NUMS: [i128; 44] = [
        1,
        2,
        3,
        5,
        7,
        8,
        10,
        11,
        13,
        16,
        17,
        19,
        23,
        29,
        31,
        32,
        37,
        41,
        43,
        47,
        53,
        59,
        61,
        64,
        67,
        71,
        73,
        79,
        83,
        89,
        97,
        100,
        128,
        200,
        333,
        500,
        666,
        1000,
        6666,
        10000,
        i16::MAX as i128,
        i32::MAX as i128,
        i64::MAX as i128,
        i128::MAX,
    ];

    fn number(n: i128) -> NumberWithBase {
        NumberWithBase::from(I256::try_from(n).unwrap())
    }

    #[test]
    fn can_parse_base() {
        for (aliases, base) in [
            (["2", "b", "bin", "binary"], Binary),
            (["8", "o", "oct", "octal"], Octal),
            (["10", "d", "dec", "decimal"], Decimal),
            (["16", "h", "hex", "hexadecimal"], Hexadecimal),
        ] {
            for alias in aliases {
                assert_eq!(alias.parse::<Base>().unwrap(), base, "{alias}");
                assert_eq!(alias.to_uppercase().parse::<Base>().unwrap(), base, "{alias}");
            }
        }
        assert!("3".parse::<Base>().is_err());
    }

    #[test]
    fn can_detect_base() {
        assert_eq!(Base::detect("0b100").unwrap(), Binary);
        assert_eq!(Base::detect("0o100").unwrap(), Octal);
        assert_eq!(Base::detect("100").unwrap(), Decimal);
        assert_eq!(Base::detect("0x100").unwrap(), Hexadecimal);

        assert_eq!(Base::detect("0B100").unwrap(), Binary);
        assert_eq!(Base::detect("0O100").unwrap(), Octal);
        assert_eq!(Base::detect("0X100").unwrap(), Hexadecimal);

        assert_eq!(Base::detect("-0B100").unwrap(), Binary);
        assert_eq!(Base::detect("-0O100").unwrap(), Octal);
        assert_eq!(Base::detect("-0X100").unwrap(), Hexadecimal);

        assert_eq!(Base::detect("0123456789abcdef").unwrap(), Hexadecimal);

        let _ = Base::detect("0b234abc").unwrap_err();
        let _ = Base::detect("0o89cba").unwrap_err();
        let _ = Base::detect("0123456789abcdefg").unwrap_err();
        let _ = Base::detect("0x123abclpmk").unwrap_err();
        let _ = Base::detect("hello world").unwrap_err();
    }

    #[test]
    fn formats_positive_numbers() {
        for n in NUMS {
            let num = number(n);
            assert_eq!(num.with_base(Binary).format(), format!("{n:b}"));
            assert_eq!(num.with_base(Octal).format(), format!("{n:o}"));
            assert_eq!(num.with_base(Decimal).format(), n.to_string());
            assert_eq!(num.with_base(Hexadecimal).format(), format!("{n:x}"));

            assert_eq!(format!("{num}"), n.to_string());
            assert_eq!(format!("{num:x}"), format!("{n:x}"));
            assert_eq!(format!("{num:#x}"), format!("{n:#x}"));
            assert_eq!(format!("{:#?}", num.with_base(Binary)), format!("{n:#b}"));
            assert_eq!(format!("{:#?}", num.with_base(Octal)), format!("{n:#o}"));
        }
    }

    #[test]
    fn formats_negative_numbers() {
        for n in NUMS.into_iter().map(|n| -n).chain([i128::MIN]) {
            let num = number(n);
            // The underlying number is 256 bits wide, so the two's complement is sign-extended.
            assert_eq!(num.with_base(Binary).format(), format!("{n:1>256b}"));
            assert_eq!(num.with_base(Hexadecimal).format(), format!("{n:f>64x}"));
            // Decimal digits never carry the sign; `Display` adds it back.
            assert_eq!(num.with_base(Decimal).format(), n.to_string().trim_start_matches('-'));
            assert_eq!(format!("{num}"), n.to_string());
        }
    }
}
