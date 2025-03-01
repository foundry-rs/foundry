// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::parser::{ParserError, SubtagIterator};
use crate::shortvec::ShortBoxSlice;
use core::ops::RangeInclusive;
use core::str::FromStr;
use tinystr::TinyAsciiStr;

/// A value used in a list of [`Fields`](super::Fields).
///
/// The value has to be a sequence of one or more alphanumerical strings
/// separated by `-`.
/// Each part of the sequence has to be no shorter than three characters and no
/// longer than 8.
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::transform::Value;
///
/// "hybrid".parse::<Value>().expect("Valid Value.");
///
/// "hybrid-foobar".parse::<Value>().expect("Valid Value.");
///
/// "no".parse::<Value>().expect_err("Invalid Value.");
/// ```
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Default)]
pub struct Value(ShortBoxSlice<TinyAsciiStr<{ *TYPE_LENGTH.end() }>>);

const TYPE_LENGTH: RangeInclusive<usize> = 3..=8;
const TRUE_TVALUE: TinyAsciiStr<8> = tinystr::tinystr!(8, "true");

impl Value {
    /// A constructor which takes a utf8 slice, parses it and
    /// produces a well-formed [`Value`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::transform::Value;
    ///
    /// let value = Value::try_from_bytes(b"hybrid").expect("Parsing failed.");
    /// ```
    pub fn try_from_bytes(input: &[u8]) -> Result<Self, ParserError> {
        let mut v = ShortBoxSlice::default();
        let mut has_value = false;

        for subtag in SubtagIterator::new(input) {
            if !Self::is_type_subtag(subtag) {
                return Err(ParserError::InvalidExtension);
            }
            has_value = true;
            let val =
                TinyAsciiStr::from_bytes(subtag).map_err(|_| ParserError::InvalidExtension)?;
            if val != TRUE_TVALUE {
                v.push(val);
            }
        }

        if !has_value {
            return Err(ParserError::InvalidExtension);
        }
        Ok(Self(v))
    }

    pub(crate) fn from_short_slice_unchecked(
        input: ShortBoxSlice<TinyAsciiStr<{ *TYPE_LENGTH.end() }>>,
    ) -> Self {
        Self(input)
    }

    pub(crate) fn is_type_subtag(t: &[u8]) -> bool {
        TYPE_LENGTH.contains(&t.len()) && t.iter().all(u8::is_ascii_alphanumeric)
    }

    pub(crate) fn parse_subtag(
        t: &[u8],
    ) -> Result<Option<TinyAsciiStr<{ *TYPE_LENGTH.end() }>>, ParserError> {
        let s = TinyAsciiStr::from_bytes(t).map_err(|_| ParserError::InvalidSubtag)?;
        if !TYPE_LENGTH.contains(&t.len()) || !s.is_ascii_alphanumeric() {
            return Err(ParserError::InvalidExtension);
        }

        let s = s.to_ascii_lowercase();

        if s == TRUE_TVALUE {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        if self.0.is_empty() {
            f("true")?;
        } else {
            self.0.iter().map(TinyAsciiStr::as_str).try_for_each(f)?;
        }
        Ok(())
    }
}

impl FromStr for Value {
    type Err = ParserError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::try_from_bytes(source.as_bytes())
    }
}

impl_writeable_for_each_subtag_str_no_test!(Value, selff, selff.0.is_empty() => alloc::borrow::Cow::Borrowed("true"));

#[test]
fn test_writeable() {
    use writeable::assert_writeable_eq;

    let hybrid = "hybrid".parse().unwrap();
    let foobar = "foobar".parse().unwrap();

    assert_writeable_eq!(Value::default(), "true");
    assert_writeable_eq!(
        Value::from_short_slice_unchecked(vec![hybrid].into()),
        "hybrid"
    );
    assert_writeable_eq!(
        Value::from_short_slice_unchecked(vec![hybrid, foobar].into()),
        "hybrid-foobar"
    );
}
