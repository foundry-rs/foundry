// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use super::Attribute;

use crate::shortvec::ShortBoxSlice;
use alloc::vec::Vec;
use core::ops::Deref;

/// A set of [`Attribute`] elements as defined in [`Unicode Extension Attributes`].
///
/// [`Unicode Extension Attributes`]: https://unicode.org/reports/tr35/tr35.html#u_Extension
///
/// # Examples
///
/// ```
/// use icu::locid::extensions::unicode::{Attribute, Attributes};
///
/// let attribute1: Attribute =
///     "foobar".parse().expect("Failed to parse a variant subtag.");
///
/// let attribute2: Attribute = "testing"
///     .parse()
///     .expect("Failed to parse a variant subtag.");
/// let mut v = vec![attribute1, attribute2];
/// v.sort();
/// v.dedup();
///
/// let attributes: Attributes = Attributes::from_vec_unchecked(v);
/// assert_eq!(attributes.to_string(), "foobar-testing");
/// ```
#[derive(Default, Debug, PartialEq, Eq, Clone, Hash, PartialOrd, Ord)]
pub struct Attributes(ShortBoxSlice<Attribute>);

impl Attributes {
    /// Returns a new empty set of attributes. Same as [`default()`](Default::default()), but is `const`.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::Attributes;
    ///
    /// assert_eq!(Attributes::new(), Attributes::default());
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self(ShortBoxSlice::new())
    }

    /// A constructor which takes a pre-sorted list of [`Attribute`] elements.
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::{Attribute, Attributes};
    ///
    /// let attribute1: Attribute = "foobar".parse().expect("Parsing failed.");
    /// let attribute2: Attribute = "testing".parse().expect("Parsing failed.");
    /// let mut v = vec![attribute1, attribute2];
    /// v.sort();
    /// v.dedup();
    ///
    /// let attributes = Attributes::from_vec_unchecked(v);
    /// ```
    ///
    /// Notice: For performance- and memory-constrained environments, it is recommended
    /// for the caller to use [`binary_search`](slice::binary_search) instead of [`sort`](slice::sort)
    /// and [`dedup`](Vec::dedup()).
    pub fn from_vec_unchecked(input: Vec<Attribute>) -> Self {
        Self(input.into())
    }

    pub(crate) fn from_short_slice_unchecked(input: ShortBoxSlice<Attribute>) -> Self {
        Self(input)
    }

    /// Empties the [`Attributes`] list.
    ///
    /// Returns the old list.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::{attribute, Attributes};
    /// use writeable::assert_writeable_eq;
    ///
    /// let mut attributes = Attributes::from_vec_unchecked(vec![
    ///     attribute!("foobar"),
    ///     attribute!("testing"),
    /// ]);
    ///
    /// assert_writeable_eq!(attributes, "foobar-testing");
    ///
    /// attributes.clear();
    ///
    /// assert_writeable_eq!(attributes, "");
    /// ```
    pub fn clear(&mut self) -> Self {
        core::mem::take(self)
    }

    pub(crate) fn for_each_subtag_str<E, F>(&self, f: &mut F) -> Result<(), E>
    where
        F: FnMut(&str) -> Result<(), E>,
    {
        self.deref().iter().map(|t| t.as_str()).try_for_each(f)
    }
}

impl_writeable_for_subtag_list!(Attributes, "foobar", "testing");

impl Deref for Attributes {
    type Target = [Attribute];

    fn deref(&self) -> &[Attribute] {
        self.0.deref()
    }
}
