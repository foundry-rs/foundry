use crate::{ParseSignedError, I256, U256};
use alloc::string::{String, ToString};
use core::fmt;

const MAX_U64_EXPONENT: u8 = 19;

/// Converts the input to a U256 and converts from Ether to Wei.
///
/// # Examples
///
/// ```
/// use alloy_primitives::{
///     utils::{parse_ether, Unit},
///     U256,
/// };
///
/// let eth = Unit::ETHER.wei();
/// assert_eq!(parse_ether("1").unwrap(), eth);
/// ```
pub fn parse_ether(eth: &str) -> Result<U256, UnitsError> {
    ParseUnits::parse_units(eth, Unit::ETHER).map(Into::into)
}

/// Parses a decimal number and multiplies it with 10^units.
///
/// # Examples
///
/// ```
/// use alloy_primitives::{utils::parse_units, U256};
///
/// let amount_in_eth = U256::from_str_radix("15230001000000000000", 10).unwrap();
/// let amount_in_gwei = U256::from_str_radix("15230001000", 10).unwrap();
/// let amount_in_wei = U256::from_str_radix("15230001000", 10).unwrap();
/// assert_eq!(amount_in_eth, parse_units("15.230001000000000000", "ether").unwrap().into());
/// assert_eq!(amount_in_gwei, parse_units("15.230001000000000000", "gwei").unwrap().into());
/// assert_eq!(amount_in_wei, parse_units("15230001000", "wei").unwrap().into());
/// ```
///
/// Example of trying to parse decimal WEI, which should fail, as WEI is the smallest
/// ETH denominator. 1 ETH = 10^18 WEI.
///
/// ```should_panic
/// use alloy_primitives::{utils::parse_units, U256};
/// let amount_in_wei = U256::from_str_radix("15230001000", 10).unwrap();
/// assert_eq!(amount_in_wei, parse_units("15.230001000000000000", "wei").unwrap().into());
/// ```
pub fn parse_units<K, E>(amount: &str, units: K) -> Result<ParseUnits, UnitsError>
where
    K: TryInto<Unit, Error = E>,
    UnitsError: From<E>,
{
    ParseUnits::parse_units(amount, units.try_into()?)
}

/// Formats the given number of Wei as an Ether amount.
///
/// # Examples
///
/// ```
/// use alloy_primitives::{utils::format_ether, U256};
///
/// let eth = format_ether(1395633240123456000_u128);
/// assert_eq!(format_ether(1395633240123456000_u128), "1.395633240123456000");
/// ```
pub fn format_ether<T: Into<ParseUnits>>(amount: T) -> String {
    amount.into().format_units(Unit::ETHER)
}

/// Formats the given number of Wei as the given unit.
///
/// # Examples
///
/// ```
/// use alloy_primitives::{utils::format_units, U256};
///
/// let eth = U256::from_str_radix("1395633240123456000", 10).unwrap();
/// assert_eq!(format_units(eth, "eth").unwrap(), "1.395633240123456000");
///
/// assert_eq!(format_units(i64::MIN, "gwei").unwrap(), "-9223372036.854775808");
///
/// assert_eq!(format_units(i128::MIN, 36).unwrap(), "-170.141183460469231731687303715884105728");
/// ```
pub fn format_units<T, K, E>(amount: T, units: K) -> Result<String, UnitsError>
where
    T: Into<ParseUnits>,
    K: TryInto<Unit, Error = E>,
    UnitsError: From<E>,
{
    units.try_into().map(|units| amount.into().format_units(units)).map_err(UnitsError::from)
}

/// Error type for [`Unit`]-related operations.
#[derive(Debug)]
pub enum UnitsError {
    /// The provided units are not recognized.
    InvalidUnit(String),
    /// Overflow when parsing a signed number.
    ParseSigned(ParseSignedError),
}

impl core::error::Error for UnitsError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidUnit(_) => None,
            Self::ParseSigned(e) => Some(e),
        }
    }
}

impl fmt::Display for UnitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUnit(s) => write!(f, "{s:?} is not a valid unit"),
            Self::ParseSigned(e) => e.fmt(f),
        }
    }
}

impl From<ruint::ParseError> for UnitsError {
    fn from(value: ruint::ParseError) -> Self {
        Self::ParseSigned(value.into())
    }
}

impl From<ParseSignedError> for UnitsError {
    fn from(value: ParseSignedError) -> Self {
        Self::ParseSigned(value)
    }
}

/// This enum holds the numeric types that a possible to be returned by `parse_units` and
/// that are taken by `format_units`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ParseUnits {
    /// Unsigned 256-bit integer.
    U256(U256),
    /// Signed 256-bit integer.
    I256(I256),
}

impl From<ParseUnits> for U256 {
    #[inline]
    fn from(value: ParseUnits) -> Self {
        value.get_absolute()
    }
}

impl From<ParseUnits> for I256 {
    #[inline]
    fn from(value: ParseUnits) -> Self {
        value.get_signed()
    }
}

impl fmt::Display for ParseUnits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U256(val) => val.fmt(f),
            Self::I256(val) => val.fmt(f),
        }
    }
}

macro_rules! impl_from_integers {
    ($convert:ident($($t:ty),* $(,)?)) => {$(
        impl From<$t> for ParseUnits {
            fn from(value: $t) -> Self {
                Self::$convert($convert::try_from(value).unwrap())
            }
        }
    )*}
}

impl_from_integers!(U256(u8, u16, u32, u64, u128, usize, U256));
impl_from_integers!(I256(i8, i16, i32, i64, i128, isize, I256));

macro_rules! impl_try_into_absolute {
    ($($t:ty),* $(,)?) => { $(
        impl TryFrom<ParseUnits> for $t {
            type Error = <$t as TryFrom<U256>>::Error;

            fn try_from(value: ParseUnits) -> Result<Self, Self::Error> {
                <$t>::try_from(value.get_absolute())
            }
        }
    )* };
}

impl_try_into_absolute!(u64, u128);

impl ParseUnits {
    /// Parses a decimal number and multiplies it with 10^units.
    ///
    /// See [`parse_units`] for more information.
    #[allow(clippy::self_named_constructors)]
    pub fn parse_units(amount: &str, unit: Unit) -> Result<Self, UnitsError> {
        let exponent = unit.get() as usize;

        let mut amount = amount.to_string();
        let negative = amount.starts_with('-');
        let dec_len = if let Some(di) = amount.find('.') {
            amount.remove(di);
            amount[di..].len()
        } else {
            0
        };
        let amount = amount.as_str();

        if dec_len > exponent {
            // Truncate the decimal part if it is longer than the exponent
            let amount = &amount[..(amount.len() - (dec_len - exponent))];
            if negative {
                // Edge case: We have removed the entire number and only the negative sign is left.
                //            Return 0 as a I256 given the input was signed.
                if amount == "-" {
                    Ok(Self::I256(I256::ZERO))
                } else {
                    Ok(Self::I256(I256::from_dec_str(amount)?))
                }
            } else {
                Ok(Self::U256(U256::from_str_radix(amount, 10)?))
            }
        } else if negative {
            // Edge case: Only a negative sign was given, return 0 as a I256 given the input was
            // signed.
            if amount == "-" {
                Ok(Self::I256(I256::ZERO))
            } else {
                let mut n = I256::from_dec_str(amount)?;
                n *= I256::try_from(10u8)
                    .unwrap()
                    .checked_pow(U256::from(exponent - dec_len))
                    .ok_or(UnitsError::ParseSigned(ParseSignedError::IntegerOverflow))?;
                Ok(Self::I256(n))
            }
        } else {
            let mut a_uint = U256::from_str_radix(amount, 10)?;
            a_uint *= U256::from(10)
                .checked_pow(U256::from(exponent - dec_len))
                .ok_or(UnitsError::ParseSigned(ParseSignedError::IntegerOverflow))?;
            Ok(Self::U256(a_uint))
        }
    }

    /// Formats the given number of Wei as the given unit.
    ///
    /// See [`format_units`] for more information.
    pub fn format_units(&self, mut unit: Unit) -> String {
        // Edge case: If the number is signed and the unit is the largest possible unit, we need to
        //            subtract 1 from the unit to avoid overflow.
        if self.is_signed() && unit == Unit::MAX {
            unit = Unit::new(Unit::MAX.get() - 1).unwrap();
        }
        let units = unit.get() as usize;
        let exp10 = unit.wei();

        // TODO: `decimals` are formatted twice because U256 does not support alignment
        // (`:0>width`).
        match *self {
            Self::U256(amount) => {
                let integer = amount / exp10;
                let decimals = (amount % exp10).to_string();
                format!("{integer}.{decimals:0>units$}")
            }
            Self::I256(amount) => {
                let exp10 = I256::from_raw(exp10);
                let sign = if amount.is_negative() { "-" } else { "" };
                let integer = (amount / exp10).twos_complement();
                let decimals = ((amount % exp10).twos_complement()).to_string();
                format!("{sign}{integer}.{decimals:0>units$}")
            }
        }
    }

    /// Returns `true` if the number is signed.
    #[inline]
    pub const fn is_signed(&self) -> bool {
        matches!(self, Self::I256(_))
    }

    /// Returns `true` if the number is unsigned.
    #[inline]
    pub const fn is_unsigned(&self) -> bool {
        matches!(self, Self::U256(_))
    }

    /// Returns `true` if the number is negative.
    #[inline]
    pub const fn is_negative(&self) -> bool {
        match self {
            Self::U256(_) => false,
            Self::I256(n) => n.is_negative(),
        }
    }

    /// Returns `true` if the number is positive.
    #[inline]
    pub const fn is_positive(&self) -> bool {
        match self {
            Self::U256(_) => true,
            Self::I256(n) => n.is_positive(),
        }
    }

    /// Returns `true` if the number is zero.
    #[inline]
    pub fn is_zero(&self) -> bool {
        match self {
            Self::U256(n) => n.is_zero(),
            Self::I256(n) => n.is_zero(),
        }
    }

    /// Returns the absolute value of the number.
    #[inline]
    pub const fn get_absolute(self) -> U256 {
        match self {
            Self::U256(n) => n,
            Self::I256(n) => n.into_raw(),
        }
    }

    /// Returns the signed value of the number.
    #[inline]
    pub const fn get_signed(self) -> I256 {
        match self {
            Self::U256(n) => I256::from_raw(n),
            Self::I256(n) => n,
        }
    }
}

/// Ethereum unit. Always less than [`77`](Unit::MAX).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unit(u8);

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl TryFrom<u8> for Unit {
    type Error = UnitsError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| UnitsError::InvalidUnit(value.to_string()))
    }
}

impl TryFrom<String> for Unit {
    type Error = UnitsError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl<'a> TryFrom<&'a String> for Unit {
    type Error = UnitsError;

    fn try_from(value: &'a String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<&str> for Unit {
    type Error = UnitsError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl core::str::FromStr for Unit {
    type Err = UnitsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(unit) = crate::U8::from_str(s) {
            return Self::new(unit.to()).ok_or_else(|| UnitsError::InvalidUnit(s.to_string()));
        }

        Ok(match s.to_ascii_lowercase().as_str() {
            "eth" | "ether" => Self::ETHER,
            "pwei" | "milli" | "milliether" | "finney" => Self::PWEI,
            "twei" | "micro" | "microether" | "szabo" => Self::TWEI,
            "gwei" | "nano" | "nanoether" | "shannon" => Self::GWEI,
            "mwei" | "pico" | "picoether" | "lovelace" => Self::MWEI,
            "kwei" | "femto" | "femtoether" | "babbage" => Self::KWEI,
            "wei" => Self::WEI,
            _ => return Err(UnitsError::InvalidUnit(s.to_string())),
        })
    }
}

impl Unit {
    /// Wei is equivalent to 1 wei.
    pub const WEI: Self = unsafe { Self::new_unchecked(0) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::WEI` instead")]
    pub const Wei: Self = Self::WEI;

    /// Kwei is equivalent to 1e3 wei.
    pub const KWEI: Self = unsafe { Self::new_unchecked(3) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::KWEI` instead")]
    pub const Kwei: Self = Self::KWEI;

    /// Mwei is equivalent to 1e6 wei.
    pub const MWEI: Self = unsafe { Self::new_unchecked(6) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::MWEI` instead")]
    pub const Mwei: Self = Self::MWEI;

    /// Gwei is equivalent to 1e9 wei.
    pub const GWEI: Self = unsafe { Self::new_unchecked(9) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::GWEI` instead")]
    pub const Gwei: Self = Self::GWEI;

    /// Twei is equivalent to 1e12 wei.
    pub const TWEI: Self = unsafe { Self::new_unchecked(12) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::TWEI` instead")]
    pub const Twei: Self = Self::TWEI;

    /// Pwei is equivalent to 1e15 wei.
    pub const PWEI: Self = unsafe { Self::new_unchecked(15) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::PWEI` instead")]
    pub const Pwei: Self = Self::PWEI;

    /// Ether is equivalent to 1e18 wei.
    pub const ETHER: Self = unsafe { Self::new_unchecked(18) };
    #[allow(non_upper_case_globals)]
    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `Unit::ETHER` instead")]
    pub const Ether: Self = Self::ETHER;

    /// The smallest unit.
    pub const MIN: Self = Self::WEI;
    /// The largest unit.
    pub const MAX: Self = unsafe { Self::new_unchecked(77) };

    /// Creates a new `Unit` instance, checking for overflow.
    #[inline]
    pub const fn new(units: u8) -> Option<Self> {
        if units <= Self::MAX.get() {
            // SAFETY: `units` is contained in the valid range.
            Some(unsafe { Self::new_unchecked(units) })
        } else {
            None
        }
    }

    /// Creates a new `Unit` instance.
    ///
    /// # Safety
    ///
    /// `x` must be less than [`Unit::MAX`].
    #[inline]
    pub const unsafe fn new_unchecked(x: u8) -> Self {
        Self(x)
    }

    /// Returns `10^self`, which is the number of Wei in this unit.
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_primitives::{utils::Unit, U256};
    ///
    /// assert_eq!(U256::from(1u128), Unit::WEI.wei());
    /// assert_eq!(U256::from(1_000u128), Unit::KWEI.wei());
    /// assert_eq!(U256::from(1_000_000u128), Unit::MWEI.wei());
    /// assert_eq!(U256::from(1_000_000_000u128), Unit::GWEI.wei());
    /// assert_eq!(U256::from(1_000_000_000_000u128), Unit::TWEI.wei());
    /// assert_eq!(U256::from(1_000_000_000_000_000u128), Unit::PWEI.wei());
    /// assert_eq!(U256::from(1_000_000_000_000_000_000u128), Unit::ETHER.wei());
    /// ```
    #[inline]
    pub fn wei(self) -> U256 {
        if self.get() <= MAX_U64_EXPONENT {
            self.wei_const()
        } else {
            U256::from(10u8).pow(U256::from(self.get()))
        }
    }

    /// Returns `10^self`, which is the number of Wei in this unit.
    ///
    /// # Panics
    ///
    /// Panics if `10^self` would overflow a `u64` (`self > 19`). If this can happen, use
    /// [`wei`](Self::wei) instead.
    #[inline]
    pub const fn wei_const(self) -> U256 {
        if self.get() > MAX_U64_EXPONENT {
            panic!("overflow")
        }
        U256::from_limbs([10u64.pow(self.get() as u32), 0, 0, 0])
    }

    /// Returns the numeric value of the unit.
    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }

    #[doc(hidden)]
    #[deprecated(since = "0.5.0", note = "use `get` instead")]
    pub const fn as_num(&self) -> u8 {
        self.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_values() {
        assert_eq!(Unit::WEI.get(), 0);
        assert_eq!(Unit::KWEI.get(), 3);
        assert_eq!(Unit::MWEI.get(), 6);
        assert_eq!(Unit::GWEI.get(), 9);
        assert_eq!(Unit::TWEI.get(), 12);
        assert_eq!(Unit::PWEI.get(), 15);
        assert_eq!(Unit::ETHER.get(), 18);
        assert_eq!(Unit::new(10).unwrap().get(), 10);
        assert_eq!(Unit::new(20).unwrap().get(), 20);
    }

    #[test]
    fn unit_wei() {
        let assert = |unit: Unit| {
            let wei = unit.wei();
            assert_eq!(wei.to::<u128>(), 10u128.pow(unit.get() as u32));
            assert_eq!(wei, U256::from(10u8).pow(U256::from(unit.get())));
        };
        assert(Unit::WEI);
        assert(Unit::KWEI);
        assert(Unit::MWEI);
        assert(Unit::GWEI);
        assert(Unit::TWEI);
        assert(Unit::PWEI);
        assert(Unit::ETHER);
        assert(Unit::new(10).unwrap());
        assert(Unit::new(20).unwrap());
    }

    #[test]
    fn parse() {
        assert_eq!(Unit::try_from("wei").unwrap(), Unit::WEI);
        assert_eq!(Unit::try_from("kwei").unwrap(), Unit::KWEI);
        assert_eq!(Unit::try_from("mwei").unwrap(), Unit::MWEI);
        assert_eq!(Unit::try_from("gwei").unwrap(), Unit::GWEI);
        assert_eq!(Unit::try_from("twei").unwrap(), Unit::TWEI);
        assert_eq!(Unit::try_from("pwei").unwrap(), Unit::PWEI);
        assert_eq!(Unit::try_from("ether").unwrap(), Unit::ETHER);
    }

    #[test]
    fn wei_in_ether() {
        assert_eq!(Unit::ETHER.wei(), U256::from(1e18 as u64));
    }

    #[test]
    fn test_format_ether_unsigned() {
        let eth = format_ether(Unit::ETHER.wei());
        assert_eq!(eth.parse::<f64>().unwrap() as u64, 1);

        let eth = format_ether(1395633240123456000_u128);
        assert_eq!(eth.parse::<f64>().unwrap(), 1.395633240123456);

        let eth = format_ether(U256::from_str_radix("1395633240123456000", 10).unwrap());
        assert_eq!(eth.parse::<f64>().unwrap(), 1.395633240123456);

        let eth = format_ether(U256::from_str_radix("1395633240123456789", 10).unwrap());
        assert_eq!(eth, "1.395633240123456789");

        let eth = format_ether(U256::from_str_radix("1005633240123456789", 10).unwrap());
        assert_eq!(eth, "1.005633240123456789");

        let eth = format_ether(u16::MAX);
        assert_eq!(eth, "0.000000000000065535");

        // Note: This covers usize on 32 bit systems.
        let eth = format_ether(u32::MAX);
        assert_eq!(eth, "0.000000004294967295");

        // Note: This covers usize on 64 bit systems.
        let eth = format_ether(u64::MAX);
        assert_eq!(eth, "18.446744073709551615");
    }

    #[test]
    fn test_format_ether_signed() {
        let eth = format_ether(I256::from_dec_str("-1395633240123456000").unwrap());
        assert_eq!(eth.parse::<f64>().unwrap(), -1.395633240123456);

        let eth = format_ether(I256::from_dec_str("-1395633240123456789").unwrap());
        assert_eq!(eth, "-1.395633240123456789");

        let eth = format_ether(I256::from_dec_str("1005633240123456789").unwrap());
        assert_eq!(eth, "1.005633240123456789");

        let eth = format_ether(i8::MIN);
        assert_eq!(eth, "-0.000000000000000128");

        let eth = format_ether(i8::MAX);
        assert_eq!(eth, "0.000000000000000127");

        let eth = format_ether(i16::MIN);
        assert_eq!(eth, "-0.000000000000032768");

        // Note: This covers isize on 32 bit systems.
        let eth = format_ether(i32::MIN);
        assert_eq!(eth, "-0.000000002147483648");

        // Note: This covers isize on 64 bit systems.
        let eth = format_ether(i64::MIN);
        assert_eq!(eth, "-9.223372036854775808");
    }

    #[test]
    fn test_format_units_unsigned() {
        let gwei_in_ether = format_units(Unit::ETHER.wei(), 9).unwrap();
        assert_eq!(gwei_in_ether.parse::<f64>().unwrap() as u64, 1e9 as u64);

        let eth = format_units(Unit::ETHER.wei(), "ether").unwrap();
        assert_eq!(eth.parse::<f64>().unwrap() as u64, 1);

        let eth = format_units(1395633240123456000_u128, "ether").unwrap();
        assert_eq!(eth.parse::<f64>().unwrap(), 1.395633240123456);

        let eth = format_units(U256::from_str_radix("1395633240123456000", 10).unwrap(), "ether")
            .unwrap();
        assert_eq!(eth.parse::<f64>().unwrap(), 1.395633240123456);

        let eth = format_units(U256::from_str_radix("1395633240123456789", 10).unwrap(), "ether")
            .unwrap();
        assert_eq!(eth, "1.395633240123456789");

        let eth = format_units(U256::from_str_radix("1005633240123456789", 10).unwrap(), "ether")
            .unwrap();
        assert_eq!(eth, "1.005633240123456789");

        let eth = format_units(u8::MAX, 4).unwrap();
        assert_eq!(eth, "0.0255");

        let eth = format_units(u16::MAX, "ether").unwrap();
        assert_eq!(eth, "0.000000000000065535");

        // Note: This covers usize on 32 bit systems.
        let eth = format_units(u32::MAX, 18).unwrap();
        assert_eq!(eth, "0.000000004294967295");

        // Note: This covers usize on 64 bit systems.
        let eth = format_units(u64::MAX, "gwei").unwrap();
        assert_eq!(eth, "18446744073.709551615");

        let eth = format_units(u128::MAX, 36).unwrap();
        assert_eq!(eth, "340.282366920938463463374607431768211455");

        let eth = format_units(U256::MAX, 77).unwrap();
        assert_eq!(
            eth,
            "1.15792089237316195423570985008687907853269984665640564039457584007913129639935"
        );

        let _err = format_units(U256::MAX, 78).unwrap_err();
        let _err = format_units(U256::MAX, 79).unwrap_err();
    }

    #[test]
    fn test_format_units_signed() {
        let eth =
            format_units(I256::from_dec_str("-1395633240123456000").unwrap(), "ether").unwrap();
        assert_eq!(eth.parse::<f64>().unwrap(), -1.395633240123456);

        let eth =
            format_units(I256::from_dec_str("-1395633240123456789").unwrap(), "ether").unwrap();
        assert_eq!(eth, "-1.395633240123456789");

        let eth =
            format_units(I256::from_dec_str("1005633240123456789").unwrap(), "ether").unwrap();
        assert_eq!(eth, "1.005633240123456789");

        let eth = format_units(i8::MIN, 4).unwrap();
        assert_eq!(eth, "-0.0128");
        assert_eq!(eth.parse::<f64>().unwrap(), -0.0128_f64);

        let eth = format_units(i8::MAX, 4).unwrap();
        assert_eq!(eth, "0.0127");
        assert_eq!(eth.parse::<f64>().unwrap(), 0.0127);

        let eth = format_units(i16::MIN, "ether").unwrap();
        assert_eq!(eth, "-0.000000000000032768");

        // Note: This covers isize on 32 bit systems.
        let eth = format_units(i32::MIN, 18).unwrap();
        assert_eq!(eth, "-0.000000002147483648");

        // Note: This covers isize on 64 bit systems.
        let eth = format_units(i64::MIN, "gwei").unwrap();
        assert_eq!(eth, "-9223372036.854775808");

        let eth = format_units(i128::MIN, 36).unwrap();
        assert_eq!(eth, "-170.141183460469231731687303715884105728");

        let eth = format_units(I256::MIN, 76).unwrap();
        let min = "-5.7896044618658097711785492504343953926634992332820282019728792003956564819968";
        assert_eq!(eth, min);
        // doesn't error
        let eth = format_units(I256::MIN, 77).unwrap();
        assert_eq!(eth, min);

        let _err = format_units(I256::MIN, 78).unwrap_err();
        let _err = format_units(I256::MIN, 79).unwrap_err();
    }

    #[test]
    fn parse_large_units() {
        let decimals = 27u8;
        let val = "10.55";

        let n: U256 = parse_units(val, decimals).unwrap().into();
        assert_eq!(n.to_string(), "10550000000000000000000000000");
    }

    #[test]
    fn test_parse_units() {
        let gwei: U256 = parse_units("1.5", 9).unwrap().into();
        assert_eq!(gwei, U256::from(15e8 as u64));

        let token: U256 = parse_units("1163.56926418", 8).unwrap().into();
        assert_eq!(token, U256::from(116356926418u64));

        let eth_dec_float: U256 = parse_units("1.39563324", "ether").unwrap().into();
        assert_eq!(eth_dec_float, U256::from_str_radix("1395633240000000000", 10).unwrap());

        let eth_dec_string: U256 = parse_units("1.39563324", "ether").unwrap().into();
        assert_eq!(eth_dec_string, U256::from_str_radix("1395633240000000000", 10).unwrap());

        let eth: U256 = parse_units("1", "ether").unwrap().into();
        assert_eq!(eth, Unit::ETHER.wei());

        let val: U256 = parse_units("2.3", "ether").unwrap().into();
        assert_eq!(val, U256::from_str_radix("2300000000000000000", 10).unwrap());

        let n: U256 = parse_units(".2", 2).unwrap().into();
        assert_eq!(n, U256::from(20), "leading dot");

        let n: U256 = parse_units("333.21", 2).unwrap().into();
        assert_eq!(n, U256::from(33321), "trailing dot");

        let n: U256 = parse_units("98766", 16).unwrap().into();
        assert_eq!(n, U256::from_str_radix("987660000000000000000", 10).unwrap(), "no dot");

        let n: U256 = parse_units("3_3_0", 3).unwrap().into();
        assert_eq!(n, U256::from(330000), "underscore");

        let n: U256 = parse_units("330", 0).unwrap().into();
        assert_eq!(n, U256::from(330), "zero decimals");

        let n: U256 = parse_units(".1234", 3).unwrap().into();
        assert_eq!(n, U256::from(123), "truncate too many decimals");

        assert!(parse_units("1", 80).is_err(), "overflow");

        let two_e30 = U256::from(2) * U256::from_limbs([0x4674edea40000000, 0xc9f2c9cd0, 0x0, 0x0]);
        let n: U256 = parse_units("2", 30).unwrap().into();
        assert_eq!(n, two_e30, "2e30");

        let n: U256 = parse_units(".33_319_2", 0).unwrap().into();
        assert_eq!(n, U256::ZERO, "mix");

        let n: U256 = parse_units("", 3).unwrap().into();
        assert_eq!(n, U256::ZERO, "empty");
    }

    #[test]
    fn test_signed_parse_units() {
        let gwei: I256 = parse_units("-1.5", 9).unwrap().into();
        assert_eq!(gwei.as_i64(), -15e8 as i64);

        let token: I256 = parse_units("-1163.56926418", 8).unwrap().into();
        assert_eq!(token.as_i64(), -116356926418);

        let eth_dec_float: I256 = parse_units("-1.39563324", "ether").unwrap().into();
        assert_eq!(eth_dec_float, I256::from_dec_str("-1395633240000000000").unwrap());

        let eth_dec_string: I256 = parse_units("-1.39563324", "ether").unwrap().into();
        assert_eq!(eth_dec_string, I256::from_dec_str("-1395633240000000000").unwrap());

        let eth: I256 = parse_units("-1", "ether").unwrap().into();
        assert_eq!(eth, I256::from_raw(Unit::ETHER.wei()) * I256::MINUS_ONE);

        let val: I256 = parse_units("-2.3", "ether").unwrap().into();
        assert_eq!(val, I256::from_dec_str("-2300000000000000000").unwrap());

        let n: I256 = parse_units("-.2", 2).unwrap().into();
        assert_eq!(n, I256::try_from(-20).unwrap(), "leading dot");

        let n: I256 = parse_units("-333.21", 2).unwrap().into();
        assert_eq!(n, I256::try_from(-33321).unwrap(), "trailing dot");

        let n: I256 = parse_units("-98766", 16).unwrap().into();
        assert_eq!(n, I256::from_dec_str("-987660000000000000000").unwrap(), "no dot");

        let n: I256 = parse_units("-3_3_0", 3).unwrap().into();
        assert_eq!(n, I256::try_from(-330000).unwrap(), "underscore");

        let n: I256 = parse_units("-330", 0).unwrap().into();
        assert_eq!(n, I256::try_from(-330).unwrap(), "zero decimals");

        let n: I256 = parse_units("-.1234", 3).unwrap().into();
        assert_eq!(n, I256::try_from(-123).unwrap(), "truncate too many decimals");

        assert!(parse_units("-1", 80).is_err(), "overflow");

        let two_e30 = I256::try_from(-2).unwrap()
            * I256::from_raw(U256::from_limbs([0x4674edea40000000, 0xc9f2c9cd0, 0x0, 0x0]));
        let n: I256 = parse_units("-2", 30).unwrap().into();
        assert_eq!(n, two_e30, "-2e30");

        let n: I256 = parse_units("-.33_319_2", 0).unwrap().into();
        assert_eq!(n, I256::ZERO, "mix");

        let n: I256 = parse_units("-", 3).unwrap().into();
        assert_eq!(n, I256::ZERO, "empty");
    }
}
