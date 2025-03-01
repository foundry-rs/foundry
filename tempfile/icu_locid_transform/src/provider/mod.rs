// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

// Provider structs must be stable
#![allow(clippy::exhaustive_structs, clippy::exhaustive_enums)]

//! 🚧 \[Unstable\] Data provider struct definitions for this ICU4X component.
//!
//! <div class="stab unstable">
//! 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
//! including in SemVer minor releases. While the serde representation of data structs is guaranteed
//! to be stable, their Rust representation might not be. Use with caution.
//! </div>
//!
//! Read more about data providers: [`icu_provider`]

mod canonicalizer;
pub use canonicalizer::*;
use icu_locid::subtags::Language;
mod directionality;
pub use directionality::*;
mod expander;
pub use expander::*;
mod fallback;
pub use fallback::*;

#[cfg(feature = "compiled_data")]
#[derive(Debug)]
/// Baked data
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. In particular, the `DataProvider` implementations are only
/// guaranteed to match with this version's `*_unstable` providers. Use with caution.
/// </div>
pub struct Baked;

#[cfg(feature = "compiled_data")]
const _: () = {
    pub mod icu {
        pub use crate as locid_transform;
        pub use icu_locid as locid;
    }
    icu_locid_transform_data::make_provider!(Baked);
    icu_locid_transform_data::impl_fallback_likelysubtags_v1!(Baked);
    icu_locid_transform_data::impl_fallback_parents_v1!(Baked);
    icu_locid_transform_data::impl_fallback_supplement_co_v1!(Baked);
    icu_locid_transform_data::impl_locid_transform_aliases_v2!(Baked);
    icu_locid_transform_data::impl_locid_transform_likelysubtags_ext_v1!(Baked);
    icu_locid_transform_data::impl_locid_transform_likelysubtags_l_v1!(Baked);
    icu_locid_transform_data::impl_locid_transform_likelysubtags_sr_v1!(Baked);
    icu_locid_transform_data::impl_locid_transform_script_dir_v1!(Baked);
};

#[cfg(feature = "datagen")]
use icu_provider::prelude::*;

#[cfg(feature = "datagen")]
/// The latest minimum set of keys required by this component.
pub const KEYS: &[DataKey] = &[
    AliasesV2Marker::KEY,
    CollationFallbackSupplementV1Marker::KEY,
    LikelySubtagsExtendedV1Marker::KEY,
    LikelySubtagsForLanguageV1Marker::KEY,
    LikelySubtagsForScriptRegionV1Marker::KEY,
    LocaleFallbackLikelySubtagsV1Marker::KEY,
    LocaleFallbackParentsV1Marker::KEY,
    ScriptDirectionV1Marker::KEY,
];

use alloc::borrow::Cow;
use tinystr::{TinyAsciiStr, UnvalidatedTinyAsciiStr};

// We use raw TinyAsciiStrs for map keys, as we then don't have to
// validate them as subtags on deserialization. Map lookup can be
// done even if they are not valid tags (an invalid key will just
// become inaccessible).
type UnvalidatedLanguage = UnvalidatedTinyAsciiStr<3>;
type UnvalidatedScript = UnvalidatedTinyAsciiStr<4>;
type UnvalidatedRegion = UnvalidatedTinyAsciiStr<3>;
type UnvalidatedVariant = UnvalidatedTinyAsciiStr<8>;
type UnvalidatedSubdivision = UnvalidatedTinyAsciiStr<7>;
type SemivalidatedSubdivision = TinyAsciiStr<7>;

// LanguageIdentifier doesn't have an AsULE implementation, so we have
// to store strs and parse when needed.
type UnvalidatedLanguageIdentifier = str;
type UnvalidatedLanguageIdentifierPair = StrStrPairVarULE;
type UnvalidatedLanguageVariantsPair = LanguageStrStrPairVarULE;

#[zerovec::make_varule(StrStrPairVarULE)]
#[zerovec::derive(Debug)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    zerovec::derive(Deserialize)
)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    zerovec::derive(Serialize),
    databake(path = icu_locid_transform::provider),
)]
/// A pair of strings with a EncodeAsVarULE implementation.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
pub struct StrStrPair<'a>(
    #[cfg_attr(feature = "serde", serde(borrow))] pub Cow<'a, str>,
    #[cfg_attr(feature = "serde", serde(borrow))] pub Cow<'a, str>,
);

#[zerovec::make_varule(LanguageStrStrPairVarULE)]
#[zerovec::derive(Debug)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    zerovec::derive(Deserialize)
)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    zerovec::derive(Serialize),
    databake(path = icu_locid_transform::provider),
)]
/// A triplet of strings with a EncodeAsVarULE implementation.
pub struct LanguageStrStrPair<'a>(
    pub Language,
    #[cfg_attr(feature = "serde", serde(borrow))] pub Cow<'a, str>,
    #[cfg_attr(feature = "serde", serde(borrow))] pub Cow<'a, str>,
);
