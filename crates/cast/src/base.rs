use alloy_primitives::{utils::ParseUnits, Sign, I256, U256};
use eyre::Result;
use std::{
    convert::Infallible,
    fmt::{Binary, Debug, Display, Formatter, LowerHex, Octal, Result as FmtResult, UpperHex},
    num::IntErrorKind,
    str::FromStr,
};

/* -------------------------------------------- Base -------------------------------------------- */

/// Represents a number's [radix] or base. Currently it supports the same bases that [std::fmt]
/// supports.
///
/// [radix]: https://en.wikipedia.org/wiki/Radix
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
            n => Err(eyre::eyre!("Invalid base \"{}\". Possible values: 2, 8, 10, 16", n)),
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
        Self::try_from(n.saturating_to::<u32>())
    }
}

impl From<Base> for u32 {
    fn from(b: Base) -> Self {
        b as Self
    }
}

impl From<Base> for String {
    fn from(b: Base) -> Self {
        b.to_string()
    }
}

impl Base {
    pub fn unwrap_or_detect(base: Option<&str>, s: impl AsRef<str>) -> Result<Self> {
        match base {
            Some(base) => base.parse(),
            None => Self::detect(s),
        }
    }

    /// Try parsing a number's base from a string.
    pub fn detect(s: impl AsRef<str>) -> Result<Self> {
        let s = s.as_ref();
        match s {
            // Ignore sign
            _ if s.starts_with(['+', '-']) => Self::detect(&s[1..]),
            // Verify binary and octal values with u128::from_str_radix as U256 does not support
            // them;
            // assume overflows are within u128::MAX and U256::MAX, we're not using the parsed value
            // anyway;
            // strip prefix when using u128::from_str_radix because it does not recognize it as
            // valid.
            _ if s.starts_with("0b") => match u64::from_str_radix(&s[2..], 2) {
                Ok(_) => Ok(Self::Binary),
                Err(e) => match e.kind() {
                    IntErrorKind::PosOverflow => Ok(Self::Binary),
                    _ => Err(eyre::eyre!("could not parse binary value: {}", e)),
                },
            },
            _ if s.starts_with("0o") => match u64::from_str_radix(&s[2..], 8) {
                Ok(_) => Ok(Self::Octal),
                Err(e) => match e.kind() {
                    IntErrorKind::PosOverflow => Ok(Self::Octal),
                    _ => Err(eyre::eyre!("could not parse octal value: {e}")),
                },
            },
            _ if s.starts_with("0x") => match u64::from_str_radix(&s[2..], 16) {
                Ok(_) => Ok(Self::Hexadecimal),
                Err(e) => match e.kind() {
                    IntErrorKind::PosOverflow => Ok(Self::Hexadecimal),
                    _ => Err(eyre::eyre!("could not parse hexadecimal value: {e}")),
                },
            },
            // No prefix => first try parsing as decimal
            _ => match U256::from_str_radix(s, 10) {
                // Can be both, ambiguous but default to Decimal
                Ok(_) => Ok(Self::Decimal),
                Err(_) => match U256::from_str_radix(s, 16) {
                    Ok(_) => Ok(Self::Hexadecimal),
                    Err(e) => Err(eyre::eyre!(
                        "could not autodetect base as neither decimal or hexadecimal: {e}"
                    )),
                },
            },
        }
    }

    /// Returns the Rust standard prefix for a base
    pub const fn prefix(&self) -> &str {
        match self {
            Self::Binary => "0b",
            Self::Octal => "0o",
            Self::Decimal => "",
            Self::Hexadecimal => "0x",
        }
    }
}

/* --------------------------------------- NumberWithBase --------------------------------------- */

/// Utility struct for parsing numbers and formatting them into different [bases][Base].
///
/// # Example
///
/// ```
/// use cast::base::NumberWithBase;
/// use alloy_primitives::U256;
///
/// let number: NumberWithBase = U256::from(12345).into();
/// assert_eq!(number.format(), "12345");
///
/// // Debug uses number.base() to determine which base to format to, which defaults to Base::Decimal
/// assert_eq!(format!("{:?}", number), "12345");
///
/// // Display uses Base::Decimal
/// assert_eq!(format!("{}", number), "12345");
///
/// // The alternate formatter ("#") prepends the base's prefix
/// assert_eq!(format!("{:x}", number), "3039");
/// assert_eq!(format!("{:#x}", number), "0x3039");
///
/// assert_eq!(format!("{:b}", number), "11000000111001");
/// assert_eq!(format!("{:#b}", number), "0b11000000111001");
///
/// assert_eq!(format!("{:o}", number), "30071");
/// assert_eq!(format!("{:#o}", number), "0o30071");
/// ```
#[derive(Clone, Copy)]
pub struct NumberWithBase {
    /// The number.
    number: U256,
    /// Whether the number is positive or zero.
    is_nonnegative: bool,
    /// The base to format to.
    base: Base,
}

impl std::ops::Deref for NumberWithBase {
    type Target = U256;

    fn deref(&self) -> &Self::Target {
        &self.number
    }
}

// Format using self.base
impl Debug for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let prefix = self.base.prefix();
        if self.number.is_zero() {
            f.pad_integral(true, prefix, "0")
        } else {
            // Add sign only for decimal
            let is_nonnegative = match self.base {
                Base::Decimal => self.is_nonnegative,
                _ => true,
            };
            f.pad_integral(is_nonnegative, prefix, &self.format())
        }
    }
}

impl Binary for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(&self.with_base(Base::Binary), f)
    }
}

impl Octal for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(&self.with_base(Base::Octal), f)
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

impl UpperHex for NumberWithBase {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let n = format!("{self:x}").to_uppercase();
        f.pad_integral(true, Base::Hexadecimal.prefix(), &n)
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

impl From<ParseUnits> for NumberWithBase {
    fn from(value: ParseUnits) -> Self {
        match value {
            ParseUnits::U256(val) => val.into(),
            ParseUnits::I256(val) => val.into(),
        }
    }
}

impl From<U256> for NumberWithBase {
    fn from(number: U256) -> Self {
        Self::new(number, true, Base::default())
    }
}

impl From<NumberWithBase> for I256 {
    fn from(n: NumberWithBase) -> Self {
        Self::from_raw(n.number)
    }
}

impl From<NumberWithBase> for U256 {
    fn from(n: NumberWithBase) -> Self {
        n.number
    }
}

impl From<NumberWithBase> for String {
    /// Formats the number into the specified base. See [NumberWithBase::format].
    ///
    /// [NumberWithBase::format]: NumberWithBase
    fn from(n: NumberWithBase) -> Self {
        n.format()
    }
}

impl NumberWithBase {
    pub fn new(number: impl Into<U256>, is_nonnegative: bool, base: Base) -> Self {
        Self { number: number.into(), is_nonnegative, base }
    }

    /// Creates a copy of the number with the provided base.
    pub fn with_base(&self, base: Base) -> Self {
        Self { number: self.number, is_nonnegative: self.is_nonnegative, base }
    }

    /// Parses a string slice into a signed integer. If base is None then it tries to determine base
    /// from the prefix, otherwise defaults to Decimal.
    pub fn parse_int(s: &str, base: Option<&str>) -> Result<Self> {
        let base = Base::unwrap_or_detect(base, s)?;
        let (number, is_nonnegative) = Self::_parse_int(s, base)?;
        Ok(Self { number, is_nonnegative, base })
    }

    /// Parses a string slice into an unsigned integer. If base is None then it tries to determine
    /// base from the prefix, otherwise defaults to Decimal.
    pub fn parse_uint(s: &str, base: Option<&str>) -> Result<Self> {
        let base = Base::unwrap_or_detect(base, s)?;
        let number = Self::_parse_uint(s, base)?;
        Ok(Self { number, is_nonnegative: true, base })
    }

    /// Returns a copy of the underlying number as an unsigned integer. If the value is negative
    /// then the two's complement of its absolute value will be returned.
    pub fn number(&self) -> U256 {
        self.number
    }

    /// Returns whether the underlying number is positive or zero.
    pub fn is_nonnegative(&self) -> bool {
        self.is_nonnegative
    }

    /// Returns the underlying base. Defaults to [Decimal][Base].
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
    ///
    /// **Note**: this method only formats the number into the base, without adding any prefixes,
    /// signs or padding. Refer to the [std::fmt] module documentation on how to format this
    /// number with the aforementioned properties.
    pub fn format(&self) -> String {
        let s = match self.base {
            Base::Binary => format!("{:b}", self.number),
            Base::Octal => format!("{:o}", self.number),
            Base::Decimal => {
                if self.is_nonnegative {
                    self.number.to_string()
                } else {
                    let s = I256::from_raw(self.number).to_string();
                    s.strip_prefix('-').unwrap_or(&s).to_string()
                }
            }
            Base::Hexadecimal => format!("{:x}", self.number),
        };
        if s.starts_with('0') {
            s.trim_start_matches('0').to_string()
        } else {
            s
        }
    }

    fn _parse_int(s: &str, base: Base) -> Result<(U256, bool)> {
        let (s, sign) = get_sign(s);
        let mut n = Self::_parse_uint(s, base)?;

        let is_neg = matches!(sign, Sign::Negative);
        if is_neg {
            n = (!n).overflowing_add(U256::from(1)).0;
        }

        Ok((n, !is_neg))
    }

    fn _parse_uint(s: &str, base: Base) -> Result<U256> {
        let s = match s.get(0..2) {
            Some("0x" | "0X" | "0o" | "0O" | "0b" | "0B") => &s[2..],
            _ => s,
        };
        U256::from_str_radix(s, base as u64).map_err(Into::into)
    }
}

/* ------------------------------------------- ToBase ------------------------------------------- */

/// Facilitates formatting an integer into a [Base].
pub trait ToBase {
    type Err;

    /// Formats self into a base, specifying whether to add the base prefix or not.
    ///
    /// Tries converting `self` into a [NumberWithBase] and then formats into the provided base by
    /// using the [Debug] implementation.
    ///
    /// # Example
    ///
    /// ```
    /// use alloy_primitives::U256;
    /// use cast::base::{Base, ToBase};
    ///
    /// // Any type that implements ToBase
    /// let number = U256::from(12345);
    /// assert_eq!(number.to_base(Base::Decimal, false).unwrap(), "12345");
    /// assert_eq!(number.to_base(Base::Hexadecimal, false).unwrap(), "3039");
    /// assert_eq!(number.to_base(Base::Hexadecimal, true).unwrap(), "0x3039");
    /// assert_eq!(number.to_base(Base::Binary, true).unwrap(), "0b11000000111001");
    /// assert_eq!(number.to_base(Base::Octal, true).unwrap(), "0o30071");
    /// ```
    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err>;
}

impl ToBase for NumberWithBase {
    type Err = Infallible;

    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err> {
        let n = self.with_base(base);
        if add_prefix {
            Ok(format!("{n:#?}"))
        } else {
            Ok(format!("{n:?}"))
        }
    }
}

impl ToBase for I256 {
    type Err = Infallible;

    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err> {
        let n = NumberWithBase::from(*self).with_base(base);
        if add_prefix {
            Ok(format!("{n:#?}"))
        } else {
            Ok(format!("{n:?}"))
        }
    }
}

impl ToBase for U256 {
    type Err = Infallible;

    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err> {
        let n = NumberWithBase::from(*self).with_base(base);
        if add_prefix {
            Ok(format!("{n:#?}"))
        } else {
            Ok(format!("{n:?}"))
        }
    }
}

impl ToBase for String {
    type Err = eyre::Report;

    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err> {
        str::to_base(self, base, add_prefix)
    }
}

impl ToBase for str {
    type Err = eyre::Report;

    fn to_base(&self, base: Base, add_prefix: bool) -> Result<String, Self::Err> {
        let n = NumberWithBase::from_str(self)?.with_base(base);
        if add_prefix {
            Ok(format!("{n:#?}"))
        } else {
            Ok(format!("{n:?}"))
        }
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

    const POS_NUM: [i128; 44] = [
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

    const NEG_NUM: [i128; 44] = [
        -1,
        -2,
        -3,
        -5,
        -7,
        -8,
        -10,
        -11,
        -13,
        -16,
        -17,
        -19,
        -23,
        -29,
        -31,
        -32,
        -37,
        -41,
        -43,
        -47,
        -53,
        -59,
        -61,
        -64,
        -67,
        -71,
        -73,
        -79,
        -83,
        -89,
        -97,
        -100,
        -128,
        -200,
        -333,
        -500,
        -666,
        -1000,
        -6666,
        -10000,
        i16::MIN as i128,
        i32::MIN as i128,
        i64::MIN as i128,
        i128::MIN,
    ];

    #[test]
    fn test_defaults() {
        let def: Base = Default::default();
        assert!(matches!(def, Decimal));

        let n: NumberWithBase = U256::ZERO.into();
        assert!(matches!(n.base, Decimal));
        let n: NumberWithBase = I256::ZERO.into();
        assert!(matches!(n.base, Decimal));
    }

    #[test]
    fn can_parse_base() {
        assert_eq!("2".parse::<Base>().unwrap(), Binary);
        assert_eq!("b".parse::<Base>().unwrap(), Binary);
        assert_eq!("bin".parse::<Base>().unwrap(), Binary);
        assert_eq!("binary".parse::<Base>().unwrap(), Binary);

        assert_eq!("8".parse::<Base>().unwrap(), Octal);
        assert_eq!("o".parse::<Base>().unwrap(), Octal);
        assert_eq!("oct".parse::<Base>().unwrap(), Octal);
        assert_eq!("octal".parse::<Base>().unwrap(), Octal);

        assert_eq!("10".parse::<Base>().unwrap(), Decimal);
        assert_eq!("d".parse::<Base>().unwrap(), Decimal);
        assert_eq!("dec".parse::<Base>().unwrap(), Decimal);
        assert_eq!("decimal".parse::<Base>().unwrap(), Decimal);

        assert_eq!("16".parse::<Base>().unwrap(), Hexadecimal);
        assert_eq!("h".parse::<Base>().unwrap(), Hexadecimal);
        assert_eq!("hex".parse::<Base>().unwrap(), Hexadecimal);
        assert_eq!("hexadecimal".parse::<Base>().unwrap(), Hexadecimal);
    }

    #[test]
    fn can_detect_base() {
        assert_eq!(Base::detect("0b100").unwrap(), Binary);
        assert_eq!(Base::detect("0o100").unwrap(), Octal);
        assert_eq!(Base::detect("100").unwrap(), Decimal);
        assert_eq!(Base::detect("0x100").unwrap(), Hexadecimal);

        assert_eq!(Base::detect("0123456789abcdef").unwrap(), Hexadecimal);

        let _ = Base::detect("0b234abc").unwrap_err();
        let _ = Base::detect("0o89cba").unwrap_err();
        let _ = Base::detect("0123456789abcdefg").unwrap_err();
        let _ = Base::detect("0x123abclpmk").unwrap_err();
        let _ = Base::detect("hello world").unwrap_err();
    }

    #[test]
    fn test_format_pos() {
        let expected_2: Vec<_> = POS_NUM.iter().map(|n| format!("{n:b}")).collect();
        let expected_8: Vec<_> = POS_NUM.iter().map(|n| format!("{n:o}")).collect();
        let expected_10: Vec<_> = POS_NUM.iter().map(|n| format!("{n:}")).collect();
        let expected_l16: Vec<_> = POS_NUM.iter().map(|n| format!("{n:x}")).collect();
        let expected_u16: Vec<_> = POS_NUM.iter().map(|n| format!("{n:X}")).collect();

        for (i, n) in POS_NUM.into_iter().enumerate() {
            let mut num: NumberWithBase = I256::try_from(n).unwrap().into();

            assert_eq!(num.set_base(Binary).format(), expected_2[i]);
            assert_eq!(num.set_base(Octal).format(), expected_8[i]);
            assert_eq!(num.set_base(Decimal).format(), expected_10[i]);
            assert_eq!(num.set_base(Hexadecimal).format(), expected_l16[i]);
            assert_eq!(num.set_base(Hexadecimal).format().to_uppercase(), expected_u16[i]);
        }
    }

    // TODO: test for octal
    #[test]
    fn test_format_neg() {
        // underlying is 256 bits so we have to pad left manually

        let expected_2: Vec<_> = NEG_NUM.iter().map(|n| format!("{n:1>256b}")).collect();
        // let expected_8: Vec<_> = NEG_NUM.iter().map(|n| format!("1{:7>85o}", n)).collect();
        // Sign not included, see NumberWithBase::format
        let expected_10: Vec<_> =
            NEG_NUM.iter().map(|n| format!("{n:}").trim_matches('-').to_string()).collect();
        let expected_l16: Vec<_> = NEG_NUM.iter().map(|n| format!("{n:f>64x}")).collect();
        let expected_u16: Vec<_> = NEG_NUM.iter().map(|n| format!("{n:F>64X}")).collect();

        for (i, n) in NEG_NUM.into_iter().enumerate() {
            let mut num: NumberWithBase = I256::try_from(n).unwrap().into();

            assert_eq!(num.set_base(Binary).format(), expected_2[i]);
            // assert_eq!(num.set_base(Octal).format(), expected_8[i]);
            assert_eq!(num.set_base(Decimal).format(), expected_10[i]);
            assert_eq!(num.set_base(Hexadecimal).format(), expected_l16[i]);
            assert_eq!(num.set_base(Hexadecimal).format().to_uppercase(), expected_u16[i]);
        }
    }

    #[test]
    fn test_fmt_macro() {
        let nums: Vec<_> =
            POS_NUM.into_iter().map(|n| NumberWithBase::from(I256::try_from(n).unwrap())).collect();

        let actual_2: Vec<_> = nums.iter().map(|n| format!("{n:b}")).collect();
        let actual_2_alt: Vec<_> = nums.iter().map(|n| format!("{n:#b}")).collect();
        let actual_8: Vec<_> = nums.iter().map(|n| format!("{n:o}")).collect();
        let actual_8_alt: Vec<_> = nums.iter().map(|n| format!("{n:#o}")).collect();
        let actual_10: Vec<_> = nums.iter().map(|n| format!("{n:}")).collect();
        let actual_10_alt: Vec<_> = nums.iter().map(|n| format!("{n:#}")).collect();
        let actual_l16: Vec<_> = nums.iter().map(|n| format!("{n:x}")).collect();
        let actual_l16_alt: Vec<_> = nums.iter().map(|n| format!("{n:#x}")).collect();
        let actual_u16: Vec<_> = nums.iter().map(|n| format!("{n:X}")).collect();
        let actual_u16_alt: Vec<_> = nums.iter().map(|n| format!("{n:#X}")).collect();

        let expected_2: Vec<_> = POS_NUM.iter().map(|n| format!("{n:b}")).collect();
        let expected_2_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{n:#b}")).collect();
        let expected_8: Vec<_> = POS_NUM.iter().map(|n| format!("{n:o}")).collect();
        let expected_8_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{n:#o}")).collect();
        let expected_10: Vec<_> = POS_NUM.iter().map(|n| format!("{n:}")).collect();
        let expected_10_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{n:#}")).collect();
        let expected_l16: Vec<_> = POS_NUM.iter().map(|n| format!("{n:x}")).collect();
        let expected_l16_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{n:#x}")).collect();
        let expected_u16: Vec<_> = POS_NUM.iter().map(|n| format!("{n:X}")).collect();
        let expected_u16_alt: Vec<_> = POS_NUM.iter().map(|n| format!("{n:#X}")).collect();

        for (i, _) in POS_NUM.iter().enumerate() {
            assert_eq!(actual_2[i], expected_2[i]);
            assert_eq!(actual_2_alt[i], expected_2_alt[i]);

            assert_eq!(actual_8[i], expected_8[i]);
            assert_eq!(actual_8_alt[i], expected_8_alt[i]);

            assert_eq!(actual_10[i], expected_10[i]);
            assert_eq!(actual_10_alt[i], expected_10_alt[i]);

            assert_eq!(actual_l16[i], expected_l16[i]);
            assert_eq!(actual_l16_alt[i], expected_l16_alt[i]);

            assert_eq!(actual_u16[i], expected_u16[i]);
            assert_eq!(actual_u16_alt[i], expected_u16_alt[i]);
        }
    }
}
