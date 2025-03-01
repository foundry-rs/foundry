//! Support for the [`postgres_types`] crate.
//!
//! **WARNING**: this module depends entirely on [`postgres_types`, which is not yet stable,
//! therefore this module is exempt from the semver guarantees of this crate.

use super::{FixedBytes, Sign, Signed};
use bytes::{BufMut, BytesMut};
use derive_more::Display;
use postgres_types::{accepts, to_sql_checked, FromSql, IsNull, ToSql, Type, WrongType};
use std::{
    error::Error,
    iter,
    str::{from_utf8, FromStr},
};

/// Converts `FixedBytes` to Postgres Bytea Type.
impl<const BITS: usize> ToSql for FixedBytes<BITS> {
    fn to_sql(&self, _: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError> {
        out.put_slice(&self[..]);
        Ok(IsNull::No)
    }

    accepts!(BYTEA);

    to_sql_checked!();
}

/// Converts `FixedBytes` From Postgres Bytea Type.
impl<'a, const BITS: usize> FromSql<'a> for FixedBytes<BITS> {
    accepts!(BYTEA);

    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(Self::try_from(raw)?)
    }
}

// https://github.com/recmo/uint/blob/6c755ad7cd54a0706d20f11f3f63b0d977af0226/src/support/postgres.rs#L22

type BoxedError = Box<dyn Error + Sync + Send + 'static>;

const fn rem_up(a: usize, b: usize) -> usize {
    let rem = a % b;
    if rem > 0 {
        rem
    } else {
        b
    }
}

fn last_idx<T: PartialEq>(x: &[T], value: &T) -> usize {
    x.iter().rposition(|b| b != value).map_or(0, |idx| idx + 1)
}

fn trim_end_vec<T: PartialEq>(vec: &mut Vec<T>, value: &T) {
    vec.truncate(last_idx(vec, value));
}

/// Error when converting to Postgres types.
#[derive(Clone, Debug, PartialEq, Eq, Display)]
pub enum ToSqlError {
    /// The value is too large for the type.
    #[display("Signed<{_0}> value too large to fit target type {_1}")]
    Overflow(usize, Type),
}

impl core::error::Error for ToSqlError {}

/// Convert to Postgres types.
///
/// Compatible [Postgres data types][dt] are:
///
/// * `BOOL`, `SMALLINT`, `INTEGER`, `BIGINT` which are 1, 16, 32 and 64 bit signed integers
///   respectively.
/// * `OID` which is a 32 bit unsigned integer.
/// * `DECIMAL` and `NUMERIC`, which are variable length.
/// * `MONEY` which is a 64 bit integer with two decimals.
/// * `BYTEA`, `BIT`, `VARBIT` interpreted as a big-endian binary number.
/// * `CHAR`, `VARCHAR`, `TEXT` as `0x`-prefixed big-endian hex strings.
/// * `JSON`, `JSONB` as a hex string compatible with the Serde serialization.
///
/// # Errors
///
/// Returns an error when trying to convert to a value that is too small to fit
/// the number. Note that this depends on the value, not the type, so a
/// [`Signed<256>`] can be stored in a `SMALLINT` column, as long as the values
/// are less than $2^{16}$.
///
/// # Implementation details
///
/// The Postgres binary formats are used in the wire-protocol and the
/// the `COPY BINARY` command, but they have very little documentation. You are
/// pointed to the source code, for example this is the implementation of the
/// the `NUMERIC` type serializer: [`numeric.c`][numeric].
///
/// [dt]:https://www.postgresql.org/docs/9.5/datatype.html
/// [numeric]: https://github.com/postgres/postgres/blob/05a5a1775c89f6beb326725282e7eea1373cbec8/src/backend/utils/adt/numeric.c#L1082
impl<const BITS: usize, const LIMBS: usize> ToSql for Signed<BITS, LIMBS> {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxedError> {
        match *ty {
            // Big-endian simple types
            // Note `BufMut::put_*` methods write big-endian by default.
            Type::BOOL => out.put_u8(u8::from(bool::try_from(self.0)?)),
            Type::INT2 => out.put_i16(self.0.try_into()?),
            Type::INT4 => out.put_i32(self.0.try_into()?),
            Type::OID => out.put_u32(self.0.try_into()?),
            Type::INT8 => out.put_i64(self.0.try_into()?),

            Type::MONEY => {
                // Like i64, but with two decimals.
                out.put_i64(
                    i64::try_from(self.0)?
                        .checked_mul(100)
                        .ok_or(ToSqlError::Overflow(BITS, ty.clone()))?,
                );
            }

            // Binary strings
            Type::BYTEA => out.put_slice(&self.0.to_be_bytes_vec()),
            Type::BIT | Type::VARBIT => {
                // Bit in little-endian so the first bit is the least significant.
                // Length must be at least one bit.
                if BITS == 0 {
                    if *ty == Type::BIT {
                        // `bit(0)` is not a valid type, but varbit can be empty.
                        return Err(Box::new(WrongType::new::<Self>(ty.clone())));
                    }
                    out.put_i32(0);
                } else {
                    // Bits are output in big-endian order, but padded at the
                    // least significant end.
                    let padding = 8 - rem_up(BITS, 8);
                    out.put_i32(Self::BITS.try_into()?);
                    let bytes = self.0.as_le_bytes();
                    let mut bytes = bytes.iter().rev();
                    let mut shifted = bytes.next().unwrap() << padding;
                    for byte in bytes {
                        shifted |= if padding > 0 { byte >> (8 - padding) } else { 0 };
                        out.put_u8(shifted);
                        shifted = byte << padding;
                    }
                    out.put_u8(shifted);
                }
            }

            // Hex strings
            Type::CHAR | Type::TEXT | Type::VARCHAR => {
                out.put_slice(format!("{self:#x}").as_bytes());
            }
            Type::JSON | Type::JSONB => {
                if *ty == Type::JSONB {
                    // Version 1 of JSONB is just plain text JSON.
                    out.put_u8(1);
                }
                out.put_slice(format!("\"{self:#x}\"").as_bytes());
            }

            // Binary coded decimal types
            // See <https://github.com/postgres/postgres/blob/05a5a1775c89f6beb326725282e7eea1373cbec8/src/backend/utils/adt/numeric.c#L253>
            Type::NUMERIC => {
                // Everything is done in big-endian base 1000 digits.
                const BASE: u64 = 10000;

                let sign = match self.sign() {
                    Sign::Positive => 0x0000,
                    _ => 0x4000,
                };

                let mut digits: Vec<_> = self.abs().0.to_base_be(BASE).collect();
                let exponent = digits.len().saturating_sub(1).try_into()?;

                // Trailing zeros are removed.
                trim_end_vec(&mut digits, &0);

                out.put_i16(digits.len().try_into()?); // Number of digits.
                out.put_i16(exponent); // Exponent of first digit.

                out.put_i16(sign);
                out.put_i16(0); // dscale: Number of digits to the right of the decimal point.
                for digit in digits {
                    debug_assert!(digit < BASE);
                    #[allow(clippy::cast_possible_truncation)] // 10000 < i16::MAX
                    out.put_i16(digit as i16);
                }
            }

            // Unsupported types
            _ => {
                return Err(Box::new(WrongType::new::<Self>(ty.clone())));
            }
        };
        Ok(IsNull::No)
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, |Type::BOOL| Type::CHAR
            | Type::INT2
            | Type::INT4
            | Type::INT8
            | Type::OID
            | Type::FLOAT4
            | Type::FLOAT8
            | Type::MONEY
            | Type::NUMERIC
            | Type::BYTEA
            | Type::TEXT
            | Type::VARCHAR
            | Type::JSON
            | Type::JSONB
            | Type::BIT
            | Type::VARBIT)
    }

    to_sql_checked!();
}

/// Error when converting from Postgres types.
#[derive(Clone, Debug, PartialEq, Eq, Display)]
pub enum FromSqlError {
    /// The value is too large for the type.
    #[display("the value is too large for the Signed type")]
    Overflow,

    /// The value is not valid for the type.
    #[display("unexpected data for type {_0}")]
    ParseError(Type),
}

impl core::error::Error for FromSqlError {}

impl<'a, const BITS: usize, const LIMBS: usize> FromSql<'a> for Signed<BITS, LIMBS> {
    fn accepts(ty: &Type) -> bool {
        <Self as ToSql>::accepts(ty)
    }

    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(match *ty {
            Type::BOOL => match raw {
                [0] => Self::ZERO,
                [1] => Self::try_from(1)?,
                _ => return Err(Box::new(FromSqlError::ParseError(ty.clone()))),
            },
            Type::INT2 => i16::from_be_bytes(raw.try_into()?).try_into()?,
            Type::INT4 => i32::from_be_bytes(raw.try_into()?).try_into()?,
            Type::OID => u32::from_be_bytes(raw.try_into()?).try_into()?,
            Type::INT8 => i64::from_be_bytes(raw.try_into()?).try_into()?,
            Type::MONEY => (i64::from_be_bytes(raw.try_into()?) / 100).try_into()?,

            // Binary strings
            Type::BYTEA => Self::try_from_be_slice(raw).ok_or(FromSqlError::Overflow)?,
            Type::BIT | Type::VARBIT => {
                // Parse header
                if raw.len() < 4 {
                    return Err(Box::new(FromSqlError::ParseError(ty.clone())));
                }
                let len: usize = i32::from_be_bytes(raw[..4].try_into()?).try_into()?;
                let raw = &raw[4..];

                // Shift padding to the other end
                let padding = 8 - rem_up(len, 8);
                let mut raw = raw.to_owned();
                if padding > 0 {
                    for i in (1..raw.len()).rev() {
                        raw[i] = (raw[i] >> padding) | (raw[i - 1] << (8 - padding));
                    }
                    raw[0] >>= padding;
                }
                // Construct from bits
                Self::try_from_be_slice(&raw).ok_or(FromSqlError::Overflow)?
            }

            // Hex strings
            Type::CHAR | Type::TEXT | Type::VARCHAR => Self::from_str(from_utf8(raw)?)?,

            // Hex strings
            Type::JSON | Type::JSONB => {
                let raw = if *ty == Type::JSONB {
                    if raw[0] == 1 {
                        &raw[1..]
                    } else {
                        // Unsupported version
                        return Err(Box::new(FromSqlError::ParseError(ty.clone())));
                    }
                } else {
                    raw
                };
                let str = from_utf8(raw)?;
                let str = if str.starts_with('"') && str.ends_with('"') {
                    // Stringified number
                    &str[1..str.len() - 1]
                } else {
                    str
                };
                Self::from_str(str)?
            }

            // Numeric types
            Type::NUMERIC => {
                // Parse header
                if raw.len() < 8 {
                    return Err(Box::new(FromSqlError::ParseError(ty.clone())));
                }
                let digits = i16::from_be_bytes(raw[0..2].try_into()?);
                let exponent = i16::from_be_bytes(raw[2..4].try_into()?);
                let sign = i16::from_be_bytes(raw[4..6].try_into()?);
                let dscale = i16::from_be_bytes(raw[6..8].try_into()?);
                let raw = &raw[8..];
                #[allow(clippy::cast_sign_loss)] // Signs are checked
                if digits < 0
                    || exponent < 0
                    || dscale != 0
                    || digits > exponent + 1
                    || raw.len() != digits as usize * 2
                {
                    return Err(Box::new(FromSqlError::ParseError(ty.clone())));
                }
                let mut error = false;
                let iter = raw.chunks_exact(2).filter_map(|raw| {
                    if error {
                        return None;
                    }
                    let digit = i16::from_be_bytes(raw.try_into().unwrap());
                    if !(0..10000).contains(&digit) {
                        error = true;
                        return None;
                    }
                    #[allow(clippy::cast_sign_loss)] // Signs are checked
                    Some(digit as u64)
                });
                #[allow(clippy::cast_sign_loss)]
                // Expression can not be negative due to checks above
                let iter = iter.chain(iter::repeat(0).take((exponent + 1 - digits) as usize));

                let mut value = Self::from_base_be(10000, iter)?;
                if sign == 0x4000 {
                    value = -value;
                }
                if error {
                    return Err(Box::new(FromSqlError::ParseError(ty.clone())));
                }

                value
            }

            // Unsupported types
            _ => return Err(Box::new(WrongType::new::<Self>(ty.clone()))),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::I256;

    #[test]
    fn positive_i256_from_sql() {
        assert_eq!(
            I256::from_sql(
                &Type::NUMERIC,
                &[
                    0x00, 0x01, // ndigits: 1
                    0x00, 0x00, // weight: 0
                    0x00, 0x00, // sign: 0x0000 (positive)
                    0x00, 0x00, // scale: 0
                    0x00, 0x01, // digit: 1
                ]
            )
            .unwrap(),
            I256::ONE
        );
    }

    #[test]
    fn positive_i256_to_sql() {
        let mut bytes = BytesMut::with_capacity(64);
        I256::ONE.to_sql(&Type::NUMERIC, &mut bytes).unwrap();
        assert_eq!(
            *bytes.freeze(),
            [
                0x00, 0x01, // ndigits: 1
                0x00, 0x00, // weight: 0
                0x00, 0x00, // sign: 0x0000 (positive)
                0x00, 0x00, // scale: 0
                0x00, 0x01, // digit: 1
            ],
        );
    }

    #[test]
    fn negative_i256_from_sql() {
        assert_eq!(
            I256::from_sql(
                &Type::NUMERIC,
                &[
                    0x00, 0x01, // ndigits: 1
                    0x00, 0x00, // weight: 0
                    0x40, 0x00, // sign: 0x4000 (negative)
                    0x00, 0x00, // scale: 0
                    0x00, 0x01, // digit: 1
                ]
            )
            .unwrap(),
            I256::MINUS_ONE
        );
    }

    #[test]
    fn negative_i256_to_sql() {
        let mut bytes = BytesMut::with_capacity(64);
        I256::MINUS_ONE.to_sql(&Type::NUMERIC, &mut bytes).unwrap();
        assert_eq!(
            *bytes.freeze(),
            [
                0x00, 0x01, // ndigits: 1
                0x00, 0x00, // weight: 0
                0x40, 0x00, // sign: 0x4000 (negative)
                0x00, 0x00, // scale: 0
                0x00, 0x01, // digit: 1
            ],
        );
    }
}
