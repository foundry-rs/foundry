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

pub mod names;

use crate::script::ScriptWithExt;
use crate::Script;

use core::ops::RangeInclusive;
use core::str;
use icu_collections::codepointinvlist::CodePointInversionList;
use icu_collections::codepointinvliststringlist::CodePointInversionListAndStringList;
use icu_collections::codepointtrie::{CodePointMapRange, CodePointTrie, TrieValue};
use icu_provider::prelude::*;
use icu_provider::{DataKeyMetadata, FallbackPriority};
use zerofrom::ZeroFrom;

use zerovec::{VarZeroVec, ZeroSlice, ZeroVecError};

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
        pub use crate as properties;
        pub use icu_collections as collections;
        #[allow(unused_imports)] // baked data may or may not need this
        pub use icu_locid_transform as locid_transform;
    }
    icu_properties_data::make_provider!(Baked);
    icu_properties_data::impl_propnames_from_gcb_v1!(Baked);
    icu_properties_data::impl_propnames_from_bc_v1!(Baked);
    icu_properties_data::impl_propnames_from_ccc_v1!(Baked);
    icu_properties_data::impl_propnames_from_ea_v1!(Baked);
    icu_properties_data::impl_propnames_from_gc_v1!(Baked);
    icu_properties_data::impl_propnames_from_gcm_v1!(Baked);
    icu_properties_data::impl_propnames_from_hst_v1!(Baked);
    icu_properties_data::impl_propnames_from_insc_v1!(Baked);
    icu_properties_data::impl_propnames_from_jt_v1!(Baked);
    icu_properties_data::impl_propnames_from_lb_v1!(Baked);
    icu_properties_data::impl_propnames_from_sb_v1!(Baked);
    icu_properties_data::impl_propnames_from_sc_v1!(Baked);
    icu_properties_data::impl_propnames_from_wb_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_bc_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_ea_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_gc_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_gcb_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_hst_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_insc_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_jt_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_lb_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_sb_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_sc_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_linear_wb_v1!(Baked);
    icu_properties_data::impl_propnames_to_long_sparse_ccc_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_bc_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_ea_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_gc_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_gcb_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_hst_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_insc_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_jt_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_lb_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_sb_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear_wb_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_linear4_sc_v1!(Baked);
    icu_properties_data::impl_propnames_to_short_sparse_ccc_v1!(Baked);
    icu_properties_data::impl_props_ahex_v1!(Baked);
    icu_properties_data::impl_props_alnum_v1!(Baked);
    icu_properties_data::impl_props_alpha_v1!(Baked);
    icu_properties_data::impl_props_basic_emoji_v1!(Baked);
    icu_properties_data::impl_props_bc_v1!(Baked);
    icu_properties_data::impl_props_bidi_c_v1!(Baked);
    icu_properties_data::impl_props_bidi_m_v1!(Baked);
    icu_properties_data::impl_props_bidiauxiliaryprops_v1!(Baked);
    icu_properties_data::impl_props_blank_v1!(Baked);
    icu_properties_data::impl_props_cased_v1!(Baked);
    icu_properties_data::impl_props_ccc_v1!(Baked);
    icu_properties_data::impl_props_ci_v1!(Baked);
    icu_properties_data::impl_props_comp_ex_v1!(Baked);
    icu_properties_data::impl_props_cwcf_v1!(Baked);
    icu_properties_data::impl_props_cwcm_v1!(Baked);
    icu_properties_data::impl_props_cwkcf_v1!(Baked);
    icu_properties_data::impl_props_cwl_v1!(Baked);
    icu_properties_data::impl_props_cwt_v1!(Baked);
    icu_properties_data::impl_props_cwu_v1!(Baked);
    icu_properties_data::impl_props_dash_v1!(Baked);
    icu_properties_data::impl_props_dep_v1!(Baked);
    icu_properties_data::impl_props_di_v1!(Baked);
    icu_properties_data::impl_props_dia_v1!(Baked);
    icu_properties_data::impl_props_ea_v1!(Baked);
    icu_properties_data::impl_props_ebase_v1!(Baked);
    icu_properties_data::impl_props_ecomp_v1!(Baked);
    icu_properties_data::impl_props_emod_v1!(Baked);
    icu_properties_data::impl_props_emoji_v1!(Baked);
    icu_properties_data::impl_props_epres_v1!(Baked);
    icu_properties_data::impl_props_exemplarchars_auxiliary_v1!(Baked);
    icu_properties_data::impl_props_exemplarchars_index_v1!(Baked);
    icu_properties_data::impl_props_exemplarchars_main_v1!(Baked);
    icu_properties_data::impl_props_exemplarchars_numbers_v1!(Baked);
    icu_properties_data::impl_props_exemplarchars_punctuation_v1!(Baked);
    icu_properties_data::impl_props_ext_v1!(Baked);
    icu_properties_data::impl_props_extpict_v1!(Baked);
    icu_properties_data::impl_props_gc_v1!(Baked);
    icu_properties_data::impl_props_gcb_v1!(Baked);
    icu_properties_data::impl_props_gr_base_v1!(Baked);
    icu_properties_data::impl_props_gr_ext_v1!(Baked);
    icu_properties_data::impl_props_gr_link_v1!(Baked);
    icu_properties_data::impl_props_graph_v1!(Baked);
    icu_properties_data::impl_props_hex_v1!(Baked);
    icu_properties_data::impl_props_hst_v1!(Baked);
    icu_properties_data::impl_props_hyphen_v1!(Baked);
    icu_properties_data::impl_props_idc_v1!(Baked);
    icu_properties_data::impl_props_ideo_v1!(Baked);
    icu_properties_data::impl_props_ids_v1!(Baked);
    icu_properties_data::impl_props_idsb_v1!(Baked);
    icu_properties_data::impl_props_idst_v1!(Baked);
    icu_properties_data::impl_props_insc_v1!(Baked);
    icu_properties_data::impl_props_join_c_v1!(Baked);
    icu_properties_data::impl_props_jt_v1!(Baked);
    icu_properties_data::impl_props_lb_v1!(Baked);
    icu_properties_data::impl_props_loe_v1!(Baked);
    icu_properties_data::impl_props_lower_v1!(Baked);
    icu_properties_data::impl_props_math_v1!(Baked);
    icu_properties_data::impl_props_nchar_v1!(Baked);
    icu_properties_data::impl_props_nfcinert_v1!(Baked);
    icu_properties_data::impl_props_nfdinert_v1!(Baked);
    icu_properties_data::impl_props_nfkcinert_v1!(Baked);
    icu_properties_data::impl_props_nfkdinert_v1!(Baked);
    icu_properties_data::impl_props_pat_syn_v1!(Baked);
    icu_properties_data::impl_props_pat_ws_v1!(Baked);
    icu_properties_data::impl_props_pcm_v1!(Baked);
    icu_properties_data::impl_props_print_v1!(Baked);
    icu_properties_data::impl_props_qmark_v1!(Baked);
    icu_properties_data::impl_props_radical_v1!(Baked);
    icu_properties_data::impl_props_ri_v1!(Baked);
    icu_properties_data::impl_props_sb_v1!(Baked);
    icu_properties_data::impl_props_sc_v1!(Baked);
    icu_properties_data::impl_props_scx_v1!(Baked);
    icu_properties_data::impl_props_sd_v1!(Baked);
    icu_properties_data::impl_props_segstart_v1!(Baked);
    icu_properties_data::impl_props_sensitive_v1!(Baked);
    icu_properties_data::impl_props_sterm_v1!(Baked);
    icu_properties_data::impl_props_term_v1!(Baked);
    icu_properties_data::impl_props_uideo_v1!(Baked);
    icu_properties_data::impl_props_upper_v1!(Baked);
    icu_properties_data::impl_props_vs_v1!(Baked);
    icu_properties_data::impl_props_wb_v1!(Baked);
    icu_properties_data::impl_props_wspace_v1!(Baked);
    icu_properties_data::impl_props_xdigit_v1!(Baked);
    icu_properties_data::impl_props_xidc_v1!(Baked);
    icu_properties_data::impl_props_xids_v1!(Baked);
};

// include the specialized structs for the compact representation of Bidi property data
pub mod bidi_data;

/// A set of characters which share a particular property value.
///
/// This data enum is extensible, more backends may be added in the future.
/// Old data can be used with newer code but not vice versa.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Eq, PartialEq, Clone, yoke::Yokeable, zerofrom::ZeroFrom)]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[non_exhaustive]
pub enum PropertyCodePointSetV1<'data> {
    /// The set of characters, represented as an inversion list
    InversionList(#[cfg_attr(feature = "serde", serde(borrow))] CodePointInversionList<'data>),
    // new variants should go BELOW existing ones
    // Serde serializes based on variant name and index in the enum
    // https://docs.rs/serde/latest/serde/trait.Serializer.html#tymethod.serialize_unit_variant
}

/// A map efficiently storing data about individual characters.
///
/// This data enum is extensible, more backends may be added in the future.
/// Old data can be used with newer code but not vice versa.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Clone, Debug, Eq, PartialEq, yoke::Yokeable, zerofrom::ZeroFrom)]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[non_exhaustive]
pub enum PropertyCodePointMapV1<'data, T: TrieValue> {
    /// A codepoint trie storing the data
    CodePointTrie(#[cfg_attr(feature = "serde", serde(borrow))] CodePointTrie<'data, T>),
    // new variants should go BELOW existing ones
    // Serde serializes based on variant name and index in the enum
    // https://docs.rs/serde/latest/serde/trait.Serializer.html#tymethod.serialize_unit_variant
}

/// A set of characters and strings which share a particular property value.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[derive(Debug, Eq, PartialEq, Clone, yoke::Yokeable, zerofrom::ZeroFrom)]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[non_exhaustive]
pub enum PropertyUnicodeSetV1<'data> {
    /// A set representing characters in an inversion list, and the strings in a list.
    CPInversionListStrList(
        #[cfg_attr(feature = "serde", serde(borrow))] CodePointInversionListAndStringList<'data>,
    ),
    // new variants should go BELOW existing ones
    // Serde serializes based on variant name and index in the enum
    // https://docs.rs/serde/latest/serde/trait.Serializer.html#tymethod.serialize_unit_variant
}

impl<'data> PropertyUnicodeSetV1<'data> {
    #[inline]
    pub(crate) fn contains(&self, s: &str) -> bool {
        match *self {
            Self::CPInversionListStrList(ref l) => l.contains(s),
        }
    }

    #[inline]
    pub(crate) fn contains32(&self, cp: u32) -> bool {
        match *self {
            Self::CPInversionListStrList(ref l) => l.contains32(cp),
        }
    }

    #[inline]
    pub(crate) fn contains_char(&self, ch: char) -> bool {
        match *self {
            Self::CPInversionListStrList(ref l) => l.contains_char(ch),
        }
    }

    #[inline]
    pub(crate) fn from_code_point_inversion_list_string_list(
        l: CodePointInversionListAndStringList<'static>,
    ) -> Self {
        Self::CPInversionListStrList(l)
    }

    #[inline]
    pub(crate) fn as_code_point_inversion_list_string_list(
        &'_ self,
    ) -> Option<&'_ CodePointInversionListAndStringList<'data>> {
        match *self {
            Self::CPInversionListStrList(ref l) => Some(l),
            // any other backing data structure that cannot return a CPInversionListStrList in O(1) time should return None
        }
    }

    #[inline]
    pub(crate) fn to_code_point_inversion_list_string_list(
        &self,
    ) -> CodePointInversionListAndStringList<'_> {
        match *self {
            Self::CPInversionListStrList(ref t) => ZeroFrom::zero_from(t),
        }
    }
}

/// A struct that efficiently stores `Script` and `Script_Extensions` property data.
///
/// <div class="stab unstable">
/// 🚧 This code is considered unstable; it may change at any time, in breaking or non-breaking ways,
/// including in SemVer minor releases. While the serde representation of data structs is guaranteed
/// to be stable, their Rust representation might not be. Use with caution.
/// </div>
#[icu_provider::data_struct(marker(
    ScriptWithExtensionsPropertyV1Marker,
    "props/scx@1",
    singleton
))]
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(
    feature = "datagen", 
    derive(serde::Serialize, databake::Bake),
    databake(path = icu_properties::provider),
)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct ScriptWithExtensionsPropertyV1<'data> {
    /// Note: The `ScriptWithExt` values in this array will assume a 12-bit layout. The 2
    /// higher order bits 11..10 will indicate how to deduce the Script value and
    /// Script_Extensions value, nearly matching the representation
    /// [in ICU](https://github.com/unicode-org/icu/blob/main/icu4c/source/common/uprops.h):
    ///
    /// | High order 2 bits value | Script                                                 | Script_Extensions                                              |
    /// |-------------------------|--------------------------------------------------------|----------------------------------------------------------------|
    /// | 3                       | First value in sub-array, index given by lower 10 bits | Sub-array excluding first value, index given by lower 10 bits  |
    /// | 2                       | Script=Inherited                                       | Entire sub-array, index given by lower 10 bits                 |
    /// | 1                       | Script=Common                                          | Entire sub-array, index given by lower 10 bits                 |
    /// | 0                       | Value in lower 10 bits                                 | `[ Script value ]` single-element array                        |
    ///
    /// When the lower 10 bits of the value are used as an index, that index is
    /// used for the outer-level vector of the nested `extensions` structure.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub trie: CodePointTrie<'data, ScriptWithExt>,

    /// This companion structure stores Script_Extensions values, which are
    /// themselves arrays / vectors. This structure only stores the values for
    /// cases in which `scx(cp) != [ sc(cp) ]`. Each sub-vector is distinct. The
    /// sub-vector represents the Script_Extensions array value for a code point,
    /// and may also indicate Script value, as described for the `trie` field.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub extensions: VarZeroVec<'data, ZeroSlice<Script>>,
}

impl<'data> ScriptWithExtensionsPropertyV1<'data> {
    // This method is intended to be used by constructors of deserialized data
    // in a data provider.
    #[doc(hidden)]
    pub fn new(
        trie: CodePointTrie<'data, ScriptWithExt>,
        extensions: VarZeroVec<'data, ZeroSlice<Script>>,
    ) -> ScriptWithExtensionsPropertyV1<'data> {
        ScriptWithExtensionsPropertyV1 { trie, extensions }
    }
}

// See CodePointSetData for documentation of these functions
impl<'data> PropertyCodePointSetV1<'data> {
    #[inline]
    pub(crate) fn contains(&self, ch: char) -> bool {
        match *self {
            Self::InversionList(ref l) => l.contains(ch),
        }
    }

    #[inline]
    pub(crate) fn contains32(&self, ch: u32) -> bool {
        match *self {
            Self::InversionList(ref l) => l.contains32(ch),
        }
    }

    #[inline]
    pub(crate) fn iter_ranges(&self) -> impl Iterator<Item = RangeInclusive<u32>> + '_ {
        match *self {
            Self::InversionList(ref l) => l.iter_ranges(),
        }
    }

    #[inline]
    pub(crate) fn iter_ranges_complemented(
        &self,
    ) -> impl Iterator<Item = RangeInclusive<u32>> + '_ {
        match *self {
            Self::InversionList(ref l) => l.iter_ranges_complemented(),
        }
    }

    #[inline]
    pub(crate) fn from_code_point_inversion_list(l: CodePointInversionList<'static>) -> Self {
        Self::InversionList(l)
    }

    #[inline]
    pub(crate) fn as_code_point_inversion_list(
        &'_ self,
    ) -> Option<&'_ CodePointInversionList<'data>> {
        match *self {
            Self::InversionList(ref l) => Some(l),
            // any other backing data structure that cannot return a CPInvList in O(1) time should return None
        }
    }

    #[inline]
    pub(crate) fn to_code_point_inversion_list(&self) -> CodePointInversionList<'_> {
        match *self {
            Self::InversionList(ref t) => ZeroFrom::zero_from(t),
        }
    }
}

// See CodePointMapData for documentation of these functions
impl<'data, T: TrieValue> PropertyCodePointMapV1<'data, T> {
    #[inline]
    pub(crate) fn get32(&self, ch: u32) -> T {
        match *self {
            Self::CodePointTrie(ref t) => t.get32(ch),
        }
    }

    #[inline]
    pub(crate) fn try_into_converted<P>(
        self,
    ) -> Result<PropertyCodePointMapV1<'data, P>, ZeroVecError>
    where
        P: TrieValue,
    {
        match self {
            Self::CodePointTrie(t) => t
                .try_into_converted()
                .map(PropertyCodePointMapV1::CodePointTrie),
        }
    }

    #[inline]
    pub(crate) fn get_set_for_value(&self, value: T) -> CodePointInversionList<'static> {
        match *self {
            Self::CodePointTrie(ref t) => t.get_set_for_value(value),
        }
    }

    #[inline]
    pub(crate) fn iter_ranges(&self) -> impl Iterator<Item = CodePointMapRange<T>> + '_ {
        match *self {
            Self::CodePointTrie(ref t) => t.iter_ranges(),
        }
    }
    #[inline]
    pub(crate) fn iter_ranges_mapped<'a, U: Eq + 'a>(
        &'a self,
        map: impl FnMut(T) -> U + Copy + 'a,
    ) -> impl Iterator<Item = CodePointMapRange<U>> + 'a {
        match *self {
            Self::CodePointTrie(ref t) => t.iter_ranges_mapped(map),
        }
    }

    #[inline]
    pub(crate) fn from_code_point_trie(trie: CodePointTrie<'static, T>) -> Self {
        Self::CodePointTrie(trie)
    }

    #[inline]
    pub(crate) fn as_code_point_trie(&self) -> Option<&CodePointTrie<'data, T>> {
        match *self {
            Self::CodePointTrie(ref t) => Some(t),
            // any other backing data structure that cannot return a CPT in O(1) time should return None
        }
    }

    #[inline]
    pub(crate) fn to_code_point_trie(&self) -> CodePointTrie<'_, T> {
        match *self {
            Self::CodePointTrie(ref t) => ZeroFrom::zero_from(t),
        }
    }
}

macro_rules! expand {
    (
        ($(($code_point_set_marker:ident, $bin_cp_s:literal),)+),
        ($(($unicode_set_marker:ident, $bin_us_s:literal, $us_singleton:literal),)+),
        ($(($code_point_map_marker:ident,
            $name_value_marker:ident,

            $((sparse: $value_short_name_marker_sparse:ident, $value_long_name_marker_sparse:ident),)?
            $((linear: $value_short_name_marker_linear:ident, $value_long_name_marker_linear:ident ),)?
            $((linear4: $value_short_name_marker_linear4:ident, $value_long_name_marker_linear4:ident ),)?
            $enum_s:literal, $value_ty:ident),)+)
    ) => {

            // Data keys that return code point sets (represented as CodePointSetData).
            // For now, synonymous with binary properties of code points only.
            $(
                #[doc = core::concat!("Data marker for the '", $bin_cp_s, "' Unicode property")]
                #[derive(Debug, Default)]
                #[cfg_attr(
                    feature = "datagen",
                    derive(databake::Bake),
                    databake(path = icu_properties::provider),
                )]
                pub struct $code_point_set_marker;

                impl DataMarker for $code_point_set_marker {
                    type Yokeable = PropertyCodePointSetV1<'static>;
                }
                impl KeyedDataMarker for $code_point_set_marker {
                    const KEY: DataKey = data_key!(concat!("props/", $bin_cp_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                }

            )+

            // Data keys that return sets of strings + code points (represented as UnicodeSetData).
            // Includes:
            //   - binary properties of strings + code points
            //   - exemplar characters
            $(
                #[doc = core::concat!("Data marker for the '", $bin_us_s, "' Unicode property")]
                #[derive(Debug, Default)]
                #[cfg_attr(
                    feature = "datagen",
                    derive(databake::Bake),
                    databake(path = icu_properties::provider),
                )]
                pub struct $unicode_set_marker;

                impl DataMarker for $unicode_set_marker {
                    type Yokeable = PropertyUnicodeSetV1<'static>;
                }
                impl KeyedDataMarker for $unicode_set_marker {
                    const KEY: DataKey = data_key!(concat!("props/", $bin_us_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, $us_singleton));
                }
            )+

            // Data keys that return code point map (represented as CodePointMapData).
            // For now, synonymous with enumerated properties [of code points only].
            $(
                #[doc = core::concat!("Data marker for the '", $enum_s, "' Unicode property")]
                #[derive(Debug, Default)]
                #[cfg_attr(
                    feature = "datagen",
                    derive(databake::Bake),
                    databake(path = icu_properties::provider),
                )]
                pub struct $code_point_map_marker;

                impl DataMarker for $code_point_map_marker {
                    type Yokeable = PropertyCodePointMapV1<'static, crate::$value_ty>;
                }

                impl KeyedDataMarker for $code_point_map_marker {
                    const KEY: DataKey = data_key!(concat!("props/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                }


                #[doc = core::concat!("Data marker for parsing the names of the values of the '", $enum_s, "' Unicode property")]
                #[derive(Debug, Default)]
                #[cfg_attr(
                    feature = "datagen",
                    derive(databake::Bake),
                    databake(path = icu_properties::provider),
                )]
                pub struct $name_value_marker;

                impl DataMarker for $name_value_marker {
                    type Yokeable = names::PropertyValueNameToEnumMapV1<'static>;
                }

                impl KeyedDataMarker for $name_value_marker {
                    const KEY: DataKey = data_key!(concat!("propnames/from/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                }

                $(
                    #[doc = core::concat!("Data marker for producing short names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_short_name_marker_sparse;

                    impl DataMarker for $value_short_name_marker_sparse {
                        type Yokeable = names::PropertyEnumToValueNameSparseMapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_short_name_marker_sparse {
                        const KEY: DataKey = data_key!(concat!("propnames/to/short/sparse/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }

                    #[doc = core::concat!("Data marker for producing long names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_long_name_marker_sparse;

                    impl DataMarker for $value_long_name_marker_sparse {
                        type Yokeable = names::PropertyEnumToValueNameSparseMapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_long_name_marker_sparse {
                        const KEY: DataKey = data_key!(concat!("propnames/to/long/sparse/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }
                )?

                $(
                    #[doc = core::concat!("Data marker for producing short names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_short_name_marker_linear;

                    impl DataMarker for $value_short_name_marker_linear {
                        type Yokeable = names::PropertyEnumToValueNameLinearMapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_short_name_marker_linear {
                        const KEY: DataKey = data_key!(concat!("propnames/to/short/linear/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }

                    #[doc = core::concat!("Data marker for producing long names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_long_name_marker_linear;

                    impl DataMarker for $value_long_name_marker_linear {
                        type Yokeable = names::PropertyEnumToValueNameLinearMapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_long_name_marker_linear {
                        const KEY: DataKey = data_key!(concat!("propnames/to/long/linear/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }
                )?

                $(
                    #[doc = core::concat!("Data marker for producing short names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_short_name_marker_linear4;

                    impl DataMarker for $value_short_name_marker_linear4 {
                        type Yokeable = names::PropertyEnumToValueNameLinearTiny4MapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_short_name_marker_linear4 {
                        const KEY: DataKey = data_key!(concat!("propnames/to/short/linear4/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }

                    #[doc = core::concat!("Data marker for producing long names of the values of the '", $enum_s, "' Unicode property")]
                    #[derive(Debug, Default)]
                    #[cfg_attr(
                        feature = "datagen",
                        derive(databake::Bake),
                        databake(path = icu_properties::provider),
                    )]
                    pub struct $value_long_name_marker_linear4;

                    impl DataMarker for $value_long_name_marker_linear4 {
                        // Tiny4 is only for short names
                        type Yokeable = names::PropertyEnumToValueNameLinearMapV1<'static>;
                    }

                    impl KeyedDataMarker for $value_long_name_marker_linear4 {
                        const KEY: DataKey = data_key!(concat!("propnames/to/long/linear/", $enum_s, "@1"), DataKeyMetadata::construct_internal(FallbackPriority::Language, None, None, true));
                    }
                )?
            )+

        /// All data keys in this module.
        pub const KEYS: &[DataKey] = &[
            $($code_point_set_marker::KEY,)+
            $($unicode_set_marker::KEY,)+
            $(
                $code_point_map_marker::KEY,
                $name_value_marker::KEY,
                $($value_short_name_marker_sparse::KEY, $value_long_name_marker_sparse::KEY,)?
                $($value_short_name_marker_linear::KEY, $value_long_name_marker_linear::KEY,)?
                $($value_short_name_marker_linear4::KEY, $value_long_name_marker_linear4::KEY,)?
            )+
            bidi_data::BidiAuxiliaryPropertiesV1Marker::KEY,
            GeneralCategoryMaskNameToValueV1Marker::KEY,
            ScriptWithExtensionsPropertyV1Marker::KEY,
        ];
    };
}

pub use self::names::GeneralCategoryMaskNameToValueV1Marker;

expand!(
    (
        // code point sets
        (AsciiHexDigitV1Marker, "AHex"),
        (AlnumV1Marker, "alnum"),
        (AlphabeticV1Marker, "Alpha"),
        (BidiControlV1Marker, "Bidi_C"),
        (BidiMirroredV1Marker, "Bidi_M"),
        (BlankV1Marker, "blank"),
        (CasedV1Marker, "Cased"),
        (CaseIgnorableV1Marker, "CI"),
        (FullCompositionExclusionV1Marker, "Comp_Ex"),
        (ChangesWhenCasefoldedV1Marker, "CWCF"),
        (ChangesWhenCasemappedV1Marker, "CWCM"),
        (ChangesWhenNfkcCasefoldedV1Marker, "CWKCF"),
        (ChangesWhenLowercasedV1Marker, "CWL"),
        (ChangesWhenTitlecasedV1Marker, "CWT"),
        (ChangesWhenUppercasedV1Marker, "CWU"),
        (DashV1Marker, "Dash"),
        (DeprecatedV1Marker, "Dep"),
        (DefaultIgnorableCodePointV1Marker, "DI"),
        (DiacriticV1Marker, "Dia"),
        (EmojiModifierBaseV1Marker, "EBase"),
        (EmojiComponentV1Marker, "EComp"),
        (EmojiModifierV1Marker, "EMod"),
        (EmojiV1Marker, "Emoji"),
        (EmojiPresentationV1Marker, "EPres"),
        (ExtenderV1Marker, "Ext"),
        (ExtendedPictographicV1Marker, "ExtPict"),
        (GraphV1Marker, "graph"),
        (GraphemeBaseV1Marker, "Gr_Base"),
        (GraphemeExtendV1Marker, "Gr_Ext"),
        (GraphemeLinkV1Marker, "Gr_Link"),
        (HexDigitV1Marker, "Hex"),
        (HyphenV1Marker, "Hyphen"),
        (IdContinueV1Marker, "IDC"),
        (IdeographicV1Marker, "Ideo"),
        (IdStartV1Marker, "IDS"),
        (IdsBinaryOperatorV1Marker, "IDSB"),
        (IdsTrinaryOperatorV1Marker, "IDST"),
        (JoinControlV1Marker, "Join_C"),
        (LogicalOrderExceptionV1Marker, "LOE"),
        (LowercaseV1Marker, "Lower"),
        (MathV1Marker, "Math"),
        (NoncharacterCodePointV1Marker, "NChar"),
        (NfcInertV1Marker, "nfcinert"),
        (NfdInertV1Marker, "nfdinert"),
        (NfkcInertV1Marker, "nfkcinert"),
        (NfkdInertV1Marker, "nfkdinert"),
        (PatternSyntaxV1Marker, "Pat_Syn"),
        (PatternWhiteSpaceV1Marker, "Pat_WS"),
        (PrependedConcatenationMarkV1Marker, "PCM"),
        (PrintV1Marker, "print"),
        (QuotationMarkV1Marker, "QMark"),
        (RadicalV1Marker, "Radical"),
        (RegionalIndicatorV1Marker, "RI"),
        (SoftDottedV1Marker, "SD"),
        (SegmentStarterV1Marker, "segstart"),
        (CaseSensitiveV1Marker, "Sensitive"),
        (SentenceTerminalV1Marker, "STerm"),
        (TerminalPunctuationV1Marker, "Term"),
        (UnifiedIdeographV1Marker, "UIdeo"),
        (UppercaseV1Marker, "Upper"),
        (VariationSelectorV1Marker, "VS"),
        (WhiteSpaceV1Marker, "WSpace"),
        (XdigitV1Marker, "xdigit"),
        (XidContinueV1Marker, "XIDC"),
        (XidStartV1Marker, "XIDS"),
    ),
    (
        // UnicodeSets (code points + strings)
        (BasicEmojiV1Marker, "Basic_Emoji", true),
        (ExemplarCharactersMainV1Marker, "exemplarchars/main", false),
        (
            ExemplarCharactersAuxiliaryV1Marker,
            "exemplarchars/auxiliary",
            false
        ),
        (
            ExemplarCharactersPunctuationV1Marker,
            "exemplarchars/punctuation",
            false
        ),
        (
            ExemplarCharactersNumbersV1Marker,
            "exemplarchars/numbers",
            false
        ),
        (
            ExemplarCharactersIndexV1Marker,
            "exemplarchars/index",
            false
        ),
    ),
    (
        // code point maps
        (
            CanonicalCombiningClassV1Marker,
            CanonicalCombiningClassNameToValueV1Marker,
            (
                sparse: CanonicalCombiningClassValueToShortNameV1Marker,
                CanonicalCombiningClassValueToLongNameV1Marker
            ),
            "ccc",
            CanonicalCombiningClass
        ),
        (
            GeneralCategoryV1Marker,
            GeneralCategoryNameToValueV1Marker,
            (
                linear: GeneralCategoryValueToShortNameV1Marker,
                GeneralCategoryValueToLongNameV1Marker
            ),
            "gc",
            GeneralCategory
        ),
        (
            BidiClassV1Marker,
            BidiClassNameToValueV1Marker,
            (
                linear: BidiClassValueToShortNameV1Marker,
                BidiClassValueToLongNameV1Marker
            ),
            "bc",
            BidiClass
        ),
        (
            ScriptV1Marker,
            ScriptNameToValueV1Marker,
            (
                linear4: ScriptValueToShortNameV1Marker,
                ScriptValueToLongNameV1Marker
            ),
            "sc",
            Script
        ),
        (
            HangulSyllableTypeV1Marker,
            HangulSyllableTypeNameToValueV1Marker,
            (
                linear: HangulSyllableTypeValueToShortNameV1Marker,
                HangulSyllableTypeValueToLongNameV1Marker
            ),
            "hst",
            HangulSyllableType
        ),
        (
            EastAsianWidthV1Marker,
            EastAsianWidthNameToValueV1Marker,
            (
                linear: EastAsianWidthValueToShortNameV1Marker,
                EastAsianWidthValueToLongNameV1Marker
            ),
            "ea",
            EastAsianWidth
        ),
        (
            LineBreakV1Marker,
            LineBreakNameToValueV1Marker,
            (
                linear: LineBreakValueToShortNameV1Marker,
                LineBreakValueToLongNameV1Marker
            ),
            "lb",
            LineBreak
        ),
        (
            GraphemeClusterBreakV1Marker,
            GraphemeClusterBreakNameToValueV1Marker,
            (
                linear: GraphemeClusterBreakValueToShortNameV1Marker,
                GraphemeClusterBreakValueToLongNameV1Marker
            ),
            "GCB",
            GraphemeClusterBreak
        ),
        (
            WordBreakV1Marker,
            WordBreakNameToValueV1Marker,
            (
                linear: WordBreakValueToShortNameV1Marker,
                WordBreakValueToLongNameV1Marker
            ),
            "WB",
            WordBreak
        ),
        (
            SentenceBreakV1Marker,
            SentenceBreakNameToValueV1Marker,
            (
                linear: SentenceBreakValueToShortNameV1Marker,
                SentenceBreakValueToLongNameV1Marker
            ),
            "SB",
            SentenceBreak
        ),
        (
            IndicSyllabicCategoryV1Marker,
            IndicSyllabicCategoryNameToValueV1Marker,
            (
                linear: IndicSyllabicCategoryValueToShortNameV1Marker,
                IndicSyllabicCategoryValueToLongNameV1Marker
            ),
            "InSC",
            IndicSyllabicCategory
        ),
        (
            JoiningTypeV1Marker,
            JoiningTypeNameToValueV1Marker,
            (
                linear: JoiningTypeValueToShortNameV1Marker,
                JoiningTypeValueToLongNameV1Marker
            ),
            "jt",
            JoiningType
        ),
        // note: the names key for the GCM mask is handled above
    )
);
