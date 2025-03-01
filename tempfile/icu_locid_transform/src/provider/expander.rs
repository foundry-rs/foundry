// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use super::*;
use icu_locid::subtags::{Language, Region, Script};
use icu_provider::prelude::*;
use zerovec::ZeroMap;

#[icu_provider::data_struct(marker(
    LikelySubtagsV1Marker,
    "locid_transform/likelysubtags@1",
    singleton
))]
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
/// This likely subtags data is used for the minimize and maximize operations.
/// Each field defines a mapping from an old identifier to a new identifier,
/// based upon the rules in
/// <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
///
/// The data is stored is broken down into smaller vectors based upon the rules
/// defined for the likely subtags maximize algorithm.
///
/// For efficiency, only the relevant part of the LanguageIdentifier is stored
/// for searching and replacing. E.g., the `language_script` field is used to store
/// rules for `LanguageIdentifier`s that contain a language and a script, but not a
/// region.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[yoke(prove_covariance_manually)]
pub struct LikelySubtagsV1<'data> {
    /// Language and script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_script: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedScript), Region>,
    /// Language and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_region: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedRegion), Script>,
    /// Just language.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language: ZeroMap<'data, UnvalidatedLanguage, (Script, Region)>,
    /// Script and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script_region: ZeroMap<'data, (UnvalidatedScript, UnvalidatedRegion), Language>,
    /// Just script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script: ZeroMap<'data, UnvalidatedScript, (Language, Region)>,
    /// Just region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region: ZeroMap<'data, UnvalidatedRegion, (Language, Script)>,
    /// Undefined.
    pub und: (Language, Script, Region),
}

#[icu_provider::data_struct(marker(
    LikelySubtagsForLanguageV1Marker,
    "locid_transform/likelysubtags_l@1",
    singleton
))]
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
/// This likely subtags data is used for the minimize and maximize operations.
/// Each field defines a mapping from an old identifier to a new identifier,
/// based upon the rules in
/// <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
///
/// The data is stored is broken down into smaller vectors based upon the rules
/// defined for the likely subtags maximize algorithm.
///
/// For efficiency, only the relevant part of the LanguageIdentifier is stored
/// for searching and replacing. E.g., the `language_script` field is used to store
/// rules for `LanguageIdentifier`s that contain a language and a script, but not a
/// region.
///
/// This struct contains mappings when the input contains a language subtag.
/// Also see [`LikelySubtagsForScriptRegionV1`].
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[yoke(prove_covariance_manually)]
pub struct LikelySubtagsForLanguageV1<'data> {
    /// Language and script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_script: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedScript), Region>,
    /// Language and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_region: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedRegion), Script>,
    /// Just language.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language: ZeroMap<'data, UnvalidatedLanguage, (Script, Region)>,
    /// Undefined.
    pub und: (Language, Script, Region),
}

impl<'data> From<LikelySubtagsV1<'data>> for LikelySubtagsForLanguageV1<'data> {
    fn from(other: LikelySubtagsV1<'data>) -> Self {
        Self {
            language_script: other.language_script,
            language_region: other.language_region,
            language: other.language,
            und: other.und,
        }
    }
}

impl<'data> LikelySubtagsForLanguageV1<'data> {
    pub(crate) fn clone_from_borrowed(other: &LikelySubtagsV1<'data>) -> Self {
        Self {
            language_script: other.language_script.clone(),
            language_region: other.language_region.clone(),
            language: other.language.clone(),
            und: other.und,
        }
    }
}

#[icu_provider::data_struct(marker(
    LikelySubtagsForScriptRegionV1Marker,
    "locid_transform/likelysubtags_sr@1",
    singleton
))]
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
/// This likely subtags data is used for the minimize and maximize operations.
/// Each field defines a mapping from an old identifier to a new identifier,
/// based upon the rules in
/// <https://www.unicode.org/reports/tr35/#Likely_Subtags>.
///
/// The data is stored is broken down into smaller vectors based upon the rules
/// defined for the likely subtags maximize algorithm.
///
/// For efficiency, only the relevant part of the LanguageIdentifier is stored
/// for searching and replacing. E.g., the `script_region` field is used to store
/// rules for `LanguageIdentifier`s that contain a script and a region, but not a
/// language.
///
/// This struct contains mappings when the input does not contain a language subtag.
/// Also see [`LikelySubtagsForLanguageV1`].
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[yoke(prove_covariance_manually)]
pub struct LikelySubtagsForScriptRegionV1<'data> {
    /// Script and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script_region: ZeroMap<'data, (UnvalidatedScript, UnvalidatedRegion), Language>,
    /// Just script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script: ZeroMap<'data, UnvalidatedScript, (Language, Region)>,
    /// Just region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region: ZeroMap<'data, UnvalidatedRegion, (Language, Script)>,
}

impl<'data> From<LikelySubtagsV1<'data>> for LikelySubtagsForScriptRegionV1<'data> {
    fn from(other: LikelySubtagsV1<'data>) -> Self {
        Self {
            script_region: other.script_region,
            script: other.script,
            region: other.region,
        }
    }
}

#[icu_provider::data_struct(marker(
    LikelySubtagsExtendedV1Marker,
    "locid_transform/likelysubtags_ext@1",
    singleton
))]
#[derive(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
/// This likely subtags data is used for full coverage of locales, including ones that
/// don't otherwise have data in the Common Locale Data Repository (CLDR).
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[yoke(prove_covariance_manually)]
pub struct LikelySubtagsExtendedV1<'data> {
    /// Language and script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_script: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedScript), Region>,
    /// Language and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_region: ZeroMap<'data, (UnvalidatedLanguage, UnvalidatedRegion), Script>,
    /// Just language.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language: ZeroMap<'data, UnvalidatedLanguage, (Script, Region)>,
    /// Script and region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script_region: ZeroMap<'data, (UnvalidatedScript, UnvalidatedRegion), Language>,
    /// Just script.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script: ZeroMap<'data, UnvalidatedScript, (Language, Region)>,
    /// Just region.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region: ZeroMap<'data, UnvalidatedRegion, (Language, Script)>,
}

impl<'data> From<LikelySubtagsV1<'data>> for LikelySubtagsExtendedV1<'data> {
    fn from(other: LikelySubtagsV1<'data>) -> Self {
        Self {
            language_script: other.language_script,
            language_region: other.language_region,
            language: other.language,
            script_region: other.script_region,
            script: other.script,
            region: other.region,
        }
    }
}
