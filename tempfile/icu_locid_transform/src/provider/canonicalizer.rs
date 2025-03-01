// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use super::*;
use icu_locid::subtags::{Language, Region, Script, Variant};
use icu_provider::prelude::*;
use tinystr::UnvalidatedTinyAsciiStr;
use zerovec::{VarZeroVec, ZeroMap, ZeroSlice};

#[icu_provider::data_struct(marker(AliasesV1Marker, "locid_transform/aliases@1", singleton))]
#[derive(PartialEq, Clone, Default)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
/// This alias data is used for locale canonicalization. Each field defines a
/// mapping from an old identifier to a new identifier, based upon the rules in
/// from <http://unicode.org/reports/tr35/#LocaleId_Canonicalization>. The data
/// is stored in sorted order, allowing for binary search to identify rules to
/// apply. It is broken down into smaller vectors based upon some characteristic
/// of the data, to help avoid unnecessary searches. For example, the `sgn_region`
/// field contains aliases for sign language and region, so that it is not
/// necessary to search the data unless the input is a sign language.
///
/// The algorithm in tr35 is not guaranteed to terminate on data other than what
/// is currently in CLDR. For this reason, it is not a good idea to attempt to add
/// or modify aliases for use in this structure.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
// TODO: Use validated types as value types
#[derive(Debug)]
pub struct AliasesV1<'data> {
    /// `[language(-variant)+\] -> [langid]`
    /// This is not a map as it's searched linearly according to the canonicalization rules.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_variants: VarZeroVec<'data, UnvalidatedLanguageIdentifierPair>,
    /// `sgn-[region] -> [language]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub sgn_region: ZeroMap<'data, UnvalidatedRegion, Language>,
    /// `[language{2}] -> [langid]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_len2: ZeroMap<'data, UnvalidatedTinyAsciiStr<2>, UnvalidatedLanguageIdentifier>,
    /// `[language{3}] -> [langid]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_len3: ZeroMap<'data, UnvalidatedLanguage, UnvalidatedLanguageIdentifier>,
    /// `[langid] -> [langid]`
    /// This is not a map as it's searched linearly according to the canonicalization rules.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language: VarZeroVec<'data, UnvalidatedLanguageIdentifierPair>,

    /// `[script] -> [script]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script: ZeroMap<'data, UnvalidatedScript, Script>,

    /// `[region{2}] -> [region]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_alpha: ZeroMap<'data, UnvalidatedTinyAsciiStr<2>, Region>,
    /// `[region{3}] -> [region]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_num: ZeroMap<'data, UnvalidatedRegion, Region>,

    /// `[region] -> [region]+`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub complex_region: ZeroMap<'data, UnvalidatedRegion, ZeroSlice<Region>>,

    /// `[variant] -> [variant]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub variant: ZeroMap<'data, UnvalidatedVariant, Variant>,

    /// `[value{7}] -> [value{7}]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub subdivision: ZeroMap<'data, UnvalidatedSubdivision, SemivalidatedSubdivision>,
}

#[cfg(feature = "datagen")]
impl<'data> From<AliasesV2<'data>> for AliasesV1<'data> {
    fn from(value: AliasesV2<'data>) -> Self {
        let language_variants = value
            .language_variants
            .iter()
            .map(zerofrom::ZeroFrom::zero_from)
            .map(|v: LanguageStrStrPair| {
                let langid = alloc::format!("{0}-{1}", v.0, v.1);
                StrStrPair(langid.into(), v.2)
            })
            .collect::<alloc::vec::Vec<StrStrPair>>();

        Self {
            language_variants: VarZeroVec::from(&language_variants),
            sgn_region: value.sgn_region,
            language_len2: value.language_len2,
            language_len3: value.language_len3,
            language: value.language,
            script: value.script,
            region_alpha: value.region_alpha,
            region_num: value.region_num,
            complex_region: value.complex_region,
            variant: value.variant,
            subdivision: value.subdivision,
        }
    }
}

impl<'data> TryFrom<AliasesV1<'data>> for AliasesV2<'data> {
    type Error = icu_provider::DataError;

    fn try_from(value: AliasesV1<'data>) -> Result<Self, Self::Error> {
        #[allow(unused_imports)]
        use alloc::borrow::ToOwned;

        let language_variants = value
            .language_variants
            .iter()
            .map(zerofrom::ZeroFrom::zero_from)
            .map(|v: StrStrPair| -> Result<LanguageStrStrPair, DataError> {
                let (lang, variant) =
                    v.0.split_once('-')
                        .ok_or_else(|| DataError::custom("Each pair should be language-variant"))?;
                let lang: Language = lang
                    .parse()
                    .map_err(|_| DataError::custom("Language should be a valid language subtag"))?;
                Ok(LanguageStrStrPair(lang, variant.to_owned().into(), v.1))
            })
            .collect::<Result<alloc::vec::Vec<_>, _>>()?;

        Ok(Self {
            language_variants: VarZeroVec::from(&language_variants),
            sgn_region: value.sgn_region,
            language_len2: value.language_len2,
            language_len3: value.language_len3,
            language: value.language,
            script: value.script,
            region_alpha: value.region_alpha,
            region_num: value.region_num,
            complex_region: value.complex_region,
            variant: value.variant,
            subdivision: value.subdivision,
        })
    }
}

#[icu_provider::data_struct(marker(AliasesV2Marker, "locid_transform/aliases@2", singleton))]
#[derive(PartialEq, Clone, Default)]
#[cfg_attr(
    feature = "datagen",
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_locid_transform::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[yoke(prove_covariance_manually)]
/// This alias data is used for locale canonicalization. Each field defines a
/// mapping from an old identifier to a new identifier, based upon the rules in
/// from <http://unicode.org/reports/tr35/#LocaleId_Canonicalization>. The data
/// is stored in sorted order, allowing for binary search to identify rules to
/// apply. It is broken down into smaller vectors based upon some characteristic
/// of the data, to help avoid unnecessary searches. For example, the `sgn_region`
/// field contains aliases for sign language and region, so that it is not
/// necessary to search the data unless the input is a sign language.
///
/// The algorithm in tr35 is not guaranteed to terminate on data other than what
/// is currently in CLDR. For this reason, it is not a good idea to attempt to add
/// or modify aliases for use in this structure.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
// TODO: Use validated types as value types
// Notice: V2 improves the alignment of `language_variants` speeding up canonicalization by upon
// to 40%. See https://github.com/unicode-org/icu4x/pull/2935 for details.
#[derive(Debug)]
pub struct AliasesV2<'data> {
    /// `[language, variant(-variant)*] -> [langid]`
    /// This is not a map as it's searched linearly according to the canonicalization rules.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_variants: VarZeroVec<'data, UnvalidatedLanguageVariantsPair>,
    /// `sgn-[region] -> [language]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub sgn_region: ZeroMap<'data, UnvalidatedRegion, Language>,
    /// `[language{2}] -> [langid]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_len2: ZeroMap<'data, UnvalidatedTinyAsciiStr<2>, UnvalidatedLanguageIdentifier>,
    /// `[language{3}] -> [langid]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language_len3: ZeroMap<'data, UnvalidatedLanguage, UnvalidatedLanguageIdentifier>,
    /// `[langid] -> [langid]`
    /// This is not a map as it's searched linearly according to the canonicalization rules.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub language: VarZeroVec<'data, UnvalidatedLanguageIdentifierPair>,

    /// `[script] -> [script]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub script: ZeroMap<'data, UnvalidatedScript, Script>,

    /// `[region{2}] -> [region]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_alpha: ZeroMap<'data, UnvalidatedTinyAsciiStr<2>, Region>,
    /// `[region{3}] -> [region]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_num: ZeroMap<'data, UnvalidatedRegion, Region>,

    /// `[region] -> [region]+`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub complex_region: ZeroMap<'data, UnvalidatedRegion, ZeroSlice<Region>>,

    /// `[variant] -> [variant]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub variant: ZeroMap<'data, UnvalidatedVariant, Variant>,

    /// `[value{7}] -> [value{7}]`
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub subdivision: ZeroMap<'data, UnvalidatedSubdivision, SemivalidatedSubdivision>,
}
