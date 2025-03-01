use crate::{dynamic::ty::as_tuple, DynSolType, DynSolValue, Result};
use alloc::vec::Vec;
use alloy_primitives::{Address, Function, Sign, I256, U256};
use alloy_sol_types::Word;
use core::fmt;
use parser::{
    new_input,
    utils::{array_parser, char_parser, spanned},
    Input,
};
use winnow::{
    ascii::{alpha0, alpha1, digit1, hex_digit0, hex_digit1, space0},
    combinator::{cut_err, dispatch, empty, fail, opt, preceded, trace},
    error::{
        AddContext, ContextError, ErrMode, FromExternalError, ParserError, StrContext,
        StrContextValue,
    },
    stream::Stream,
    token::take_while,
    ModalParser, ModalResult, Parser,
};

impl DynSolType {
    /// Coerces a string into a [`DynSolValue`] via this type.
    ///
    /// # Syntax
    ///
    /// - [`Bool`](DynSolType::Bool): `true|false`
    /// - [`Int`](DynSolType::Int): `[+-]?{Uint}`
    /// - [`Uint`](DynSolType::Uint): `{literal}(\.[0-9]+)?(\s*{unit})?`
    ///   - literal: base 2, 8, 10, or 16 integer literal. If not in base 10, must be prefixed with
    ///     `0b`, `0o`, or `0x` respectively.
    ///   - unit: same as [Solidity ether units](https://docs.soliditylang.org/en/latest/units-and-global-variables.html#ether-units)
    ///   - decimals with more digits than the unit's exponent value are not allowed
    /// - [`FixedBytes`](DynSolType::FixedBytes): `(0x)?[0-9A-Fa-f]{$0*2}`
    /// - [`Address`](DynSolType::Address): `(0x)?[0-9A-Fa-f]{40}`
    /// - [`Function`](DynSolType::Function): `(0x)?[0-9A-Fa-f]{48}`
    /// - [`Bytes`](DynSolType::Bytes): `(0x)?[0-9A-Fa-f]+`
    /// - [`String`](DynSolType::String): `.*`
    ///   - can be surrounded by a pair of `"` or `'`
    ///   - trims whitespace if not surrounded
    /// - [`Array`](DynSolType::Array): any number of the inner type delimited by commas (`,`) and
    ///   surrounded by brackets (`[]`)
    /// - [`FixedArray`](DynSolType::FixedArray): exactly the given number of the inner type
    ///   delimited by commas (`,`) and surrounded by brackets (`[]`)
    /// - [`Tuple`](DynSolType::Tuple): the inner types delimited by commas (`,`) and surrounded by
    ///   parentheses (`()`)
    #[cfg_attr(
        feature = "eip712",
        doc = "- [`CustomStruct`](DynSolType::CustomStruct): the same as `Tuple`"
    )]
    ///
    /// # Examples
    ///
    /// ```
    /// use alloy_dyn_abi::{DynSolType, DynSolValue};
    /// use alloy_primitives::U256;
    ///
    /// let ty: DynSolType = "(uint256,string)[]".parse()?;
    /// let value = ty.coerce_str("[(0, \"hello\"), (4.2e1, \"world\")]")?;
    /// assert_eq!(
    ///     value,
    ///     DynSolValue::Array(vec![
    ///         DynSolValue::Tuple(vec![
    ///             DynSolValue::Uint(U256::from(0), 256),
    ///             DynSolValue::String(String::from("hello"))
    ///         ]),
    ///         DynSolValue::Tuple(vec![
    ///             DynSolValue::Uint(U256::from(42), 256),
    ///             DynSolValue::String(String::from("world"))
    ///         ]),
    ///     ])
    /// );
    /// assert!(value.matches(&ty));
    /// assert_eq!(value.as_type().unwrap(), ty);
    /// # Ok::<_, alloy_dyn_abi::Error>(())
    /// ```
    #[doc(alias = "tokenize")] // from ethabi
    pub fn coerce_str(&self, s: &str) -> Result<DynSolValue> {
        ValueParser::new(self)
            .parse(new_input(s))
            .map_err(|e| crate::Error::TypeParser(parser::Error::parser(e)))
    }
}

struct ValueParser<'a> {
    ty: &'a DynSolType,
    list_end: Option<char>,
}

impl<'i> Parser<Input<'i>, DynSolValue, ErrMode<ContextError>> for ValueParser<'_> {
    fn parse_next(&mut self, input: &mut Input<'i>) -> ModalResult<DynSolValue, ContextError> {
        #[cfg(feature = "debug")]
        let name = self.ty.sol_type_name();
        #[cfg(not(feature = "debug"))]
        let name = "value_parser";
        trace(name, move |input: &mut Input<'i>| match self.ty {
            DynSolType::Bool => bool(input).map(DynSolValue::Bool),
            &DynSolType::Int(size) => {
                int(size).parse_next(input).map(|int| DynSolValue::Int(int, size))
            }
            &DynSolType::Uint(size) => {
                uint(size).parse_next(input).map(|uint| DynSolValue::Uint(uint, size))
            }
            &DynSolType::FixedBytes(size) => {
                fixed_bytes(size).parse_next(input).map(|word| DynSolValue::FixedBytes(word, size))
            }
            DynSolType::Address => address(input).map(DynSolValue::Address),
            DynSolType::Function => function(input).map(DynSolValue::Function),
            DynSolType::Bytes => bytes(input).map(DynSolValue::Bytes),
            DynSolType::String => {
                self.string().parse_next(input).map(|s| DynSolValue::String(s.into()))
            }
            DynSolType::Array(ty) => self.in_list(']', |this| {
                this.with(ty).array().parse_next(input).map(DynSolValue::Array)
            }),
            DynSolType::FixedArray(ty, len) => self.in_list(']', |this| {
                this.with(ty).fixed_array(*len).parse_next(input).map(DynSolValue::FixedArray)
            }),
            as_tuple!(DynSolType tys) => {
                self.in_list(')', |this| this.tuple(tys).parse_next(input).map(DynSolValue::Tuple))
            }
        })
        .parse_next(input)
    }
}

impl<'a> ValueParser<'a> {
    #[inline]
    const fn new(ty: &'a DynSolType) -> Self {
        Self { list_end: None, ty }
    }

    #[inline]
    fn in_list<F: FnOnce(&mut Self) -> R, R>(&mut self, list_end: char, f: F) -> R {
        let prev = core::mem::replace(&mut self.list_end, Some(list_end));
        let r = f(self);
        self.list_end = prev;
        r
    }

    #[inline]
    const fn with(&self, ty: &'a DynSolType) -> Self {
        Self { list_end: self.list_end, ty }
    }

    #[inline]
    fn string<'s, 'i: 's>(&'s self) -> impl ModalParser<Input<'i>, &'i str, ContextError> + 's {
        trace("string", |input: &mut Input<'i>| {
            let Some(delim) = input.chars().next() else {
                return Ok("");
            };
            let has_delim = matches!(delim, '"' | '\'');
            if has_delim {
                let _ = input.next_token();
            }

            // TODO: escapes?
            let mut s = if has_delim || self.list_end.is_some() {
                let (chs, l) = if has_delim {
                    ([delim, '\0'], 1)
                } else if let Some(c) = self.list_end {
                    ([',', c], 2)
                } else {
                    unreachable!()
                };
                let min = if has_delim { 0 } else { 1 };
                take_while(min.., move |c: char| !unsafe { chs.get_unchecked(..l) }.contains(&c))
                    .parse_next(input)?
            } else {
                input.next_slice(input.len())
            };

            if has_delim {
                cut_err(char_parser(delim))
                    .context(StrContext::Label("string"))
                    .parse_next(input)?;
            } else {
                s = s.trim_end();
            }

            Ok(s)
        })
    }

    #[inline]
    fn array<'i: 'a>(self) -> impl ModalParser<Input<'i>, Vec<DynSolValue>, ContextError> + 'a {
        #[cfg(feature = "debug")]
        let name = format!("{}[]", self.ty);
        #[cfg(not(feature = "debug"))]
        let name = "array";
        trace(name, array_parser(self))
    }

    #[inline]
    fn fixed_array<'i: 'a>(
        self,
        len: usize,
    ) -> impl ModalParser<Input<'i>, Vec<DynSolValue>, ContextError> + 'a {
        #[cfg(feature = "debug")]
        let name = format!("{}[{len}]", self.ty);
        #[cfg(not(feature = "debug"))]
        let name = "fixed_array";
        trace(
            name,
            array_parser(self).try_map(move |values: Vec<DynSolValue>| {
                if values.len() == len {
                    Ok(values)
                } else {
                    Err(Error::FixedArrayLengthMismatch(len, values.len()))
                }
            }),
        )
    }

    #[inline]
    #[allow(clippy::ptr_arg)]
    fn tuple<'i: 's, 't: 's, 's>(
        &'s self,
        tuple: &'t Vec<DynSolType>,
    ) -> impl ModalParser<Input<'i>, Vec<DynSolValue>, ContextError> + 's {
        #[cfg(feature = "debug")]
        let name = DynSolType::Tuple(tuple.clone()).to_string();
        #[cfg(not(feature = "debug"))]
        let name = "tuple";
        trace(name, move |input: &mut Input<'i>| {
            space0(input)?;
            char_parser('(').parse_next(input)?;

            let mut values = Vec::with_capacity(tuple.len());
            for (i, ty) in tuple.iter().enumerate() {
                if i > 0 {
                    space0(input)?;
                    char_parser(',').parse_next(input)?;
                }
                space0(input)?;
                values.push(self.with(ty).parse_next(input)?);
            }

            space0(input)?;
            char_parser(')').parse_next(input)?;

            Ok(values)
        })
    }
}

#[derive(Debug)]
enum Error {
    IntOverflow,
    FractionalNotAllowed(U256),
    NegativeUnits,
    TooManyDecimals(usize, usize),
    InvalidFixedBytesLength(usize),
    FixedArrayLengthMismatch(usize, usize),
    EmptyHexStringWithoutPrefix,
}

impl core::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntOverflow => f.write_str("number too large to fit in target type"),
            Self::TooManyDecimals(expected, actual) => {
                write!(f, "expected at most {expected} decimals, got {actual}")
            }
            Self::FractionalNotAllowed(n) => write!(f, "non-zero fraction .{n} not allowed"),
            Self::NegativeUnits => f.write_str("negative units not allowed"),
            Self::InvalidFixedBytesLength(len) => {
                write!(f, "fixed bytes length {len} greater than 32")
            }
            Self::FixedArrayLengthMismatch(expected, actual) => {
                write!(f, "fixed array length mismatch: expected {expected} elements, got {actual}")
            }
            Self::EmptyHexStringWithoutPrefix => {
                f.write_str("expected hex digits or the `0x` prefix for an empty hex string")
            }
        }
    }
}

#[inline]
fn bool(input: &mut Input<'_>) -> ModalResult<bool> {
    trace(
        "bool",
        dispatch! {alpha1.context(StrContext::Label("boolean"));
            "true" => empty.value(true),
            "false" => empty.value(false),
            _ => fail
        }
        .context(StrContext::Label("boolean")),
    )
    .parse_next(input)
}

#[inline]
fn int<'i>(size: usize) -> impl ModalParser<Input<'i>, I256, ContextError> {
    #[cfg(feature = "debug")]
    let name = format!("int{size}");
    #[cfg(not(feature = "debug"))]
    let name = "int";
    trace(
        name,
        (int_sign, uint(size)).try_map(move |(sign, abs)| {
            if !sign.is_negative() && abs.bit_len() > size - 1 {
                return Err(Error::IntOverflow);
            }
            I256::checked_from_sign_and_abs(sign, abs).ok_or(Error::IntOverflow)
        }),
    )
}

#[inline]
fn int_sign(input: &mut Input<'_>) -> ModalResult<Sign> {
    trace("int_sign", |input: &mut Input<'_>| match input.as_bytes().first() {
        Some(b'+') => {
            let _ = input.next_slice(1);
            Ok(Sign::Positive)
        }
        Some(b'-') => {
            let _ = input.next_slice(1);
            Ok(Sign::Negative)
        }
        Some(_) | None => Ok(Sign::Positive),
    })
    .parse_next(input)
}

#[inline]
fn uint<'i>(len: usize) -> impl ModalParser<Input<'i>, U256, ContextError> {
    #[cfg(feature = "debug")]
    let name = format!("uint{len}");
    #[cfg(not(feature = "debug"))]
    let name = "uint";
    trace(name, move |input: &mut Input<'_>| {
        let intpart = prefixed_int(input)?;
        let fract =
            opt(preceded(
                '.',
                cut_err(digit1.context(StrContext::Expected(StrContextValue::Description(
                    "at least one digit",
                )))),
            ))
            .parse_next(input)?;

        let intpart =
            intpart.parse::<U256>().map_err(|e| ErrMode::from_external_error(input, e))?;
        let e = opt(scientific_notation).parse_next(input)?.unwrap_or(0);

        let _ = space0(input)?;
        let units = int_units(input)?;

        let units = units as isize + e;
        if units < 0 {
            return Err(ErrMode::from_external_error(input, Error::NegativeUnits));
        }
        let units = units as usize;

        let uint = if let Some(fract) = fract {
            let fract_uint = U256::from_str_radix(fract, 10)
                .map_err(|e| ErrMode::from_external_error(input, e))?;

            if units == 0 && !fract_uint.is_zero() {
                return Err(ErrMode::from_external_error(
                    input,
                    Error::FractionalNotAllowed(fract_uint),
                ));
            }

            if fract.len() > units {
                return Err(ErrMode::from_external_error(
                    input,
                    Error::TooManyDecimals(units, fract.len()),
                ));
            }

            // (intpart * 10^fract.len() + fract) * 10^(units-fract.len())
            (|| -> Option<U256> {
                let extension = U256::from(10u64).checked_pow(U256::from(fract.len()))?;
                let extended = intpart.checked_mul(extension)?;
                let uint = fract_uint.checked_add(extended)?;
                let units = U256::from(10u64).checked_pow(U256::from(units - fract.len()))?;
                uint.checked_mul(units)
            })()
        } else if units > 0 {
            // intpart * 10^units
            (|| -> Option<U256> {
                let units = U256::from(10u64).checked_pow(U256::from(units))?;
                intpart.checked_mul(units)
            })()
        } else {
            Some(intpart)
        }
        .ok_or_else(|| ErrMode::from_external_error(input, Error::IntOverflow))?;

        if uint.bit_len() > len {
            return Err(ErrMode::from_external_error(input, Error::IntOverflow));
        }

        Ok(uint)
    })
}

#[inline]
fn prefixed_int<'i>(input: &mut Input<'i>) -> ModalResult<&'i str> {
    trace(
        "prefixed_int",
        spanned(|input: &mut Input<'i>| {
            let has_prefix =
                matches!(input.get(..2), Some("0b" | "0B" | "0o" | "0O" | "0x" | "0X"));
            let checkpoint = input.checkpoint();
            if has_prefix {
                let _ = input.next_slice(2);
                // parse hex since it's the most general
                hex_digit1(input)
            } else {
                digit1(input)
            }
            .map_err(|e: ErrMode<_>| {
                e.add_context(
                    input,
                    &checkpoint,
                    StrContext::Expected(StrContextValue::Description("at least one digit")),
                )
            })
        }),
    )
    .parse_next(input)
    .map(|(s, _)| s)
}

#[inline]
fn int_units(input: &mut Input<'_>) -> ModalResult<usize> {
    trace(
        "int_units",
        dispatch! {alpha0;
            "ether" => empty.value(18),
            "gwei" | "nano" | "nanoether" => empty.value(9),
            "" | "wei" => empty.value(0),
            _ => fail,
        },
    )
    .parse_next(input)
}

#[inline]
fn scientific_notation(input: &mut Input<'_>) -> ModalResult<isize> {
    // Check if we have 'e' or 'E' followed by an optional sign and digits
    if !matches!(input.chars().next(), Some('e' | 'E')) {
        return Err(ErrMode::from_input(input));
    }
    let _ = input.next_token();
    winnow::ascii::dec_int(input)
}

#[inline]
fn fixed_bytes<'i>(len: usize) -> impl ModalParser<Input<'i>, Word, ContextError> {
    #[cfg(feature = "debug")]
    let name = format!("bytes{len}");
    #[cfg(not(feature = "debug"))]
    let name = "bytesN";
    trace(name, move |input: &mut Input<'_>| {
        if len > Word::len_bytes() {
            return Err(
                ErrMode::from_external_error(input, Error::InvalidFixedBytesLength(len)).cut()
            );
        }

        let hex = hex_str(input)?;
        let mut out = Word::ZERO;
        match hex::decode_to_slice(hex, &mut out[..len]) {
            Ok(()) => Ok(out),
            Err(e) => Err(ErrMode::from_external_error(input, e).cut()),
        }
    })
}

#[inline]
fn address(input: &mut Input<'_>) -> ModalResult<Address> {
    trace("address", hex_str.try_map(hex::FromHex::from_hex)).parse_next(input)
}

#[inline]
fn function(input: &mut Input<'_>) -> ModalResult<Function> {
    trace("function", hex_str.try_map(hex::FromHex::from_hex)).parse_next(input)
}

#[inline]
fn bytes(input: &mut Input<'_>) -> ModalResult<Vec<u8>> {
    trace("bytes", hex_str.try_map(hex::decode)).parse_next(input)
}

#[inline]
fn hex_str<'i>(input: &mut Input<'i>) -> ModalResult<&'i str> {
    trace("hex_str", |input: &mut Input<'i>| {
        // Allow empty `bytes` only with a prefix.
        let has_prefix = opt("0x").parse_next(input)?.is_some();
        let s = hex_digit0(input)?;
        if !has_prefix && s.is_empty() {
            return Err(ErrMode::from_external_error(input, Error::EmptyHexStringWithoutPrefix));
        }
        Ok(s)
    })
    .parse_next(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{
        boxed::Box,
        string::{String, ToString},
    };
    use alloy_primitives::address;
    use core::str::FromStr;

    fn uint_test(s: &str, expected: Result<&str, ()>) {
        for (ty, negate) in [
            (DynSolType::Uint(256), false),
            (DynSolType::Int(256), false),
            (DynSolType::Int(256), true),
        ] {
            let s = if negate { &format!("-{s}") } else { s };
            let expected = if negate {
                expected.map(|s| format!("-{s}"))
            } else {
                expected.map(|s| s.to_string())
            };
            let d = format!("{s:?} as {ty:?}");

            let actual = ty.coerce_str(s);
            match (actual, expected) {
                (Ok(actual), Ok(expected)) => match (actual, ty) {
                    (DynSolValue::Uint(v, 256), DynSolType::Uint(256)) => {
                        assert_eq!(v, expected.parse::<U256>().unwrap(), "{d}");
                    }
                    (DynSolValue::Int(v, 256), DynSolType::Int(256)) => {
                        assert_eq!(v, expected.parse::<I256>().unwrap(), "{d}");
                    }
                    (actual, _) => panic!("{d}: unexpected value: {actual:?}"),
                },
                (Err(_), Err(())) => {}
                (Ok(actual), Err(_)) => panic!("{d}: expected failure, got {actual:?}"),
                (Err(e), Ok(_)) => panic!("{d}: {e:?}"),
            }
        }
    }

    #[track_caller]
    fn assert_error_contains(e: &impl core::fmt::Display, s: &str) {
        if cfg!(feature = "std") {
            let es = e.to_string();
            assert!(es.contains(s), "{s:?} not in {es:?}");
        }
    }

    #[test]
    fn coerce_bool() {
        assert_eq!(DynSolType::Bool.coerce_str("true").unwrap(), DynSolValue::Bool(true));
        assert_eq!(DynSolType::Bool.coerce_str("false").unwrap(), DynSolValue::Bool(false));

        assert!(DynSolType::Bool.coerce_str("").is_err());
        assert!(DynSolType::Bool.coerce_str("0").is_err());
        assert!(DynSolType::Bool.coerce_str("1").is_err());
        assert!(DynSolType::Bool.coerce_str("tru").is_err());
    }

    #[test]
    fn coerce_int() {
        assert_eq!(
            DynSolType::Int(256)
                .coerce_str("0x1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap(),
            DynSolValue::Int(I256::from_be_bytes([0x11; 32]), 256)
        );

        assert_eq!(
            DynSolType::Int(256)
                .coerce_str("0x2222222222222222222222222222222222222222222222222222222222222222")
                .unwrap(),
            DynSolValue::Int(I256::from_be_bytes([0x22; 32]), 256)
        );

        assert_eq!(
            DynSolType::Int(256)
                .coerce_str("0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
                .unwrap(),
            DynSolValue::Int(I256::MAX, 256)
        );
        assert!(DynSolType::Int(256)
            .coerce_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .is_err());

        assert_eq!(
            DynSolType::Int(256).coerce_str("0").unwrap(),
            DynSolValue::Int(I256::ZERO, 256)
        );

        assert_eq!(
            DynSolType::Int(256).coerce_str("-0").unwrap(),
            DynSolValue::Int(I256::ZERO, 256)
        );

        assert_eq!(
            DynSolType::Int(256).coerce_str("+0").unwrap(),
            DynSolValue::Int(I256::ZERO, 256)
        );

        assert_eq!(
            DynSolType::Int(256).coerce_str("-1").unwrap(),
            DynSolValue::Int(I256::MINUS_ONE, 256)
        );

        assert_eq!(
            DynSolType::Int(256)
                .coerce_str(
                    "57896044618658097711785492504343953926634992332820282019728792003956564819967"
                )
                .unwrap(),
            DynSolValue::Int(I256::MAX, 256)
        );
        assert_eq!(
            DynSolType::Int(256).coerce_str("-57896044618658097711785492504343953926634992332820282019728792003956564819968").unwrap(),
            DynSolValue::Int(I256::MIN, 256)
        );
    }

    #[test]
    fn coerce_int_overflow() {
        assert_eq!(
            DynSolType::Int(8).coerce_str("126").unwrap(),
            DynSolValue::Int(I256::try_from(126).unwrap(), 8),
        );
        assert_eq!(
            DynSolType::Int(8).coerce_str("127").unwrap(),
            DynSolValue::Int(I256::try_from(127).unwrap(), 8),
        );
        assert!(DynSolType::Int(8).coerce_str("128").is_err());
        assert!(DynSolType::Int(8).coerce_str("129").is_err());
        assert_eq!(
            DynSolType::Int(16).coerce_str("128").unwrap(),
            DynSolValue::Int(I256::try_from(128).unwrap(), 16),
        );
        assert_eq!(
            DynSolType::Int(16).coerce_str("129").unwrap(),
            DynSolValue::Int(I256::try_from(129).unwrap(), 16),
        );

        assert_eq!(
            DynSolType::Int(8).coerce_str("-1").unwrap(),
            DynSolValue::Int(I256::MINUS_ONE, 8),
        );
        assert_eq!(
            DynSolType::Int(16).coerce_str("-1").unwrap(),
            DynSolValue::Int(I256::MINUS_ONE, 16),
        );
    }

    #[test]
    fn coerce_uint() {
        assert_eq!(
            DynSolType::Uint(256)
                .coerce_str("0x1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap(),
            DynSolValue::Uint(U256::from_be_bytes([0x11; 32]), 256)
        );

        assert_eq!(
            DynSolType::Uint(256)
                .coerce_str("0x2222222222222222222222222222222222222222222222222222222222222222")
                .unwrap(),
            DynSolValue::Uint(U256::from_be_bytes([0x22; 32]), 256)
        );

        assert_eq!(
            DynSolType::Uint(256)
                .coerce_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
                .unwrap(),
            DynSolValue::Uint(U256::from_be_bytes([0xff; 32]), 256)
        );

        // 255 bits fails
        assert!(DynSolType::Uint(255)
            .coerce_str("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .is_err());

        assert_eq!(
            DynSolType::Uint(256)
                .coerce_str("115792089237316195423570985008687907853269984665640564039457584007913129639935")
                .unwrap(),
            DynSolValue::Uint(U256::MAX, 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0").unwrap(),
            DynSolValue::Uint(U256::ZERO, 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("1").unwrap(),
            DynSolValue::Uint(U256::from(1), 256)
        );
    }

    #[test]
    fn coerce_uint_overflow() {
        assert_eq!(
            DynSolType::Uint(8).coerce_str("254").unwrap(),
            DynSolValue::Uint(U256::from(254), 8),
        );
        assert_eq!(
            DynSolType::Uint(8).coerce_str("255").unwrap(),
            DynSolValue::Uint(U256::from(255), 8),
        );
        assert!(DynSolType::Uint(8).coerce_str("256").is_err());
        assert!(DynSolType::Uint(8).coerce_str("257").is_err());
        assert_eq!(
            DynSolType::Uint(16).coerce_str("256").unwrap(),
            DynSolValue::Uint(U256::from(256), 16),
        );
        assert_eq!(
            DynSolType::Uint(16).coerce_str("257").unwrap(),
            DynSolValue::Uint(U256::from(257), 16),
        );
    }

    #[test]
    fn coerce_uint_wei() {
        assert_eq!(
            DynSolType::Uint(256).coerce_str("1wei").unwrap(),
            DynSolValue::Uint(U256::from(1), 256)
        );
        assert_eq!(
            DynSolType::Uint(256).coerce_str("1 wei").unwrap(),
            DynSolValue::Uint(U256::from(1), 256)
        );

        assert!(DynSolType::Uint(256).coerce_str("1").is_ok());
        assert!(DynSolType::Uint(256).coerce_str("1.").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1 .").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1 .0").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1.wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1. wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1.0wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1.0 wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1.00wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("1.00 wei").is_err());
    }

    #[test]
    fn coerce_uint_gwei() {
        assert_eq!(
            DynSolType::Uint(256).coerce_str("1nano").unwrap(),
            DynSolValue::Uint(U256::from_str("1000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("1nanoether").unwrap(),
            DynSolValue::Uint(U256::from_str("1000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("1gwei").unwrap(),
            DynSolValue::Uint(U256::from_str("1000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.1 gwei").unwrap(),
            DynSolValue::Uint(U256::from_str("100000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.000000001gwei").unwrap(),
            DynSolValue::Uint(U256::from(1), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.123456789gwei").unwrap(),
            DynSolValue::Uint(U256::from_str("123456789").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("123456789123.123456789gwei").unwrap(),
            DynSolValue::Uint(U256::from_str("123456789123123456789").unwrap(), 256)
        );
    }

    #[test]
    fn coerce_uint_ether() {
        assert_eq!(
            DynSolType::Uint(256).coerce_str("10000000000ether").unwrap(),
            DynSolValue::Uint(U256::from_str("10000000000000000000000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("1ether").unwrap(),
            DynSolValue::Uint(U256::from_str("1000000000000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.01 ether").unwrap(),
            DynSolValue::Uint(U256::from_str("10000000000000000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.000000000000000001ether").unwrap(),
            DynSolValue::Uint(U256::from(1), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.000000000000000001ether"),
            DynSolType::Uint(256).coerce_str("1wei"),
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.123456789123456789ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456789123456789").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.123456789123456000ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456789123456000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("0.1234567891234560ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456789123456000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("123456.123456789123456789ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456123456789123456789").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("123456.123456789123456000ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456123456789123456000").unwrap(), 256)
        );

        assert_eq!(
            DynSolType::Uint(256).coerce_str("123456.1234567891234560ether").unwrap(),
            DynSolValue::Uint(U256::from_str("123456123456789123456000").unwrap(), 256)
        );
    }

    #[test]
    fn coerce_uint_array_ether() {
        assert_eq!(
            DynSolType::Array(Box::new(DynSolType::Uint(256)))
                .coerce_str("[ 1   ether,  10 ether ]")
                .unwrap(),
            DynSolValue::Array(vec![
                DynSolValue::Uint(U256::from_str("1000000000000000000").unwrap(), 256),
                DynSolValue::Uint(U256::from_str("10000000000000000000").unwrap(), 256),
            ])
        );
    }

    #[test]
    fn coerce_uint_invalid_units() {
        // 0.1 wei
        assert!(DynSolType::Uint(256).coerce_str("0.1 wei").is_err());
        assert!(DynSolType::Uint(256).coerce_str("0.0000000000000000001ether").is_err());

        // 1 ether + 0.1 wei
        assert!(DynSolType::Uint(256).coerce_str("1.0000000000000000001ether").is_err());

        // 1_000_000_000 ether + 0.1 wei
        assert!(DynSolType::Uint(256).coerce_str("1000000000.0000000000000000001ether").is_err());

        assert!(DynSolType::Uint(256).coerce_str("0..1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str("..1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str("1. gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str(".1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str("2.1.1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str(".1.1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str("1abc").is_err());

        assert!(DynSolType::Uint(256).coerce_str("1 gwei ").is_err());

        assert!(DynSolType::Uint(256).coerce_str("g 1 gwei").is_err());

        assert!(DynSolType::Uint(256).coerce_str("1gwei 1 gwei").is_err());
    }

    #[test]
    fn coerce_fixed_bytes() {
        let mk_word = |sl: &[u8]| {
            let mut out = Word::ZERO;
            out[..sl.len()].copy_from_slice(sl);
            out
        };

        // not actually valid, but we don't care here
        assert_eq!(
            DynSolType::FixedBytes(0).coerce_str("0x").unwrap(),
            DynSolValue::FixedBytes(mk_word(&[]), 0)
        );

        assert_eq!(
            DynSolType::FixedBytes(1).coerce_str("0x00").unwrap(),
            DynSolValue::FixedBytes(mk_word(&[0x00]), 1)
        );
        assert_eq!(
            DynSolType::FixedBytes(1).coerce_str("0x00").unwrap(),
            DynSolValue::FixedBytes(mk_word(&[0x00]), 1)
        );
        assert_eq!(
            DynSolType::FixedBytes(2).coerce_str("0017").unwrap(),
            DynSolValue::FixedBytes(mk_word(&[0x00, 0x17]), 2)
        );
        assert_eq!(
            DynSolType::FixedBytes(3).coerce_str("123456").unwrap(),
            DynSolValue::FixedBytes(mk_word(&[0x12, 0x34, 0x56]), 3)
        );

        let e = DynSolType::FixedBytes(1).coerce_str("").unwrap_err();
        assert_error_contains(&e, &Error::EmptyHexStringWithoutPrefix.to_string());
        let e = DynSolType::FixedBytes(1).coerce_str("0").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());
        let e = DynSolType::FixedBytes(1).coerce_str("0x").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::InvalidStringLength.to_string());
        let e = DynSolType::FixedBytes(1).coerce_str("0x0").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());

        let t = DynSolType::Array(Box::new(DynSolType::FixedBytes(1)));
        let e = t.coerce_str("[0]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());
        let e = t.coerce_str("[0x]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::InvalidStringLength.to_string());
        let e = t.coerce_str("[0x0]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());

        let t = DynSolType::Array(Box::new(DynSolType::Tuple(vec![DynSolType::FixedBytes(1)])));
        let e = t.coerce_str("[(0)]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());
        let e = t.coerce_str("[(0x)]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::InvalidStringLength.to_string());
        let e = t.coerce_str("[(0x0)]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());
    }

    #[test]
    fn coerce_address() {
        // 38
        assert!(DynSolType::Address.coerce_str("00000000000000000000000000000000000000").is_err());
        // 39
        assert!(DynSolType::Address.coerce_str("000000000000000000000000000000000000000").is_err());
        // 40
        assert_eq!(
            DynSolType::Address.coerce_str("0000000000000000000000000000000000000000").unwrap(),
            DynSolValue::Address(Address::ZERO)
        );
        assert_eq!(
            DynSolType::Address.coerce_str("0x1111111111111111111111111111111111111111").unwrap(),
            DynSolValue::Address(Address::new([0x11; 20]))
        );
        assert_eq!(
            DynSolType::Address.coerce_str("2222222222222222222222222222222222222222").unwrap(),
            DynSolValue::Address(Address::new([0x22; 20]))
        );
    }

    #[test]
    fn coerce_function() {
        assert_eq!(
            DynSolType::Function
                .coerce_str("000000000000000000000000000000000000000000000000")
                .unwrap(),
            DynSolValue::Function(Function::ZERO)
        );
        assert_eq!(
            DynSolType::Function
                .coerce_str("0x111111111111111111111111111111111111111111111111")
                .unwrap(),
            DynSolValue::Function(Function::new([0x11; 24]))
        );
        assert_eq!(
            DynSolType::Function
                .coerce_str("222222222222222222222222222222222222222222222222")
                .unwrap(),
            DynSolValue::Function(Function::new([0x22; 24]))
        );
    }

    #[test]
    fn coerce_bytes() {
        let e = DynSolType::Bytes.coerce_str("").unwrap_err();
        assert_error_contains(&e, &Error::EmptyHexStringWithoutPrefix.to_string());

        assert_eq!(DynSolType::Bytes.coerce_str("0x").unwrap(), DynSolValue::Bytes(vec![]));
        assert!(DynSolType::Bytes.coerce_str("0x0").is_err());
        assert!(DynSolType::Bytes.coerce_str("0").is_err());
        assert_eq!(DynSolType::Bytes.coerce_str("00").unwrap(), DynSolValue::Bytes(vec![0]));
        assert_eq!(DynSolType::Bytes.coerce_str("0x00").unwrap(), DynSolValue::Bytes(vec![0]));

        assert_eq!(
            DynSolType::Bytes.coerce_str("123456").unwrap(),
            DynSolValue::Bytes(vec![0x12, 0x34, 0x56])
        );
        assert_eq!(
            DynSolType::Bytes.coerce_str("0x0017").unwrap(),
            DynSolValue::Bytes(vec![0x00, 0x17])
        );

        let t = DynSolType::Tuple(vec![DynSolType::Bytes, DynSolType::Bytes]);
        let e = t.coerce_str("(0, 0x0)").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());

        // TODO: cut_err in `array_parser` somehow
        /*
        let t = DynSolType::Array(Box::new(DynSolType::Tuple(vec![
            DynSolType::Bytes,
            DynSolType::Bytes,
        ])));
        let e = t.coerce_str("[(0, 0x0)]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());

        let t = DynSolType::Array(Box::new(DynSolType::Tuple(vec![
            DynSolType::Bytes,
            DynSolType::Bytes,
        ])));
        let e = t.coerce_str("[(0x00, 0x0)]").unwrap_err();
        assert_error_contains(&e, &hex::FromHexError::OddLength.to_string());
        */
    }

    #[test]
    fn coerce_string() {
        assert_eq!(
            DynSolType::String.coerce_str("gavofyork").unwrap(),
            DynSolValue::String("gavofyork".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("gav of york").unwrap(),
            DynSolValue::String("gav of york".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("\"hello world\"").unwrap(),
            DynSolValue::String("hello world".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("'hello world'").unwrap(),
            DynSolValue::String("hello world".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("'\"hello world\"'").unwrap(),
            DynSolValue::String("\"hello world\"".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("'   hello world '").unwrap(),
            DynSolValue::String("   hello world ".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("'\"hello world'").unwrap(),
            DynSolValue::String("\"hello world".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("a, b").unwrap(),
            DynSolValue::String("a, b".into())
        );
        assert_eq!(
            DynSolType::String.coerce_str("hello (world)").unwrap(),
            DynSolValue::String("hello (world)".into())
        );

        assert!(DynSolType::String.coerce_str("\"hello world").is_err());
        assert!(DynSolType::String.coerce_str("\"hello world'").is_err());
        assert!(DynSolType::String.coerce_str("'hello world").is_err());
        assert!(DynSolType::String.coerce_str("'hello world\"").is_err());

        assert_eq!(
            DynSolType::String.coerce_str("Hello, world!").unwrap(),
            DynSolValue::String("Hello, world!".into())
        );
        let s = "$$g]a\"v/of;[()];2,yo\r)k_";
        assert_eq!(DynSolType::String.coerce_str(s).unwrap(), DynSolValue::String(s.into()));
    }

    #[test]
    fn coerce_strings() {
        let arr = DynSolType::Array(Box::new(DynSolType::String));
        let mk_arr = |s: &[&str]| {
            DynSolValue::Array(s.iter().map(|s| DynSolValue::String(s.to_string())).collect())
        };

        assert_eq!(arr.coerce_str("[]").unwrap(), mk_arr(&[]));
        assert_eq!(arr.coerce_str("[    ]").unwrap(), mk_arr(&[]));

        // TODO: should this be an error?
        // assert!(arr.coerce_str("[,]").is_err());
        // assert!(arr.coerce_str("[ , ]").is_err());

        assert_eq!(arr.coerce_str("[ foo bar ]").unwrap(), mk_arr(&["foo bar"]));
        assert_eq!(arr.coerce_str("[foo bar,]").unwrap(), mk_arr(&["foo bar"]));
        assert_eq!(arr.coerce_str("[  foo bar,  ]").unwrap(), mk_arr(&["foo bar"]));
        assert_eq!(arr.coerce_str("[ foo , bar ]").unwrap(), mk_arr(&["foo", "bar"]));

        assert_eq!(arr.coerce_str("[\"foo\",\"bar\"]").unwrap(), mk_arr(&["foo", "bar"]));

        assert_eq!(arr.coerce_str("['']").unwrap(), mk_arr(&[""]));
        assert_eq!(arr.coerce_str("[\"\"]").unwrap(), mk_arr(&[""]));
        assert_eq!(arr.coerce_str("['', '']").unwrap(), mk_arr(&["", ""]));
        assert_eq!(arr.coerce_str("['', \"\"]").unwrap(), mk_arr(&["", ""]));
        assert_eq!(arr.coerce_str("[\"\", '']").unwrap(), mk_arr(&["", ""]));
        assert_eq!(arr.coerce_str("[\"\", \"\"]").unwrap(), mk_arr(&["", ""]));
    }

    #[test]
    fn coerce_array_of_bytes_and_strings() {
        let ty = DynSolType::Array(Box::new(DynSolType::Bytes));
        assert_eq!(ty.coerce_str("[]"), Ok(DynSolValue::Array(vec![])));
        assert_eq!(ty.coerce_str("[0x]"), Ok(DynSolValue::Array(vec![DynSolValue::Bytes(vec![])])));

        let ty = DynSolType::Array(Box::new(DynSolType::String));
        assert_eq!(ty.coerce_str("[]"), Ok(DynSolValue::Array(vec![])));
        assert_eq!(
            ty.coerce_str("[\"\"]"),
            Ok(DynSolValue::Array(vec![DynSolValue::String(String::new())]))
        );
        assert_eq!(
            ty.coerce_str("[0x]"),
            Ok(DynSolValue::Array(vec![DynSolValue::String("0x".into())]))
        );
    }

    #[test]
    fn coerce_empty_array() {
        assert_eq!(
            DynSolType::Array(Box::new(DynSolType::Bool)).coerce_str("[]").unwrap(),
            DynSolValue::Array(vec![])
        );
        assert_eq!(
            DynSolType::FixedArray(Box::new(DynSolType::Bool), 0).coerce_str("[]").unwrap(),
            DynSolValue::FixedArray(vec![]),
        );
        assert!(DynSolType::FixedArray(Box::new(DynSolType::Bool), 1).coerce_str("[]").is_err());
    }

    #[test]
    fn coerce_bool_array() {
        assert_eq!(
            DynSolType::coerce_str(&DynSolType::Array(Box::new(DynSolType::Bool)), "[true, false]")
                .unwrap(),
            DynSolValue::Array(vec![DynSolValue::Bool(true), DynSolValue::Bool(false)])
        );
    }

    #[test]
    fn coerce_bool_array_of_arrays() {
        assert_eq!(
            DynSolType::coerce_str(
                &DynSolType::Array(Box::new(DynSolType::Array(Box::new(DynSolType::Bool)))),
                "[ [ true, true, false ], [ false]]"
            )
            .unwrap(),
            DynSolValue::Array(vec![
                DynSolValue::Array(vec![
                    DynSolValue::Bool(true),
                    DynSolValue::Bool(true),
                    DynSolValue::Bool(false)
                ]),
                DynSolValue::Array(vec![DynSolValue::Bool(false)])
            ])
        );
    }

    #[test]
    fn coerce_bool_fixed_array() {
        let ty = DynSolType::FixedArray(Box::new(DynSolType::Bool), 3);
        assert!(ty.coerce_str("[]").is_err());
        assert!(ty.coerce_str("[true]").is_err());
        assert!(ty.coerce_str("[true, false]").is_err());
        assert_eq!(
            ty.coerce_str("[true, false, true]").unwrap(),
            DynSolValue::FixedArray(vec![
                DynSolValue::Bool(true),
                DynSolValue::Bool(false),
                DynSolValue::Bool(true),
            ])
        );
        assert!(ty.coerce_str("[true, false, false, true]").is_err());
    }

    #[test]
    fn single_quoted_in_array_must_error() {
        assert!(DynSolType::Array(Box::new(DynSolType::Bool))
            .coerce_str("[true,\"false,false]")
            .is_err());
        assert!(DynSolType::Array(Box::new(DynSolType::Bool)).coerce_str("[false\"]").is_err());
        assert!(DynSolType::Array(Box::new(DynSolType::Bool))
            .coerce_str("[true,false\"]")
            .is_err());
        assert!(DynSolType::Array(Box::new(DynSolType::Bool))
            .coerce_str("[true,\"false\",false]")
            .is_err());
        assert!(DynSolType::Array(Box::new(DynSolType::Bool)).coerce_str("[true,false]").is_ok());
    }

    #[test]
    fn tuples() {
        let ty = DynSolType::Tuple(vec![DynSolType::String, DynSolType::Bool, DynSolType::String]);
        assert_eq!(
            ty.coerce_str("(\"a,]) b\", true, true? ]and] false!)").unwrap(),
            DynSolValue::Tuple(vec![
                DynSolValue::String("a,]) b".into()),
                DynSolValue::Bool(true),
                DynSolValue::String("true? ]and] false!".into()),
            ])
        );
        assert!(ty.coerce_str("(\"\", true, a, b)").is_err());
        assert!(ty.coerce_str("(a, b, true, a)").is_err());
    }

    #[test]
    fn tuples_arrays_mixed() {
        assert_eq!(
            DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                DynSolType::Array(Box::new(DynSolType::Tuple(vec![DynSolType::Bool]))),
                DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                    DynSolType::Bool,
                    DynSolType::Bool
                ]))),
            ])))
            .coerce_str("[([(true)],[(false,true)])]")
            .unwrap(),
            DynSolValue::Array(vec![DynSolValue::Tuple(vec![
                DynSolValue::Array(vec![DynSolValue::Tuple(vec![DynSolValue::Bool(true)])]),
                DynSolValue::Array(vec![DynSolValue::Tuple(vec![
                    DynSolValue::Bool(false),
                    DynSolValue::Bool(true)
                ])]),
            ])])
        );

        assert_eq!(
            DynSolType::Tuple(vec![
                DynSolType::Array(Box::new(DynSolType::Tuple(vec![DynSolType::Bool]))),
                DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                    DynSolType::Bool,
                    DynSolType::Bool
                ]))),
            ])
            .coerce_str("([(true)],[(false,true)])")
            .unwrap(),
            DynSolValue::Tuple(vec![
                DynSolValue::Array(vec![DynSolValue::Tuple(vec![DynSolValue::Bool(true)])]),
                DynSolValue::Array(vec![DynSolValue::Tuple(vec![
                    DynSolValue::Bool(false),
                    DynSolValue::Bool(true)
                ])]),
            ])
        );
    }

    #[test]
    fn tuple_array_nested() {
        assert_eq!(
            DynSolType::Tuple(vec![
                DynSolType::Array(Box::new(DynSolType::Tuple(vec![DynSolType::Address]))),
                DynSolType::Uint(256),
            ])
            .coerce_str("([(5c9d55b78febcc2061715ba4f57ecf8ea2711f2c)],2)")
            .unwrap(),
            DynSolValue::Tuple(vec![
                DynSolValue::Array(vec![DynSolValue::Tuple(vec![DynSolValue::Address(address!(
                    "5c9d55b78febcc2061715ba4f57ecf8ea2711f2c"
                ))])]),
                DynSolValue::Uint(U256::from(2), 256),
            ])
        );
    }

    // keep `n` low to avoid stack overflows (debug mode)
    #[test]
    fn lotsa_array_nesting() {
        let n = 10;

        let mut ty = DynSolType::Bool;
        for _ in 0..n {
            ty = DynSolType::Array(Box::new(ty));
        }
        let mut value_str = String::new();
        value_str.push_str(&"[".repeat(n));
        value_str.push_str("true");
        value_str.push_str(&"]".repeat(n));

        let mut value = ty.coerce_str(&value_str).unwrap();
        for _ in 0..n {
            let DynSolValue::Array(arr) = value else { panic!("{value:?}") };
            assert_eq!(arr.len(), 1);
            value = arr.into_iter().next().unwrap();
        }
        assert_eq!(value, DynSolValue::Bool(true));
    }

    #[test]
    fn lotsa_tuple_nesting() {
        let n = 10;

        let mut ty = DynSolType::Bool;
        for _ in 0..n {
            ty = DynSolType::Tuple(vec![ty]);
        }
        let mut value_str = String::new();
        value_str.push_str(&"(".repeat(n));
        value_str.push_str("true");
        value_str.push_str(&")".repeat(n));

        let mut value = ty.coerce_str(&value_str).unwrap();
        for _ in 0..n {
            let DynSolValue::Tuple(tuple) = value else { panic!("{value:?}") };
            assert_eq!(tuple.len(), 1);
            value = tuple.into_iter().next().unwrap();
        }
        assert_eq!(value, DynSolValue::Bool(true));
    }

    #[test]
    fn coerce_uint_scientific() {
        uint_test("1e18", Ok("1000000000000000000"));

        uint_test("0.03069536448928848133e20", Ok("3069536448928848133"));

        uint_test("1.5e18", Ok("1500000000000000000"));

        uint_test("1e-3 ether", Ok("1000000000000000"));
        uint_test("1.0e-3 ether", Ok("1000000000000000"));
        uint_test("1.1e-3 ether", Ok("1100000000000000"));

        uint_test("74258.225772486694040708e18", Ok("74258225772486694040708"));
        uint_test("0.03069536448928848133e20", Ok("3069536448928848133"));
        uint_test("0.000000000003069536448928848133e30", Ok("3069536448928848133"));

        uint_test("1e-1", Err(()));
        uint_test("1e-2", Err(()));
        uint_test("1e-18", Err(()));
        uint_test("1 e18", Err(()));
        uint_test("1ex", Err(()));
        uint_test("1e", Err(()));
    }
}
