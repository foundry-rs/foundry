// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Private Use Extensions is a list of extensions intended for
//! private use.
//!
//! Those extensions are treated as a pass-through, and no Unicode related
//! behavior depends on them.
//!
//! The main struct for this extension is [`Private`] which is a list of [`Subtag`]s.
//!
//! # Examples
//!
//! ```
//! use icu::locid::extensions::private::subtag;
//! use icu::locid::{locale, Locale};
//!
//! let mut loc: Locale = "en-US-x-foo-faa".parse().expect("Parsing failed.");
//!
//! assert!(loc.extensions.private.contains(&subtag!("foo")));
//! assert_eq!(loc.extensions.private.iter().next(), Some(&subtag!("foo")));
//!
//! loc.extensions.private.clear();
//!
//! assert!(loc.extensions.private.is_empty());
//! assert_eq!(loc, locale!("en-US"));
//! ```

mod other;

use alloc::vec::Vec;
use core::ops::Deref;

#[doc(inline)]
pub use other::{subtag, Subtag};

use crate::parser::ParserError;
use crate::parser::SubtagIterator;
use crate::shortvec::ShortBoxSlice;

/// A list of [`Private Use Extensions`] as defined in [`Unicode Locale
/// Identifier`] specification.
///
/// Those extensions are treated as a pass-through, and no Unicode related
/// behavior depends on them.
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::private::{Private, Subtag};
///
/// let subtag1: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
/// let subtag2: Subtag = "bar".parse().expect("Failed to parse a Subtag.");
///
/// let private = Private::from_vec_unchecked(vec![subtag1, subtag2]);
/// assert_eq!(&private.to_string(), "x-foo-bar");
/// ```
///
/// [`Private Use Extensions`]: https://unicode.org/reports/tr35/#pu_extensions
/// [`Unicode Locale Identifier`]: https://unicode.org/reports/tr35/#Unicode_locale_identifier
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash, PartialOrd, Ord)]
pub struct Private(ShortBoxSlice<Subtag>);

impl Private {
    /// Returns a new empty list of private-use extensions. Same as [`default()`](Default::default()), but is `const`.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::private::Private;
    ///
    /// assert_eq!(Private::new(), Private::default());
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self(ShortBoxSlice::new())
    }

    /// A constructor which takes a pre-sorted list of [`Subtag`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::private::{Private, Subtag};
    ///
    /// let subtag1: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
    /// let subtag2: Subtag = "bar".parse().expect("Failed to parse a Subtag.");
    ///
    /// let private = Private::from_vec_unchecked(vec![subtag1, subtag2]);
    /// assert_eq!(&private.to_string(), "x-foo-bar");
    /// ```
    pub fn from_vec_unchecked(input: Vec<Subtag>) -> Self {
        Self(input.into())
    }

    /// A constructor which takes a single [`Subtag`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::private::{Private, Subtag};
    ///
    /// let subtag: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
    ///
    /// let private = Private::new_single(subtag);
    /// assert_eq!(&private.to_string(), "x-foo");
    /// ```
    pub const fn new_single(input: Subtag) -> Self {
        Self(ShortBoxSlice::new_single(input))
    }

    /// Empties the [`Private`] list.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::private::{Private, Subtag};
    ///
    /// let subtag1: Subtag = "foo".parse().expect("Failed to parse a Subtag.");
    /// let subtag2: Subtag = "bar".parse().expect("Failed to parse a Subtag.");
    /// let mut private = Private::from_vec_unchecked(vec![subtag1, subtag2]);
    ///
    /// assert_eq!(&private.to_string(), "x-foo-bar");
    ///
    /// private.clear();
    ///
    /// assert_eq!(private, Private::new());
    /// ```
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn try_from_iter(iter: &mut SubtagIterator) -> Result<Self, ParserError> {
        let keys = iter
            .map(Subtag::try_from_bytes)
            .collect::<Result<ShortBoxSlice<_>, _>>()?;

        Ok(Self(keys))
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        if self.is_empty() {
            return Ok(());
        }
        f("x")?;
        self.deref().iter().map(|t| t.as_str()).try_for_each(f)
    }
}

writeable::impl_display_with_writeable!(Private);

impl writeable::Writeable for Private {
    fn write_to<W: core::fmt::Write + ?Sized>(&self, sink: &mut W) -> core::fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        sink.write_str("x")?;
        for key in self.iter() {
            sink.write_char('-')?;
            writeable::Writeable::write_to(key, sink)?;
        }
        Ok(())
    }

    fn writeable_length_hint(&self) -> writeable::LengthHint {
        if self.is_empty() {
            return writeable::LengthHint::exact(0);
        }
        let mut result = writeable::LengthHint::exact(1);
        for key in self.iter() {
            result += writeable::Writeable::writeable_length_hint(key) + 1;
        }
        result
    }
}

impl Deref for Private {
    type Target = [Subtag];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
