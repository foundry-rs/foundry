// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Other Use Extensions is a list of extensions other than unicode,
//! transform or private.
//!
//! Those extensions are treated as a pass-through, and no Unicode related
//! behavior depends on them.
//!
//! The main struct for this extension is [`Other`] which is a list of [`Subtag`]s.
//!
//! # Examples
//!
//! ```
//! use icu::locid::extensions::other::Other;
//! use icu::locid::Locale;
//!
//! let mut loc: Locale = "en-US-a-foo-faa".parse().expect("Parsing failed.");
//! ```

mod subtag;

use crate::parser::ParserError;
use crate::parser::SubtagIterator;
use crate::shortvec::ShortBoxSlice;
use alloc::vec::Vec;
#[doc(inline)]
pub use subtag::{subtag, Subtag};

/// A list of [`Other Use Extensions`] as defined in [`Unicode Locale
/// Identifier`] specification.
///
/// Those extensions are treated as a pass-through, and no Unicode related
/// behavior depends on them.
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::other::{Other, Subtag};
///
/// let subtag1: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
/// let subtag2: Subtag = "bar".parse().expect("Failed to parse a Subtag.");
///
/// let other = Other::from_vec_unchecked(b'a', vec![subtag1, subtag2]);
/// assert_eq!(&other.to_string(), "a-foo-bar");
/// ```
///
/// [`Other Use Extensions`]: https://unicode.org/reports/tr35/#other_extensions
/// [`Unicode Locale Identifier`]: https://unicode.org/reports/tr35/#Unicode_locale_identifier
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash, PartialOrd, Ord)]
pub struct Other {
    ext: u8,
    keys: ShortBoxSlice<Subtag>,
}

impl Other {
    /// A constructor which takes a pre-sorted list of [`Subtag`].
    ///
    /// # Panics
    ///
    /// Panics if `ext` is not ASCII alphabetic.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::other::{Other, Subtag};
    ///
    /// let subtag1: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
    /// let subtag2: Subtag = "bar".parse().expect("Failed to parse a Subtag.");
    ///
    /// let other = Other::from_vec_unchecked(b'a', vec![subtag1, subtag2]);
    /// assert_eq!(&other.to_string(), "a-foo-bar");
    /// ```
    pub fn from_vec_unchecked(ext: u8, keys: Vec<Subtag>) -> Self {
        Self::from_short_slice_unchecked(ext, keys.into())
    }

    pub(crate) fn from_short_slice_unchecked(ext: u8, keys: ShortBoxSlice<Subtag>) -> Self {
        assert!(ext.is_ascii_alphabetic());
        Self { ext, keys }
    }

    pub(crate) fn try_from_iter(ext: u8, iter: &mut SubtagIterator) -> Result<Self, ParserError> {
        debug_assert!(ext.is_ascii_alphabetic());

        let mut keys = ShortBoxSlice::new();
        while let Some(subtag) = iter.peek() {
            if !Subtag::valid_key(subtag) {
                break;
            }
            if let Ok(key) = Subtag::try_from_bytes(subtag) {
                keys.push(key);
            }
            iter.next();
        }

        Ok(Self::from_short_slice_unchecked(ext, keys))
    }

    /// Gets the tag character for this extension as a &str.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::Locale;
    ///
    /// let loc: Locale = "und-a-hello-world".parse().unwrap();
    /// let other_ext = &loc.extensions.other[0];
    /// assert_eq!(other_ext.get_ext_str(), "a");
    /// ```
    pub fn get_ext_str(&self) -> &str {
        debug_assert!(self.ext.is_ascii_alphabetic());
        unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&self.ext)) }
    }

    /// Gets the tag character for this extension as a char.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::Locale;
    ///
    /// let loc: Locale = "und-a-hello-world".parse().unwrap();
    /// let other_ext = &loc.extensions.other[0];
    /// assert_eq!(other_ext.get_ext(), 'a');
    /// ```
    pub fn get_ext(&self) -> char {
        self.ext as char
    }

    /// Gets the tag character for this extension as a byte.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::Locale;
    ///
    /// let loc: Locale = "und-a-hello-world".parse().unwrap();
    /// let other_ext = &loc.extensions.other[0];
    /// assert_eq!(other_ext.get_ext_byte(), b'a');
    /// ```
    pub fn get_ext_byte(&self) -> u8 {
        self.ext
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        f(self.get_ext_str())?;
        self.keys.iter().map(|t| t.as_str()).try_for_each(f)
    }
}

writeable::impl_display_with_writeable!(Other);

impl writeable::Writeable for Other {
    fn write_to<W: core::fmt::Write + ?Sized>(&self, sink: &mut W) -> core::fmt::Result {
        sink.write_str(self.get_ext_str())?;
        for key in self.keys.iter() {
            sink.write_char('-')?;
            writeable::Writeable::write_to(key, sink)?;
        }

        Ok(())
    }

    fn writeable_length_hint(&self) -> writeable::LengthHint {
        let mut result = writeable::LengthHint::exact(1);
        for key in self.keys.iter() {
            result += writeable::Writeable::writeable_length_hint(key) + 1;
        }
        result
    }

    fn write_to_string(&self) -> alloc::borrow::Cow<str> {
        if self.keys.is_empty() {
            return alloc::borrow::Cow::Borrowed(self.get_ext_str());
        }
        let mut string =
            alloc::string::String::with_capacity(self.writeable_length_hint().capacity());
        let _ = self.write_to(&mut string);
        alloc::borrow::Cow::Owned(string)
    }
}
