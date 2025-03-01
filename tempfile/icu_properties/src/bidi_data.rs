// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! Data and APIs for supporting specific Bidi properties data in an efficient structure.
//!
//! Supported properties are:
//! - `Bidi_Paired_Bracket`
//! - `Bidi_Paired_Bracket_Type`
//! - `Bidi_Mirrored`
//! - `Bidi_Mirroring_Glyph`

use crate::provider::bidi_data::{
    BidiAuxiliaryPropertiesV1, BidiAuxiliaryPropertiesV1Marker, CheckedBidiPairedBracketType,
};
use crate::PropertiesError;

use icu_provider::prelude::*;

/// A wrapper around certain Bidi properties data. Can be obtained via [`bidi_auxiliary_properties()`] and
/// related getters.
///
/// Most useful methods are on [`BidiAuxiliaryPropertiesBorrowed`] obtained by calling [`BidiAuxiliaryProperties::as_borrowed()`]
#[derive(Debug)]
pub struct BidiAuxiliaryProperties {
    data: DataPayload<BidiAuxiliaryPropertiesV1Marker>,
}

impl BidiAuxiliaryProperties {
    /// Construct a borrowed version of this type that can be queried.
    ///
    /// This avoids a potential small underlying cost per API call by consolidating it
    /// up front.
    #[inline]
    pub fn as_borrowed(&self) -> BidiAuxiliaryPropertiesBorrowed<'_> {
        BidiAuxiliaryPropertiesBorrowed {
            data: self.data.get(),
        }
    }

    /// Construct a new one from loaded data
    ///
    /// Typically it is preferable to use getters like [`bidi_auxiliary_properties()`] instead
    pub fn from_data(data: DataPayload<BidiAuxiliaryPropertiesV1Marker>) -> Self {
        Self { data }
    }
}

/// This struct represents the properties Bidi_Mirrored and Bidi_Mirroring_Glyph.
/// If Bidi_Mirroring_Glyph is not defined for a code point, then the value in the
/// struct is `None`.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BidiMirroringProperties {
    /// Represents the Bidi_Mirroring_Glyph property value
    pub mirroring_glyph: Option<char>,
    /// Represents the Bidi_Mirrored property value
    pub mirrored: bool,
}

/// The enum represents Bidi_Paired_Bracket_Type, the char represents Bidi_Paired_Bracket.
/// Bidi_Paired_Bracket has a value of `None` when Bidi_Paired_Bracket_Type is `None`.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BidiPairingProperties {
    /// Represents Bidi_Paired_Bracket_Type=Open, and the Bidi_Paired_Bracket value for that code point.
    Open(char),
    /// Represents Bidi_Paired_Bracket_Type=Close, and the Bidi_Paired_Bracket value for that code point.
    Close(char),
    /// Represents Bidi_Paired_Bracket_Type=None, which cooccurs with Bidi_Paired_Bracket
    /// being undefined for that code point.
    None,
}

/// A borrowed wrapper around Bidi properties data, returned by
/// [`BidiAuxiliaryProperties::as_borrowed()`]. More efficient to query.
#[derive(Debug)]
pub struct BidiAuxiliaryPropertiesBorrowed<'a> {
    data: &'a BidiAuxiliaryPropertiesV1<'a>,
}

impl<'a> BidiAuxiliaryPropertiesBorrowed<'a> {
    // The source data coming from icuexportdata will use 0 to represent the
    // property value in cases for which the Bidi_Mirroring_Glyph property value
    // of a code point is undefined. Since Rust types can be more expressive, we
    // should represent these cases as None.
    fn convert_mirroring_glyph_data(trie_data_char: char) -> Option<char> {
        if trie_data_char as u32 == 0 {
            None
        } else {
            Some(trie_data_char)
        }
    }

    /// Return a struct for the given code point representing Bidi mirroring-related
    /// property values. See [`BidiMirroringProperties`].
    ///
    /// # Examples
    /// ```
    /// use icu::properties::bidi_data;
    ///
    /// let bidi_data = bidi_data::bidi_auxiliary_properties();
    ///
    /// let open_paren = bidi_data.get32_mirroring_props('(' as u32);
    /// assert_eq!(open_paren.mirroring_glyph, Some(')'));
    /// assert_eq!(open_paren.mirrored, true);
    /// let close_paren = bidi_data.get32_mirroring_props(')' as u32);
    /// assert_eq!(close_paren.mirroring_glyph, Some('('));
    /// assert_eq!(close_paren.mirrored, true);
    /// let open_angle_bracket = bidi_data.get32_mirroring_props('<' as u32);
    /// assert_eq!(open_angle_bracket.mirroring_glyph, Some('>'));
    /// assert_eq!(open_angle_bracket.mirrored, true);
    /// let close_angle_bracket = bidi_data.get32_mirroring_props('>' as u32);
    /// assert_eq!(close_angle_bracket.mirroring_glyph, Some('<'));
    /// assert_eq!(close_angle_bracket.mirrored, true);
    /// let three = bidi_data.get32_mirroring_props('3' as u32);
    /// assert_eq!(three.mirroring_glyph, None);
    /// assert_eq!(three.mirrored, false);
    /// ```
    pub fn get32_mirroring_props(&self, code_point: u32) -> BidiMirroringProperties {
        let bidi_aux_props = self.data.trie.get32(code_point);
        let mirroring_glyph_opt =
            Self::convert_mirroring_glyph_data(bidi_aux_props.mirroring_glyph);
        BidiMirroringProperties {
            mirroring_glyph: mirroring_glyph_opt,
            mirrored: bidi_aux_props.mirrored,
        }
    }

    /// Return a struct for the given code point representing Bidi bracket
    /// pairing-related property values. See [`BidiPairingProperties`]
    ///
    /// # Examples
    /// ```
    /// use icu::properties::{bidi_data, bidi_data::BidiPairingProperties};
    ///
    /// let bidi_data = bidi_data::bidi_auxiliary_properties();
    ///
    /// let open_paren = bidi_data.get32_pairing_props('(' as u32);
    /// assert_eq!(open_paren, BidiPairingProperties::Open(')'));
    /// let close_paren = bidi_data.get32_pairing_props(')' as u32);
    /// assert_eq!(close_paren, BidiPairingProperties::Close('('));
    /// let open_angle_bracket = bidi_data.get32_pairing_props('<' as u32);
    /// assert_eq!(open_angle_bracket, BidiPairingProperties::None);
    /// let close_angle_bracket = bidi_data.get32_pairing_props('>' as u32);
    /// assert_eq!(close_angle_bracket, BidiPairingProperties::None);
    /// let three = bidi_data.get32_pairing_props('3' as u32);
    /// assert_eq!(three, BidiPairingProperties::None);
    /// ```
    pub fn get32_pairing_props(&self, code_point: u32) -> BidiPairingProperties {
        let bidi_aux_props = self.data.trie.get32(code_point);
        let mirroring_glyph = bidi_aux_props.mirroring_glyph;
        let paired_bracket_type = bidi_aux_props.paired_bracket_type;
        match paired_bracket_type {
            CheckedBidiPairedBracketType::Open => BidiPairingProperties::Open(mirroring_glyph),
            CheckedBidiPairedBracketType::Close => BidiPairingProperties::Close(mirroring_glyph),
            _ => BidiPairingProperties::None,
        }
    }
}

impl BidiAuxiliaryPropertiesBorrowed<'static> {
    /// Cheaply converts a [`BidiAuxiliaryPropertiesBorrowed<'static>`] into a [`BidiAuxiliaryProperties`].
    ///
    /// Note: Due to branching and indirection, using [`BidiAuxiliaryProperties`] might inhibit some
    /// compile-time optimizations that are possible with [`BidiAuxiliaryPropertiesBorrowed`].
    pub const fn static_to_owned(self) -> BidiAuxiliaryProperties {
        BidiAuxiliaryProperties {
            data: DataPayload::from_static_ref(self.data),
        }
    }
}

/// Creates a [`BidiAuxiliaryPropertiesV1`] struct that represents the data for certain
/// Bidi properties.
///
/// ✨ *Enabled with the `compiled_data` Cargo feature.*
///
/// [📚 Help choosing a constructor](icu_provider::constructors)
///
/// # Examples
/// ```
/// use icu::properties::bidi_data;
///
/// let bidi_data = bidi_data::bidi_auxiliary_properties();
///
/// let open_paren = bidi_data.get32_mirroring_props('(' as u32);
/// assert_eq!(open_paren.mirroring_glyph, Some(')'));
/// assert_eq!(open_paren.mirrored, true);
/// ```
#[cfg(feature = "compiled_data")]
pub const fn bidi_auxiliary_properties() -> BidiAuxiliaryPropertiesBorrowed<'static> {
    BidiAuxiliaryPropertiesBorrowed {
        data: crate::provider::Baked::SINGLETON_PROPS_BIDIAUXILIARYPROPS_V1,
    }
}

icu_provider::gen_any_buffer_data_constructors!(
    locale: skip,
    options: skip,
    result: Result<BidiAuxiliaryProperties, PropertiesError>,
    #[cfg(skip)]
    functions: [
        bidi_auxiliary_properties,
        load_bidi_auxiliary_properties_with_any_provider,
        load_bidi_auxiliary_properties_with_buffer_provider,
        load_bidi_auxiliary_properties_unstable,
    ]
);

#[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, bidi_auxiliary_properties)]
pub fn load_bidi_auxiliary_properties_unstable(
    provider: &(impl DataProvider<BidiAuxiliaryPropertiesV1Marker> + ?Sized),
) -> Result<BidiAuxiliaryProperties, PropertiesError> {
    Ok(provider
        .load(Default::default())
        .and_then(DataResponse::take_payload)
        .map(BidiAuxiliaryProperties::from_data)?)
}
