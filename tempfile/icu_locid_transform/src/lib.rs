// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Canonicalization of locale identifiers based on [`CLDR`] data.
//!
//! This module is published as its own crate ([`icu_locid_transform`](https://docs.rs/icu_locid_transform/latest/icu_locid_transform/))
//! and as part of the [`icu`](https://docs.rs/icu/latest/icu/) crate. See the latter for more details on the ICU4X project.
//!
//! It currently supports locale canonicalization based upon the canonicalization
//! algorithm from [`UTS #35: Unicode LDML 3. LocaleId Canonicalization`],
//! as well as the minimize and maximize likely subtags algorithms
//! as described in [`UTS #35: Unicode LDML 3. Likely Subtags`].
//!
//! The maximize method potentially updates a passed in locale in place
//! depending up the results of running the 'Add Likely Subtags' algorithm
//! from [`UTS #35: Unicode LDML 3. Likely Subtags`].
//!
//! This minimize method returns a new Locale that is the result of running the
//! 'Remove Likely Subtags' algorithm from [`UTS #35: Unicode LDML 3. Likely Subtags`].
//!
//! # Examples
//!
//! ```
//! use icu::locid::Locale;
//! use icu::locid_transform::{LocaleCanonicalizer, TransformResult};
//!
//! let lc = LocaleCanonicalizer::new();
//!
//! let mut locale: Locale = "ja-Latn-fonipa-hepburn-heploc"
//!     .parse()
//!     .expect("parse failed");
//! assert_eq!(lc.canonicalize(&mut locale), TransformResult::Modified);
//! assert_eq!(locale, "ja-Latn-alalc97-fonipa".parse::<Locale>().unwrap());
//! ```
//!
//! ```
//! use icu::locid::locale;
//! use icu::locid_transform::{LocaleExpander, TransformResult};
//!
//! let lc = LocaleExpander::new();
//!
//! let mut locale = locale!("zh-CN");
//! assert_eq!(lc.maximize(&mut locale), TransformResult::Modified);
//! assert_eq!(locale, locale!("zh-Hans-CN"));
//!
//! let mut locale = locale!("zh-Hant-TW");
//! assert_eq!(lc.maximize(&mut locale), TransformResult::Unmodified);
//! assert_eq!(locale, locale!("zh-Hant-TW"));
//! ```
//!
//! ```
//! use icu::locid::locale;
//! use icu::locid_transform::{LocaleExpander, TransformResult};
//! use writeable::assert_writeable_eq;
//!
//! let lc = LocaleExpander::new();
//!
//! let mut locale = locale!("zh-Hans-CN");
//! assert_eq!(lc.minimize(&mut locale), TransformResult::Modified);
//! assert_eq!(locale, locale!("zh"));
//!
//! let mut locale = locale!("zh");
//! assert_eq!(lc.minimize(&mut locale), TransformResult::Unmodified);
//! assert_eq!(locale, locale!("zh"));
//! ```
//!
//! [`ICU4X`]: ../icu/index.html
//! [`CLDR`]: http://cldr.unicode.org/
//! [`UTS #35: Unicode LDML 3. Likely Subtags`]: https://www.unicode.org/reports/tr35/#Likely_Subtags.
//! [`UTS #35: Unicode LDML 3. LocaleId Canonicalization`]: http://unicode.org/reports/tr35/#LocaleId_Canonicalization,

// https://github.com/unicode-org/icu4x/blob/main/documents/process/boilerplate.md#library-annotations
#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::exhaustive_structs,
        clippy::exhaustive_enums,
        missing_debug_implementations,
    )
)]
#![warn(missing_docs)]

extern crate alloc;

mod canonicalizer;
mod directionality;
mod error;
mod expander;
pub mod fallback;
pub mod provider;

pub use canonicalizer::LocaleCanonicalizer;
pub use directionality::{Direction, LocaleDirectionality};
pub use error::LocaleTransformError;
pub use expander::LocaleExpander;
#[doc(inline)]
pub use fallback::LocaleFallbacker;

/// Used to track the result of a transformation operation that potentially modifies its argument in place.
#[derive(Debug, PartialEq)]
#[allow(clippy::exhaustive_enums)] // this enum is stable
pub enum TransformResult {
    /// The canonicalization operation modified the locale.
    Modified,
    /// The canonicalization operation did not modify the locale.
    Unmodified,
}

#[doc(no_inline)]
pub use LocaleTransformError as Error;
