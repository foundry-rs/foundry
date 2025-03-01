// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Definitions of [Unicode Properties] and APIs for
//! retrieving property data in an appropriate data structure.
//!
//! This module is published as its own crate ([`icu_properties`](https://docs.rs/icu_properties/latest/icu_properties/))
//! and as part of the [`icu`](https://docs.rs/icu/latest/icu/) crate. See the latter for more details on the ICU4X project.
//!
//! APIs that return a [`CodePointSetData`] exist for binary properties and certain enumerated
//! properties. See the [`sets`] module for more details.
//!
//! APIs that return a [`CodePointMapData`] exist for certain enumerated properties. See the
//! [`maps`] module for more details.
//!
//! # Examples
//!
//! ## Property data as `CodePointSetData`s
//!
//! ```
//! use icu::properties::{maps, sets, GeneralCategory};
//!
//! // A binary property as a `CodePointSetData`
//!
//! assert!(sets::emoji().contains('🎃')); // U+1F383 JACK-O-LANTERN
//! assert!(!sets::emoji().contains('木')); // U+6728
//!
//! // An individual enumerated property value as a `CodePointSetData`
//!
//! let line_sep_data = maps::general_category()
//!     .get_set_for_value(GeneralCategory::LineSeparator);
//! let line_sep = line_sep_data.as_borrowed();
//!
//! assert!(line_sep.contains32(0x2028));
//! assert!(!line_sep.contains32(0x2029));
//! ```
//!
//! ## Property data as `CodePointMapData`s
//!
//! ```
//! use icu::properties::{maps, Script};
//!
//! assert_eq!(maps::script().get('🎃'), Script::Common); // U+1F383 JACK-O-LANTERN
//! assert_eq!(maps::script().get('木'), Script::Han); // U+6728
//! ```
//!
//! [`ICU4X`]: ../icu/index.html
//! [Unicode Properties]: https://unicode-org.github.io/icu/userguide/strings/properties.html
//! [`CodePointSetData`]: crate::sets::CodePointSetData
//! [`CodePointMapData`]: crate::maps::CodePointMapData
//! [`sets`]: crate::sets

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

#[cfg(feature = "bidi")]
pub mod bidi;

mod error;
pub mod maps;

// NOTE: The Pernosco debugger has special knowledge
// of the `CanonicalCombiningClass` struct inside the `props`
// module. Please do not change the crate-module-qualified
// name of that struct without coordination.
mod props;

pub mod bidi_data;
pub mod exemplar_chars;
pub mod provider;
pub(crate) mod runtime;
#[allow(clippy::exhaustive_structs)] // TODO
pub mod script;
pub mod sets;
mod trievalue;

pub use props::{
    BidiClass, CanonicalCombiningClass, EastAsianWidth, GeneralCategory, GeneralCategoryGroup,
    GraphemeClusterBreak, HangulSyllableType, IndicSyllabicCategory, JoiningType, LineBreak,
    Script, SentenceBreak, WordBreak,
};

/// Module for working with the names of property values
pub mod names {
    pub use crate::props::{
        PropertyEnumToValueNameLinearMapper, PropertyEnumToValueNameLinearMapperBorrowed,
    };
    pub use crate::props::{
        PropertyEnumToValueNameLinearTiny4Mapper, PropertyEnumToValueNameLinearTiny4MapperBorrowed,
    };
    pub use crate::props::{
        PropertyEnumToValueNameSparseMapper, PropertyEnumToValueNameSparseMapperBorrowed,
    };
    pub use crate::props::{PropertyValueNameToEnumMapper, PropertyValueNameToEnumMapperBorrowed};
}

pub use error::PropertiesError;

#[doc(no_inline)]
pub use PropertiesError as Error;
