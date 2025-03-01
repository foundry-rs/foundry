// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Utilities for performing ordering operations on locales.

use core::cmp::Ordering;

/// The result of a subtag iterator comparison operation.
///
/// See [`Locale::strict_cmp_iter`].
///
/// # Examples
///
/// Check whether a stream of subtags contains two expected locales back-to-back:
///
/// ```
/// use icu::locid::{locale, SubtagOrderingResult};
/// use std::cmp::Ordering;
///
/// let subtags = b"en-US-it-IT".split(|b| *b == b'-');
/// let locales = [locale!("en-US"), locale!("it-IT")];
/// let mut result = SubtagOrderingResult::Subtags(subtags);
/// for loc in locales.iter() {
///     match result {
///         SubtagOrderingResult::Subtags(it) => {
///             result = loc.strict_cmp_iter(it);
///         }
///         SubtagOrderingResult::Ordering(ord) => break,
///     }
/// }
///
/// assert_eq!(Ordering::Equal, result.end());
/// ```
///
/// [`Locale::strict_cmp_iter`]: crate::Locale::strict_cmp_iter
#[allow(clippy::exhaustive_enums)] // well-defined exhaustive enum semantics
#[derive(Debug)]
#[deprecated(since = "1.5.0", note = "if you need this, please file an issue")]
pub enum SubtagOrderingResult<I> {
    /// Potentially remaining subtags after the comparison operation.
    #[deprecated(since = "1.5.0", note = "if you need this, please file an issue")]
    Subtags(I),
    /// Resolved ordering between the locale object and the subtags.
    #[deprecated(since = "1.5.0", note = "if you need this, please file an issue")]
    Ordering(Ordering),
}

#[allow(deprecated)]
impl<I> SubtagOrderingResult<I>
where
    I: Iterator,
{
    /// Invoke this function if there are no remaining locale objects to chain in order to get
    /// a fully resolved [`Ordering`].
    #[inline]
    pub fn end(self) -> Ordering {
        match self {
            Self::Subtags(mut it) => match it.next() {
                Some(_) => Ordering::Less,
                None => Ordering::Equal,
            },
            Self::Ordering(o) => o,
        }
    }
}
