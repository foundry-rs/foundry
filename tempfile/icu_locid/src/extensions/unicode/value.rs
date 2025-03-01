// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::parser::{ParserError, SubtagIterator};
use crate::shortvec::ShortBoxSlice;
use core::ops::RangeInclusive;
use core::str::FromStr;
use tinystr::TinyAsciiStr;

/// A value used in a list of [`Keywords`](super::Keywords).
///
/// The value has to be a sequence of one or more alphanumerical strings
/// separated by `-`.
/// Each part of the sequence has to be no shorter than three characters and no
/// longer than 8.
///
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::unicode::{value, Value};
/// use writeable::assert_writeable_eq;
///
/// assert_writeable_eq!(value!("gregory"), "gregory");
/// assert_writeable_eq!(
///     "islamic-civil".parse::<Value>().unwrap(),
///     "islamic-civil"
/// );
///
/// // The value "true" has the special, empty string representation
/// assert_eq!(value!("true").to_string(), "");
/// ```
#[derive(Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord, Default)]
pub struct Value(ShortBoxSlice<TinyAsciiStr<{ *VALUE_LENGTH.end() }>>);

const VALUE_LENGTH: RangeInclusive<usize> = 3..=8;
const TRUE_VALUE: TinyAsciiStr<8> = tinystr::tinystr!(8, "true");

impl Value {
    /// A constructor which takes a utf8 slice, parses it and
    /// produces a well-formed [`Value`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::Value;
    ///
    /// Value::try_from_bytes(b"buddhist").expect("Parsing failed.");
    /// ```
    pub fn try_from_bytes(input: &[u8]) -> Result<Self, ParserError> {
        let mut v = ShortBoxSlice::new();

        if !input.is_empty() {
            for subtag in SubtagIterator::new(input) {
                let val = Self::subtag_from_bytes(subtag)?;
                if let Some(val) = val {
                    v.push(val);
                }
            }
        }
        Ok(Self(v))
    }

    /// Const constructor for when the value contains only a single subtag.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::Value;
    ///
    /// Value::try_from_single_subtag(b"buddhist").expect("valid subtag");
    /// Value::try_from_single_subtag(b"#####").expect_err("invalid subtag");
    /// Value::try_from_single_subtag(b"foo-bar").expect_err("not a single subtag");
    /// ```
    pub const fn try_from_single_subtag(subtag: &[u8]) -> Result<Self, ParserError> {
        match Self::subtag_from_bytes(subtag) {
            Err(_) => Err(ParserError::InvalidExtension),
            Ok(option) => Ok(Self::from_tinystr(option)),
        }
    }

    #[doc(hidden)]
    pub fn as_tinystr_slice(&self) -> &[TinyAsciiStr<8>] {
        &self.0
    }

    #[doc(hidden)]
    pub const fn as_single_subtag(&self) -> Option<&TinyAsciiStr<8>> {
        self.0.single()
    }

    #[doc(hidden)]
    pub const fn from_tinystr(subtag: Option<TinyAsciiStr<8>>) -> Self {
        match subtag {
            None => Self(ShortBoxSlice::new()),
            Some(val) => {
                debug_assert!(val.is_ascii_alphanumeric());
                debug_assert!(!matches!(val, TRUE_VALUE));
                Self(ShortBoxSlice::new_single(val))
            }
        }
    }

    pub(crate) fn from_short_slice_unchecked(input: ShortBoxSlice<TinyAsciiStr<8>>) -> Self {
        Self(input)
    }

    #[doc(hidden)]
    pub const fn subtag_from_bytes(bytes: &[u8]) -> Result<Option<TinyAsciiStr<8>>, ParserError> {
        Self::parse_subtag_from_bytes_manual_slice(bytes, 0, bytes.len())
    }

    pub(crate) fn parse_subtag(t: &[u8]) -> Result<Option<TinyAsciiStr<8>>, ParserError> {
        Self::parse_subtag_from_bytes_manual_slice(t, 0, t.len())
    }

    pub(crate) const fn parse_subtag_from_bytes_manual_slice(
        bytes: &[u8],
        start: usize,
        end: usize,
    ) -> Result<Option<TinyAsciiStr<8>>, ParserError> {
        let slice_len = end - start;
        if slice_len > *VALUE_LENGTH.end() || slice_len < *VALUE_LENGTH.start() {
            return Err(ParserError::InvalidExtension);
        }

        match TinyAsciiStr::from_bytes_manual_slice(bytes, start, end) {
            Ok(TRUE_VALUE) => Ok(None),
            Ok(s) if s.is_ascii_alphanumeric() => Ok(Some(s.to_ascii_lowercase())),
            Ok(_) => Err(ParserError::InvalidExtension),
            Err(_) => Err(ParserError::InvalidSubtag),
        }
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        self.0.iter().map(TinyAsciiStr::as_str).try_for_each(f)
    }
}

impl FromStr for Value {
    type Err = ParserError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::try_from_bytes(source.as_bytes())
    }
}

impl_writeable_for_subtag_list!(Value, "islamic", "civil");

/// A macro allowing for compile-time construction of valid Unicode [`Value`] subtag.
///
/// The macro only supports single-subtag values.
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::unicode::{key, value};
/// use icu::locid::Locale;
///
/// let loc: Locale = "de-u-ca-buddhist".parse().unwrap();
///
/// assert_eq!(
///     loc.extensions.unicode.keywords.get(&key!("ca")),
///     Some(&value!("buddhist"))
/// );
/// ```
///
/// [`Value`]: crate::extensions::unicode::Value
#[macro_export]
#[doc(hidden)]
macro_rules! extensions_unicode_value {
    ($value:literal) => {{
        // What we want:
        // const R: $crate::extensions::unicode::Value =
        //     match $crate::extensions::unicode::Value::try_from_single_subtag($value.as_bytes()) {
        //         Ok(r) => r,
        //         #[allow(clippy::panic)] // const context
        //         _ => panic!(concat!("Invalid Unicode extension value: ", $value)),
        //     };
        // Workaround until https://github.com/rust-lang/rust/issues/73255 lands:
        const R: $crate::extensions::unicode::Value =
            $crate::extensions::unicode::Value::from_tinystr(
                match $crate::extensions::unicode::Value::subtag_from_bytes($value.as_bytes()) {
                    Ok(r) => r,
                    _ => panic!(concat!("Invalid Unicode extension value: ", $value)),
                },
            );
        R
    }};
}
#[doc(inline)]
pub use extensions_unicode_value as value;
