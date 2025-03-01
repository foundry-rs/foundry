// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Language Identifier and Locale contains a set of subtags
//! which represent different fields of the structure.
//!
//! * [`Language`] is the only mandatory field, which when empty,
//! takes the value `und`.
//! * [`Script`] is an optional field representing the written script used by the locale.
//! * [`Region`] is the region used by the locale.
//! * [`Variants`] is a list of optional [`Variant`] subtags containing information about the
//!                variant adjustments used by the locale.
//!
//! Subtags can be used in isolation, and all basic operations such as parsing, syntax canonicalization
//! and serialization are supported on each individual subtag, but most commonly
//! they are used to construct a [`LanguageIdentifier`] instance.
//!
//! [`Variants`] is a special structure which contains a list of [`Variant`] subtags.
//! It is wrapped around to allow for sorting and deduplication of variants, which
//! is one of the required steps of language identifier and locale syntax canonicalization.
//!
//! # Examples
//!
//! ```
//! use icu::locid::subtags::{Language, Region, Script, Variant};
//!
//! let language: Language =
//!     "en".parse().expect("Failed to parse a language subtag.");
//! let script: Script =
//!     "arab".parse().expect("Failed to parse a script subtag.");
//! let region: Region =
//!     "cn".parse().expect("Failed to parse a region subtag.");
//! let variant: Variant =
//!     "MacOS".parse().expect("Failed to parse a variant subtag.");
//!
//! assert_eq!(language.as_str(), "en");
//! assert_eq!(script.as_str(), "Arab");
//! assert_eq!(region.as_str(), "CN");
//! assert_eq!(variant.as_str(), "macos");
//! ```
//!
//! `Notice`: The subtags are canonicalized on parsing. That means
//! that all operations work on a canonicalized version of the subtag
//! and serialization is very cheap.
//!
//! [`LanguageIdentifier`]: super::LanguageIdentifier
mod language;
mod region;
mod script;
mod variant;
mod variants;

#[doc(inline)]
pub use language::{language, Language};
#[doc(inline)]
pub use region::{region, Region};
#[doc(inline)]
pub use script::{script, Script};
#[doc(inline)]
pub use variant::{variant, Variant};
pub use variants::Variants;
