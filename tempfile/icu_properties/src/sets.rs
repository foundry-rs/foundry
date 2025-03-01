// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! The functions in this module return a [`CodePointSetData`] containing
//! the set of characters with a particular Unicode property.
//!
//! The descriptions of most properties are taken from [`TR44`], the documentation for the
//! Unicode Character Database.  Some properties are instead defined in [`TR18`], the
//! documentation for Unicode regular expressions. In particular, Annex C of this document
//! defines properties for POSIX compatibility.
//!
//! [`CodePointSetData`]: crate::sets::CodePointSetData
//! [`TR44`]: https://www.unicode.org/reports/tr44
//! [`TR18`]: https://www.unicode.org/reports/tr18

use crate::error::PropertiesError;
use crate::provider::*;
use crate::*;
use core::iter::FromIterator;
use core::ops::RangeInclusive;
use icu_collections::codepointinvlist::CodePointInversionList;
use icu_collections::codepointinvliststringlist::CodePointInversionListAndStringList;
use icu_provider::prelude::*;

//
// CodePointSet* structs, impls, & macros
// (a set with only code points)
//

/// A wrapper around code point set data. It is returned by APIs that return Unicode
/// property data in a set-like form, ex: a set of code points sharing the same
/// value for a Unicode property. Access its data via the borrowed version,
/// [`CodePointSetDataBorrowed`].
#[derive(Debug)]
pub struct CodePointSetData {
    data: DataPayload<ErasedSetlikeMarker>,
}

/// Private marker type for CodePointSetData
/// to work for all set properties at once
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ErasedSetlikeMarker;
impl DataMarker for ErasedSetlikeMarker {
    type Yokeable = PropertyCodePointSetV1<'static>;
}

impl CodePointSetData {
    /// Construct a borrowed version of this type that can be queried.
    ///
    /// This owned version if returned by functions that use a runtime data provider.
    #[inline]
    pub fn as_borrowed(&self) -> CodePointSetDataBorrowed<'_> {
        CodePointSetDataBorrowed {
            set: self.data.get(),
        }
    }

    /// Construct a new one from loaded data
    ///
    /// Typically it is preferable to use getters like [`load_ascii_hex_digit()`] instead
    pub fn from_data<M>(data: DataPayload<M>) -> Self
    where
        M: DataMarker<Yokeable = PropertyCodePointSetV1<'static>>,
    {
        Self { data: data.cast() }
    }

    /// Construct a new owned [`CodePointInversionList`]
    pub fn from_code_point_inversion_list(set: CodePointInversionList<'static>) -> Self {
        let set = PropertyCodePointSetV1::from_code_point_inversion_list(set);
        CodePointSetData::from_data(DataPayload::<ErasedSetlikeMarker>::from_owned(set))
    }

    /// Convert this type to a [`CodePointInversionList`] as a borrowed value.
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointInversionList`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// This method returns an `Option` in order to return `None` when the backing data provider
    /// cannot return a [`CodePointInversionList`], or cannot do so within the expected constant time
    /// constraint.
    pub fn as_code_point_inversion_list(&self) -> Option<&CodePointInversionList<'_>> {
        self.data.get().as_code_point_inversion_list()
    }

    /// Convert this type to a [`CodePointInversionList`], borrowing if possible,
    /// otherwise allocating a new [`CodePointInversionList`].
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointInversionList`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// The performance of the conversion to this specific return type will vary
    /// depending on the data structure that is backing `self`.
    pub fn to_code_point_inversion_list(&self) -> CodePointInversionList<'_> {
        self.data.get().to_code_point_inversion_list()
    }
}

/// A borrowed wrapper around code point set data, returned by
/// [`CodePointSetData::as_borrowed()`]. More efficient to query.
#[derive(Clone, Copy, Debug)]
pub struct CodePointSetDataBorrowed<'a> {
    set: &'a PropertyCodePointSetV1<'a>,
}

impl CodePointSetDataBorrowed<'static> {
    /// Cheaply converts a [`CodePointSetDataBorrowed<'static>`] into a [`CodePointSetData`].
    ///
    /// Note: Due to branching and indirection, using [`CodePointSetData`] might inhibit some
    /// compile-time optimizations that are possible with [`CodePointSetDataBorrowed`].
    pub const fn static_to_owned(self) -> CodePointSetData {
        CodePointSetData {
            data: DataPayload::from_static_ref(self.set),
        }
    }
}

impl<'a> CodePointSetDataBorrowed<'a> {
    /// Check if the set contains a character
    ///
    /// ```rust
    /// use icu::properties::sets;
    ///
    /// let alphabetic = sets::alphabetic();
    ///
    /// assert!(!alphabetic.contains('3'));
    /// assert!(!alphabetic.contains('੩'));  // U+0A69 GURMUKHI DIGIT THREE
    /// assert!(alphabetic.contains('A'));
    /// assert!(alphabetic.contains('Ä'));  // U+00C4 LATIN CAPITAL LETTER A WITH DIAERESIS
    /// ```
    #[inline]
    pub fn contains(self, ch: char) -> bool {
        self.set.contains(ch)
    }

    /// Check if the set contains a character as a UTF32 code unit
    ///
    /// ```rust
    /// use icu::properties::sets;
    ///
    /// let alphabetic = sets::alphabetic();
    ///
    /// assert!(!alphabetic.contains32(0x0A69));  // U+0A69 GURMUKHI DIGIT THREE
    /// assert!(alphabetic.contains32(0x00C4));  // U+00C4 LATIN CAPITAL LETTER A WITH DIAERESIS
    /// ```
    #[inline]
    pub fn contains32(self, ch: u32) -> bool {
        self.set.contains32(ch)
    }

    // Yields an [`Iterator`] returning the ranges of the code points that are
    /// included in the [`CodePointSetData`]
    ///
    /// Ranges are returned as [`RangeInclusive`], which is inclusive of its
    /// `end` bound value. An end-inclusive behavior matches the ICU4C/J
    /// behavior of ranges, ex: `UnicodeSet::contains(UChar32 start, UChar32 end)`.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let alphabetic = sets::alphabetic();
    /// let mut ranges = alphabetic.iter_ranges();
    ///
    /// assert_eq!(Some(0x0041..=0x005A), ranges.next()); // 'A'..'Z'
    /// assert_eq!(Some(0x0061..=0x007A), ranges.next()); // 'a'..'z'
    /// ```
    #[inline]
    pub fn iter_ranges(self) -> impl Iterator<Item = RangeInclusive<u32>> + 'a {
        self.set.iter_ranges()
    }

    // Yields an [`Iterator`] returning the ranges of the code points that are
    /// *not* included in the [`CodePointSetData`]
    ///
    /// Ranges are returned as [`RangeInclusive`], which is inclusive of its
    /// `end` bound value. An end-inclusive behavior matches the ICU4C/J
    /// behavior of ranges, ex: `UnicodeSet::contains(UChar32 start, UChar32 end)`.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let alphabetic = sets::alphabetic();
    /// let mut ranges = alphabetic.iter_ranges();
    ///
    /// assert_eq!(Some(0x0041..=0x005A), ranges.next()); // 'A'..'Z'
    /// assert_eq!(Some(0x0061..=0x007A), ranges.next()); // 'a'..'z'
    /// ```
    #[inline]
    pub fn iter_ranges_complemented(self) -> impl Iterator<Item = RangeInclusive<u32>> + 'a {
        self.set.iter_ranges_complemented()
    }
}

//
// UnicodeSet* structs, impls, & macros
// (a set with code points + strings)
//

/// A wrapper around `UnicodeSet` data (characters and strings)
#[derive(Debug)]
pub struct UnicodeSetData {
    data: DataPayload<ErasedUnicodeSetlikeMarker>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ErasedUnicodeSetlikeMarker;
impl DataMarker for ErasedUnicodeSetlikeMarker {
    type Yokeable = PropertyUnicodeSetV1<'static>;
}

impl UnicodeSetData {
    /// Construct a borrowed version of this type that can be queried.
    ///
    /// This avoids a potential small underlying cost per API call (ex: `contains()`) by consolidating it
    /// up front.
    #[inline]
    pub fn as_borrowed(&self) -> UnicodeSetDataBorrowed<'_> {
        UnicodeSetDataBorrowed {
            set: self.data.get(),
        }
    }

    /// Construct a new one from loaded data
    ///
    /// Typically it is preferable to use getters instead
    pub fn from_data<M>(data: DataPayload<M>) -> Self
    where
        M: DataMarker<Yokeable = PropertyUnicodeSetV1<'static>>,
    {
        Self { data: data.cast() }
    }

    /// Construct a new owned [`CodePointInversionListAndStringList`]
    pub fn from_code_point_inversion_list_string_list(
        set: CodePointInversionListAndStringList<'static>,
    ) -> Self {
        let set = PropertyUnicodeSetV1::from_code_point_inversion_list_string_list(set);
        UnicodeSetData::from_data(DataPayload::<ErasedUnicodeSetlikeMarker>::from_owned(set))
    }

    /// Convert this type to a [`CodePointInversionListAndStringList`] as a borrowed value.
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointInversionListAndStringList`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// This method returns an `Option` in order to return `None` when the backing data provider
    /// cannot return a [`CodePointInversionListAndStringList`], or cannot do so within the expected constant time
    /// constraint.
    pub fn as_code_point_inversion_list_string_list(
        &self,
    ) -> Option<&CodePointInversionListAndStringList<'_>> {
        self.data.get().as_code_point_inversion_list_string_list()
    }

    /// Convert this type to a [`CodePointInversionListAndStringList`], borrowing if possible,
    /// otherwise allocating a new [`CodePointInversionListAndStringList`].
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointInversionListAndStringList`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// The performance of the conversion to this specific return type will vary
    /// depending on the data structure that is backing `self`.
    pub fn to_code_point_inversion_list_string_list(
        &self,
    ) -> CodePointInversionListAndStringList<'_> {
        self.data.get().to_code_point_inversion_list_string_list()
    }
}

/// A borrowed wrapper around code point set data, returned by
/// [`UnicodeSetData::as_borrowed()`]. More efficient to query.
#[derive(Clone, Copy, Debug)]
pub struct UnicodeSetDataBorrowed<'a> {
    set: &'a PropertyUnicodeSetV1<'a>,
}

impl<'a> UnicodeSetDataBorrowed<'a> {
    /// Check if the set contains the string. Strings consisting of one character
    /// are treated as a character/code point.
    ///
    /// This matches ICU behavior for ICU's `UnicodeSet`.
    #[inline]
    pub fn contains(self, s: &str) -> bool {
        self.set.contains(s)
    }

    /// Check if the set contains a character as a UTF32 code unit
    #[inline]
    pub fn contains32(&self, cp: u32) -> bool {
        self.set.contains32(cp)
    }

    /// Check if the set contains the code point corresponding to the Rust character.
    #[inline]
    pub fn contains_char(&self, ch: char) -> bool {
        self.set.contains_char(ch)
    }
}

impl UnicodeSetDataBorrowed<'static> {
    /// Cheaply converts a [`UnicodeSetDataBorrowed<'static>`] into a [`UnicodeSetData`].
    ///
    /// Note: Due to branching and indirection, using [`UnicodeSetData`] might inhibit some
    /// compile-time optimizations that are possible with [`UnicodeSetDataBorrowed`].
    pub const fn static_to_owned(self) -> UnicodeSetData {
        UnicodeSetData {
            data: DataPayload::from_static_ref(self.set),
        }
    }
}

pub(crate) fn load_set_data<M, P>(provider: &P) -> Result<CodePointSetData, PropertiesError>
where
    M: KeyedDataMarker<Yokeable = PropertyCodePointSetV1<'static>>,
    P: DataProvider<M> + ?Sized,
{
    Ok(provider
        .load(Default::default())
        .and_then(DataResponse::take_payload)
        .map(CodePointSetData::from_data)?)
}

//
// Binary property getter fns
// (data as code point sets)
//

macro_rules! make_code_point_set_property {
    (
        // currently unused
        property: $property:expr;
        // currently unused
        marker: $marker_name:ident;
        keyed_data_marker: $keyed_data_marker:ty;
        func:
        $(#[$doc:meta])+
        $cvis:vis const fn $constname:ident() => $singleton_name:ident;
        $vis:vis fn $funcname:ident();
    ) => {
        #[doc = concat!("A version of [`", stringify!($constname), "()`] that uses custom data provided by a [`DataProvider`].")]
        ///
        /// Note that this will return an owned version of the data. Functionality is available on
        /// the borrowed version, accessible through [`CodePointSetData::as_borrowed`].
        $vis fn $funcname(
            provider: &(impl DataProvider<$keyed_data_marker> + ?Sized)
        ) -> Result<CodePointSetData, PropertiesError> {
            load_set_data(provider)
        }

        $(#[$doc])*
        #[cfg(feature = "compiled_data")]
        $cvis const fn $constname() -> CodePointSetDataBorrowed<'static> {
            CodePointSetDataBorrowed {
                set: crate::provider::Baked::$singleton_name,
            }
        }
    }
}

make_code_point_set_property! {
    property: "ASCII_Hex_Digit";
    marker: AsciiHexDigitProperty;
    keyed_data_marker: AsciiHexDigitV1Marker;
    func:
    /// ASCII characters commonly used for the representation of hexadecimal numbers
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let ascii_hex_digit = sets::ascii_hex_digit();
    ///
    /// assert!(ascii_hex_digit.contains('3'));
    /// assert!(!ascii_hex_digit.contains('੩'));  // U+0A69 GURMUKHI DIGIT THREE
    /// assert!(ascii_hex_digit.contains('A'));
    /// assert!(!ascii_hex_digit.contains('Ä'));  // U+00C4 LATIN CAPITAL LETTER A WITH DIAERESIS
    /// ```
    pub const fn ascii_hex_digit() => SINGLETON_PROPS_AHEX_V1;
    pub fn load_ascii_hex_digit();
}

make_code_point_set_property! {
    property: "Alnum";
    marker: AlnumProperty;
    keyed_data_marker: AlnumV1Marker;
    func:
    /// Characters with the Alphabetic or Decimal_Number property
    /// This is defined for POSIX compatibility.

    pub const fn alnum() => SINGLETON_PROPS_ALNUM_V1;
    pub fn load_alnum();
}

make_code_point_set_property! {
    property: "Alphabetic";
    marker: AlphabeticProperty;
    keyed_data_marker: AlphabeticV1Marker;
    func:
    /// Alphabetic characters
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let alphabetic = sets::alphabetic();
    ///
    /// assert!(!alphabetic.contains('3'));
    /// assert!(!alphabetic.contains('੩'));  // U+0A69 GURMUKHI DIGIT THREE
    /// assert!(alphabetic.contains('A'));
    /// assert!(alphabetic.contains('Ä'));  // U+00C4 LATIN CAPITAL LETTER A WITH DIAERESIS
    /// ```

    pub const fn alphabetic() => SINGLETON_PROPS_ALPHA_V1;
    pub fn load_alphabetic();
}

make_code_point_set_property! {
    property: "Bidi_Control";
    marker: BidiControlProperty;
    keyed_data_marker: BidiControlV1Marker;
    func:
    /// Format control characters which have specific functions in the Unicode Bidirectional
    /// Algorithm
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let bidi_control = sets::bidi_control();
    ///
    /// assert!(bidi_control.contains32(0x200F));  // RIGHT-TO-LEFT MARK
    /// assert!(!bidi_control.contains('ش'));  // U+0634 ARABIC LETTER SHEEN
    /// ```

    pub const fn bidi_control() => SINGLETON_PROPS_BIDI_C_V1;
    pub fn load_bidi_control();
}

make_code_point_set_property! {
    property: "Bidi_Mirrored";
    marker: BidiMirroredProperty;
    keyed_data_marker: BidiMirroredV1Marker;
    func:
    /// Characters that are mirrored in bidirectional text
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let bidi_mirrored = sets::bidi_mirrored();
    ///
    /// assert!(bidi_mirrored.contains('['));
    /// assert!(bidi_mirrored.contains(']'));
    /// assert!(bidi_mirrored.contains('∑'));  // U+2211 N-ARY SUMMATION
    /// assert!(!bidi_mirrored.contains('ཉ'));  // U+0F49 TIBETAN LETTER NYA
    /// ```

    pub const fn bidi_mirrored() => SINGLETON_PROPS_BIDI_M_V1;
    pub fn load_bidi_mirrored();
}

make_code_point_set_property! {
    property: "Blank";
    marker: BlankProperty;
    keyed_data_marker: BlankV1Marker;
    func:
    /// Horizontal whitespace characters

    pub const fn blank() => SINGLETON_PROPS_BLANK_V1;
    pub fn load_blank();
}

make_code_point_set_property! {
    property: "Cased";
    marker: CasedProperty;
    keyed_data_marker: CasedV1Marker;
    func:
    /// Uppercase, lowercase, and titlecase characters
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let cased = sets::cased();
    ///
    /// assert!(cased.contains('Ꙡ'));  // U+A660 CYRILLIC CAPITAL LETTER REVERSED TSE
    /// assert!(!cased.contains('ދ'));  // U+078B THAANA LETTER DHAALU
    /// ```

    pub const fn cased() => SINGLETON_PROPS_CASED_V1;
    pub fn load_cased();
}

make_code_point_set_property! {
    property: "Case_Ignorable";
    marker: CaseIgnorableProperty;
    keyed_data_marker: CaseIgnorableV1Marker;
    func:
    /// Characters which are ignored for casing purposes
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let case_ignorable = sets::case_ignorable();
    ///
    /// assert!(case_ignorable.contains(':'));
    /// assert!(!case_ignorable.contains('λ'));  // U+03BB GREEK SMALL LETTER LAMDA
    /// ```

    pub const fn case_ignorable() => SINGLETON_PROPS_CI_V1;
    pub fn load_case_ignorable();
}

make_code_point_set_property! {
    property: "Full_Composition_Exclusion";
    marker: FullCompositionExclusionProperty;
    keyed_data_marker: FullCompositionExclusionV1Marker;
    func:
    /// Characters that are excluded from composition
    /// See <https://unicode.org/Public/UNIDATA/CompositionExclusions.txt>

    pub const fn full_composition_exclusion() => SINGLETON_PROPS_COMP_EX_V1;
    pub fn load_full_composition_exclusion();
}

make_code_point_set_property! {
    property: "Changes_When_Casefolded";
    marker: ChangesWhenCasefoldedProperty;
    keyed_data_marker: ChangesWhenCasefoldedV1Marker;
    func:
    /// Characters whose normalized forms are not stable under case folding
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let changes_when_casefolded = sets::changes_when_casefolded();
    ///
    /// assert!(changes_when_casefolded.contains('ß'));  // U+00DF LATIN SMALL LETTER SHARP S
    /// assert!(!changes_when_casefolded.contains('ᜉ'));  // U+1709 TAGALOG LETTER PA
    /// ```

    pub const fn changes_when_casefolded() => SINGLETON_PROPS_CWCF_V1;
    pub fn load_changes_when_casefolded();
}

make_code_point_set_property! {
    property: "Changes_When_Casemapped";
    marker: ChangesWhenCasemappedProperty;
    keyed_data_marker: ChangesWhenCasemappedV1Marker;
    func:
    /// Characters which may change when they undergo case mapping

    pub const fn changes_when_casemapped() => SINGLETON_PROPS_CWCM_V1;
    pub fn load_changes_when_casemapped();
}

make_code_point_set_property! {
    property: "Changes_When_NFKC_Casefolded";
    marker: ChangesWhenNfkcCasefoldedProperty;
    keyed_data_marker: ChangesWhenNfkcCasefoldedV1Marker;
    func:
    /// Characters which are not identical to their NFKC_Casefold mapping
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let changes_when_nfkc_casefolded = sets::changes_when_nfkc_casefolded();
    ///
    /// assert!(changes_when_nfkc_casefolded.contains('🄵'));  // U+1F135 SQUARED LATIN CAPITAL LETTER F
    /// assert!(!changes_when_nfkc_casefolded.contains('f'));
    /// ```

    pub const fn changes_when_nfkc_casefolded() => SINGLETON_PROPS_CWKCF_V1;
    pub fn load_changes_when_nfkc_casefolded();
}

make_code_point_set_property! {
    property: "Changes_When_Lowercased";
    marker: ChangesWhenLowercasedProperty;
    keyed_data_marker: ChangesWhenLowercasedV1Marker;
    func:
    /// Characters whose normalized forms are not stable under a toLowercase mapping
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let changes_when_lowercased = sets::changes_when_lowercased();
    ///
    /// assert!(changes_when_lowercased.contains('Ⴔ'));  // U+10B4 GEORGIAN CAPITAL LETTER PHAR
    /// assert!(!changes_when_lowercased.contains('ფ'));  // U+10E4 GEORGIAN LETTER PHAR
    /// ```

    pub const fn changes_when_lowercased() => SINGLETON_PROPS_CWL_V1;
    pub fn load_changes_when_lowercased();
}

make_code_point_set_property! {
    property: "Changes_When_Titlecased";
    marker: ChangesWhenTitlecasedProperty;
    keyed_data_marker: ChangesWhenTitlecasedV1Marker;
    func:
    /// Characters whose normalized forms are not stable under a toTitlecase mapping
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let changes_when_titlecased = sets::changes_when_titlecased();
    ///
    /// assert!(changes_when_titlecased.contains('æ'));  // U+00E6 LATIN SMALL LETTER AE
    /// assert!(!changes_when_titlecased.contains('Æ'));  // U+00E6 LATIN CAPITAL LETTER AE
    /// ```

    pub const fn changes_when_titlecased() => SINGLETON_PROPS_CWT_V1;
    pub fn load_changes_when_titlecased();
}

make_code_point_set_property! {
    property: "Changes_When_Uppercased";
    marker: ChangesWhenUppercasedProperty;
    keyed_data_marker: ChangesWhenUppercasedV1Marker;
    func:
    /// Characters whose normalized forms are not stable under a toUppercase mapping
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let changes_when_uppercased = sets::changes_when_uppercased();
    ///
    /// assert!(changes_when_uppercased.contains('ւ'));  // U+0582 ARMENIAN SMALL LETTER YIWN
    /// assert!(!changes_when_uppercased.contains('Ւ'));  // U+0552 ARMENIAN CAPITAL LETTER YIWN
    /// ```

    pub const fn changes_when_uppercased() => SINGLETON_PROPS_CWU_V1;
    pub fn load_changes_when_uppercased();
}

make_code_point_set_property! {
    property: "Dash";
    marker: DashProperty;
    keyed_data_marker: DashV1Marker;
    func:
    /// Punctuation characters explicitly called out as dashes in the Unicode Standard, plus
    /// their compatibility equivalents
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let dash = sets::dash();
    ///
    /// assert!(dash.contains('⸺'));  // U+2E3A TWO-EM DASH
    /// assert!(dash.contains('-'));  // U+002D
    /// assert!(!dash.contains('='));  // U+003D
    /// ```

    pub const fn dash() => SINGLETON_PROPS_DASH_V1;
    pub fn load_dash();
}

make_code_point_set_property! {
    property: "Deprecated";
    marker: DeprecatedProperty;
    keyed_data_marker: DeprecatedV1Marker;
    func:
    /// Deprecated characters. No characters will ever be removed from the standard, but the
    /// usage of deprecated characters is strongly discouraged.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let deprecated = sets::deprecated();
    ///
    /// assert!(deprecated.contains('ឣ'));  // U+17A3 KHMER INDEPENDENT VOWEL QAQ
    /// assert!(!deprecated.contains('A'));
    /// ```

    pub const fn deprecated() => SINGLETON_PROPS_DEP_V1;
    pub fn load_deprecated();
}

make_code_point_set_property! {
    property: "Default_Ignorable_Code_Point";
    marker: DefaultIgnorableCodePointProperty;
    keyed_data_marker: DefaultIgnorableCodePointV1Marker;
    func:
    /// For programmatic determination of default ignorable code points.  New characters that
    /// should be ignored in rendering (unless explicitly supported) will be assigned in these
    /// ranges, permitting programs to correctly handle the default rendering of such
    /// characters when not otherwise supported.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let default_ignorable_code_point = sets::default_ignorable_code_point();
    ///
    /// assert!(default_ignorable_code_point.contains32(0x180B));  // MONGOLIAN FREE VARIATION SELECTOR ONE
    /// assert!(!default_ignorable_code_point.contains('E'));
    /// ```

    pub const fn default_ignorable_code_point() => SINGLETON_PROPS_DI_V1;
    pub fn load_default_ignorable_code_point();
}

make_code_point_set_property! {
    property: "Diacritic";
    marker: DiacriticProperty;
    keyed_data_marker: DiacriticV1Marker;
    func:
    /// Characters that linguistically modify the meaning of another character to which they apply
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let diacritic = sets::diacritic();
    ///
    /// assert!(diacritic.contains('\u{05B3}'));  // HEBREW POINT HATAF QAMATS
    /// assert!(!diacritic.contains('א'));  // U+05D0 HEBREW LETTER ALEF
    /// ```

    pub const fn diacritic() => SINGLETON_PROPS_DIA_V1;
    pub fn load_diacritic();
}

make_code_point_set_property! {
    property: "Emoji_Modifier_Base";
    marker: EmojiModifierBaseProperty;
    keyed_data_marker: EmojiModifierBaseV1Marker;
    func:
    /// Characters that can serve as a base for emoji modifiers
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let emoji_modifier_base = sets::emoji_modifier_base();
    ///
    /// assert!(emoji_modifier_base.contains('✊'));  // U+270A RAISED FIST
    /// assert!(!emoji_modifier_base.contains('⛰'));  // U+26F0 MOUNTAIN
    /// ```

    pub const fn emoji_modifier_base() => SINGLETON_PROPS_EBASE_V1;
    pub fn load_emoji_modifier_base();
}

make_code_point_set_property! {
    property: "Emoji_Component";
    marker: EmojiComponentProperty;
    keyed_data_marker: EmojiComponentV1Marker;
    func:
    /// Characters used in emoji sequences that normally do not appear on emoji keyboards as
    /// separate choices, such as base characters for emoji keycaps
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let emoji_component = sets::emoji_component();
    ///
    /// assert!(emoji_component.contains('🇹'));  // U+1F1F9 REGIONAL INDICATOR SYMBOL LETTER T
    /// assert!(emoji_component.contains32(0x20E3));  // COMBINING ENCLOSING KEYCAP
    /// assert!(emoji_component.contains('7'));
    /// assert!(!emoji_component.contains('T'));
    /// ```

    pub const fn emoji_component() => SINGLETON_PROPS_ECOMP_V1;
    pub fn load_emoji_component();
}

make_code_point_set_property! {
    property: "Emoji_Modifier";
    marker: EmojiModifierProperty;
    keyed_data_marker: EmojiModifierV1Marker;
    func:
    /// Characters that are emoji modifiers
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let emoji_modifier = sets::emoji_modifier();
    ///
    /// assert!(emoji_modifier.contains32(0x1F3FD));  // EMOJI MODIFIER FITZPATRICK TYPE-4
    /// assert!(!emoji_modifier.contains32(0x200C));  // ZERO WIDTH NON-JOINER
    /// ```

    pub const fn emoji_modifier() => SINGLETON_PROPS_EMOD_V1;
    pub fn load_emoji_modifier();
}

make_code_point_set_property! {
    property: "Emoji";
    marker: EmojiProperty;
    keyed_data_marker: EmojiV1Marker;
    func:
    /// Characters that are emoji
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let emoji = sets::emoji();
    ///
    /// assert!(emoji.contains('🔥'));  // U+1F525 FIRE
    /// assert!(!emoji.contains('V'));
    /// ```

    pub const fn emoji() => SINGLETON_PROPS_EMOJI_V1;
    pub fn load_emoji();
}

make_code_point_set_property! {
    property: "Emoji_Presentation";
    marker: EmojiPresentationProperty;
    keyed_data_marker: EmojiPresentationV1Marker;
    func:
    /// Characters that have emoji presentation by default
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let emoji_presentation = sets::emoji_presentation();
    ///
    /// assert!(emoji_presentation.contains('🦬')); // U+1F9AC BISON
    /// assert!(!emoji_presentation.contains('♻'));  // U+267B BLACK UNIVERSAL RECYCLING SYMBOL
    /// ```

    pub const fn emoji_presentation() => SINGLETON_PROPS_EPRES_V1;
    pub fn load_emoji_presentation();
}

make_code_point_set_property! {
    property: "Extender";
    marker: ExtenderProperty;
    keyed_data_marker: ExtenderV1Marker;
    func:
    /// Characters whose principal function is to extend the value of a preceding alphabetic
    /// character or to extend the shape of adjacent characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let extender = sets::extender();
    ///
    /// assert!(extender.contains('ヾ'));  // U+30FE KATAKANA VOICED ITERATION MARK
    /// assert!(extender.contains('ー'));  // U+30FC KATAKANA-HIRAGANA PROLONGED SOUND MARK
    /// assert!(!extender.contains('・'));  // U+30FB KATAKANA MIDDLE DOT
    /// ```

    pub const fn extender() => SINGLETON_PROPS_EXT_V1;
    pub fn load_extender();
}

make_code_point_set_property! {
    property: "Extended_Pictographic";
    marker: ExtendedPictographicProperty;
    keyed_data_marker: ExtendedPictographicV1Marker;
    func:
    /// Pictographic symbols, as well as reserved ranges in blocks largely associated with
    /// emoji characters
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let extended_pictographic = sets::extended_pictographic();
    ///
    /// assert!(extended_pictographic.contains('🥳')); // U+1F973 FACE WITH PARTY HORN AND PARTY HAT
    /// assert!(!extended_pictographic.contains('🇪'));  // U+1F1EA REGIONAL INDICATOR SYMBOL LETTER E
    /// ```

    pub const fn extended_pictographic() => SINGLETON_PROPS_EXTPICT_V1;
    pub fn load_extended_pictographic();
}

make_code_point_set_property! {
    property: "Graph";
    marker: GraphProperty;
    keyed_data_marker: GraphV1Marker;
    func:
    /// Visible characters.
    /// This is defined for POSIX compatibility.

    pub const fn graph() => SINGLETON_PROPS_GRAPH_V1;
    pub fn load_graph();
}

make_code_point_set_property! {
    property: "Grapheme_Base";
    marker: GraphemeBaseProperty;
    keyed_data_marker: GraphemeBaseV1Marker;
    func:
    /// Property used together with the definition of Standard Korean Syllable Block to define
    /// "Grapheme base". See D58 in Chapter 3, Conformance in the Unicode Standard.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let grapheme_base = sets::grapheme_base();
    ///
    /// assert!(grapheme_base.contains('ക'));  // U+0D15 MALAYALAM LETTER KA
    /// assert!(grapheme_base.contains('\u{0D3F}'));  // U+0D3F MALAYALAM VOWEL SIGN I
    /// assert!(!grapheme_base.contains('\u{0D3E}'));  // U+0D3E MALAYALAM VOWEL SIGN AA
    /// ```

    pub const fn grapheme_base() => SINGLETON_PROPS_GR_BASE_V1;
    pub fn load_grapheme_base();
}

make_code_point_set_property! {
    property: "Grapheme_Extend";
    marker: GraphemeExtendProperty;
    keyed_data_marker: GraphemeExtendV1Marker;
    func:
    /// Property used to define "Grapheme extender". See D59 in Chapter 3, Conformance in the
    /// Unicode Standard.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let grapheme_extend = sets::grapheme_extend();
    ///
    /// assert!(!grapheme_extend.contains('ക'));  // U+0D15 MALAYALAM LETTER KA
    /// assert!(!grapheme_extend.contains('\u{0D3F}'));  // U+0D3F MALAYALAM VOWEL SIGN I
    /// assert!(grapheme_extend.contains('\u{0D3E}'));  // U+0D3E MALAYALAM VOWEL SIGN AA
    /// ```

    pub const fn grapheme_extend() => SINGLETON_PROPS_GR_EXT_V1;
    pub fn load_grapheme_extend();
}

make_code_point_set_property! {
    property: "Grapheme_Link";
    marker: GraphemeLinkProperty;
    keyed_data_marker: GraphemeLinkV1Marker;
    func:
    /// Deprecated property. Formerly proposed for programmatic determination of grapheme
    /// cluster boundaries.

    pub const fn grapheme_link() => SINGLETON_PROPS_GR_LINK_V1;
    pub fn load_grapheme_link();
}

make_code_point_set_property! {
    property: "Hex_Digit";
    marker: HexDigitProperty;
    keyed_data_marker: HexDigitV1Marker;
    func:
    /// Characters commonly used for the representation of hexadecimal numbers, plus their
    /// compatibility equivalents
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let hex_digit = sets::hex_digit();
    ///
    /// assert!(hex_digit.contains('0'));
    /// assert!(!hex_digit.contains('੩'));  // U+0A69 GURMUKHI DIGIT THREE
    /// assert!(hex_digit.contains('f'));
    /// assert!(hex_digit.contains('ｆ'));  // U+FF46 FULLWIDTH LATIN SMALL LETTER F
    /// assert!(hex_digit.contains('Ｆ'));  // U+FF26 FULLWIDTH LATIN CAPITAL LETTER F
    /// assert!(!hex_digit.contains('Ä'));  // U+00C4 LATIN CAPITAL LETTER A WITH DIAERESIS
    /// ```

    pub const fn hex_digit() => SINGLETON_PROPS_HEX_V1;
    pub fn load_hex_digit();
}

make_code_point_set_property! {
    property: "Hyphen";
    marker: HyphenProperty;
    keyed_data_marker: HyphenV1Marker;
    func:
    /// Deprecated property. Dashes which are used to mark connections between pieces of
    /// words, plus the Katakana middle dot.

    pub const fn hyphen() => SINGLETON_PROPS_HYPHEN_V1;
    pub fn load_hyphen();
}

make_code_point_set_property! {
    property: "Id_Continue";
    marker: IdContinueProperty;
    keyed_data_marker: IdContinueV1Marker;
    func:
    /// Characters that can come after the first character in an identifier. If using NFKC to
    /// fold differences between characters, use [`load_xid_continue`] instead.  See
    /// [`Unicode Standard Annex #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for
    /// more details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let id_continue = sets::id_continue();
    ///
    /// assert!(id_continue.contains('x'));
    /// assert!(id_continue.contains('1'));
    /// assert!(id_continue.contains('_'));
    /// assert!(id_continue.contains('ߝ'));  // U+07DD NKO LETTER FA
    /// assert!(!id_continue.contains('ⓧ'));  // U+24E7 CIRCLED LATIN SMALL LETTER X
    /// assert!(id_continue.contains32(0xFC5E));  // ARABIC LIGATURE SHADDA WITH DAMMATAN ISOLATED FORM
    /// ```

    pub const fn id_continue() => SINGLETON_PROPS_IDC_V1;
    pub fn load_id_continue();
}

make_code_point_set_property! {
    property: "Ideographic";
    marker: IdeographicProperty;
    keyed_data_marker: IdeographicV1Marker;
    func:
    /// Characters considered to be CJKV (Chinese, Japanese, Korean, and Vietnamese)
    /// ideographs, or related siniform ideographs
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let ideographic = sets::ideographic();
    ///
    /// assert!(ideographic.contains('川'));  // U+5DDD CJK UNIFIED IDEOGRAPH-5DDD
    /// assert!(!ideographic.contains('밥'));  // U+BC25 HANGUL SYLLABLE BAB
    /// ```

    pub const fn ideographic() => SINGLETON_PROPS_IDEO_V1;
    pub fn load_ideographic();
}

make_code_point_set_property! {
    property: "Id_Start";
    marker: IdStartProperty;
    keyed_data_marker: IdStartV1Marker;
    func:
    /// Characters that can begin an identifier. If using NFKC to fold differences between
    /// characters, use [`load_xid_start`] instead.  See [`Unicode Standard Annex
    /// #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for more details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let id_start = sets::id_start();
    ///
    /// assert!(id_start.contains('x'));
    /// assert!(!id_start.contains('1'));
    /// assert!(!id_start.contains('_'));
    /// assert!(id_start.contains('ߝ'));  // U+07DD NKO LETTER FA
    /// assert!(!id_start.contains('ⓧ'));  // U+24E7 CIRCLED LATIN SMALL LETTER X
    /// assert!(id_start.contains32(0xFC5E));  // ARABIC LIGATURE SHADDA WITH DAMMATAN ISOLATED FORM
    /// ```

    pub const fn id_start() => SINGLETON_PROPS_IDS_V1;
    pub fn load_id_start();
}

make_code_point_set_property! {
    property: "Ids_Binary_Operator";
    marker: IdsBinaryOperatorProperty;
    keyed_data_marker: IdsBinaryOperatorV1Marker;
    func:
    /// Characters used in Ideographic Description Sequences
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let ids_binary_operator = sets::ids_binary_operator();
    ///
    /// assert!(ids_binary_operator.contains32(0x2FF5));  // IDEOGRAPHIC DESCRIPTION CHARACTER SURROUND FROM ABOVE
    /// assert!(!ids_binary_operator.contains32(0x3006));  // IDEOGRAPHIC CLOSING MARK
    /// ```

    pub const fn ids_binary_operator() => SINGLETON_PROPS_IDSB_V1;
    pub fn load_ids_binary_operator();
}

make_code_point_set_property! {
    property: "Ids_Trinary_Operator";
    marker: IdsTrinaryOperatorProperty;
    keyed_data_marker: IdsTrinaryOperatorV1Marker;
    func:
    /// Characters used in Ideographic Description Sequences
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let ids_trinary_operator = sets::ids_trinary_operator();
    ///
    /// assert!(ids_trinary_operator.contains32(0x2FF2));  // IDEOGRAPHIC DESCRIPTION CHARACTER LEFT TO MIDDLE AND RIGHT
    /// assert!(ids_trinary_operator.contains32(0x2FF3));  // IDEOGRAPHIC DESCRIPTION CHARACTER ABOVE TO MIDDLE AND BELOW
    /// assert!(!ids_trinary_operator.contains32(0x2FF4));
    /// assert!(!ids_trinary_operator.contains32(0x2FF5));  // IDEOGRAPHIC DESCRIPTION CHARACTER SURROUND FROM ABOVE
    /// assert!(!ids_trinary_operator.contains32(0x3006));  // IDEOGRAPHIC CLOSING MARK
    /// ```

    pub const fn ids_trinary_operator() => SINGLETON_PROPS_IDST_V1;
    pub fn load_ids_trinary_operator();
}

make_code_point_set_property! {
    property: "Join_Control";
    marker: JoinControlProperty;
    keyed_data_marker: JoinControlV1Marker;
    func:
    /// Format control characters which have specific functions for control of cursive joining
    /// and ligation
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let join_control = sets::join_control();
    ///
    /// assert!(join_control.contains32(0x200C));  // ZERO WIDTH NON-JOINER
    /// assert!(join_control.contains32(0x200D));  // ZERO WIDTH JOINER
    /// assert!(!join_control.contains32(0x200E));
    /// ```

    pub const fn join_control() => SINGLETON_PROPS_JOIN_C_V1;
    pub fn load_join_control();
}

make_code_point_set_property! {
    property: "Logical_Order_Exception";
    marker: LogicalOrderExceptionProperty;
    keyed_data_marker: LogicalOrderExceptionV1Marker;
    func:
    /// A small number of spacing vowel letters occurring in certain Southeast Asian scripts such as Thai and Lao
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let logical_order_exception = sets::logical_order_exception();
    ///
    /// assert!(logical_order_exception.contains('ແ'));  // U+0EC1 LAO VOWEL SIGN EI
    /// assert!(!logical_order_exception.contains('ະ'));  // U+0EB0 LAO VOWEL SIGN A
    /// ```

    pub const fn logical_order_exception() => SINGLETON_PROPS_LOE_V1;
    pub fn load_logical_order_exception();
}

make_code_point_set_property! {
    property: "Lowercase";
    marker: LowercaseProperty;
    keyed_data_marker: LowercaseV1Marker;
    func:
    /// Lowercase characters
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let lowercase = sets::lowercase();
    ///
    /// assert!(lowercase.contains('a'));
    /// assert!(!lowercase.contains('A'));
    /// ```

    pub const fn lowercase() => SINGLETON_PROPS_LOWER_V1;
    pub fn load_lowercase();
}

make_code_point_set_property! {
    property: "Math";
    marker: MathProperty;
    keyed_data_marker: MathV1Marker;
    func:
    /// Characters used in mathematical notation
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let math = sets::math();
    ///
    /// assert!(math.contains('='));
    /// assert!(math.contains('+'));
    /// assert!(!math.contains('-'));
    /// assert!(math.contains('−'));  // U+2212 MINUS SIGN
    /// assert!(!math.contains('/'));
    /// assert!(math.contains('∕'));  // U+2215 DIVISION SLASH
    /// ```

    pub const fn math() => SINGLETON_PROPS_MATH_V1;
    pub fn load_math();
}

make_code_point_set_property! {
    property: "Noncharacter_Code_Point";
    marker: NoncharacterCodePointProperty;
    keyed_data_marker: NoncharacterCodePointV1Marker;
    func:
    /// Code points permanently reserved for internal use
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let noncharacter_code_point = sets::noncharacter_code_point();
    ///
    /// assert!(noncharacter_code_point.contains32(0xFDD0));
    /// assert!(noncharacter_code_point.contains32(0xFFFF));
    /// assert!(!noncharacter_code_point.contains32(0x10000));
    /// ```

    pub const fn noncharacter_code_point() => SINGLETON_PROPS_NCHAR_V1;
    pub fn load_noncharacter_code_point();
}

make_code_point_set_property! {
    property: "NFC_Inert";
    marker: NfcInertProperty;
    keyed_data_marker: NfcInertV1Marker;
    func:
    /// Characters that are inert under NFC, i.e., they do not interact with adjacent characters

    pub const fn nfc_inert() => SINGLETON_PROPS_NFCINERT_V1;
    pub fn load_nfc_inert();
}

make_code_point_set_property! {
    property: "NFD_Inert";
    marker: NfdInertProperty;
    keyed_data_marker: NfdInertV1Marker;
    func:
    /// Characters that are inert under NFD, i.e., they do not interact with adjacent characters

    pub const fn nfd_inert() => SINGLETON_PROPS_NFDINERT_V1;
    pub fn load_nfd_inert();
}

make_code_point_set_property! {
    property: "NFKC_Inert";
    marker: NfkcInertProperty;
    keyed_data_marker: NfkcInertV1Marker;
    func:
    /// Characters that are inert under NFKC, i.e., they do not interact with adjacent characters

    pub const fn nfkc_inert() => SINGLETON_PROPS_NFKCINERT_V1;
    pub fn load_nfkc_inert();
}

make_code_point_set_property! {
    property: "NFKD_Inert";
    marker: NfkdInertProperty;
    keyed_data_marker: NfkdInertV1Marker;
    func:
    /// Characters that are inert under NFKD, i.e., they do not interact with adjacent characters

    pub const fn nfkd_inert() => SINGLETON_PROPS_NFKDINERT_V1;
    pub fn load_nfkd_inert();
}

make_code_point_set_property! {
    property: "Pattern_Syntax";
    marker: PatternSyntaxProperty;
    keyed_data_marker: PatternSyntaxV1Marker;
    func:
    /// Characters used as syntax in patterns (such as regular expressions). See [`Unicode
    /// Standard Annex #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for more
    /// details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let pattern_syntax = sets::pattern_syntax();
    ///
    /// assert!(pattern_syntax.contains('{'));
    /// assert!(pattern_syntax.contains('⇒'));  // U+21D2 RIGHTWARDS DOUBLE ARROW
    /// assert!(!pattern_syntax.contains('0'));
    /// ```

    pub const fn pattern_syntax() => SINGLETON_PROPS_PAT_SYN_V1;
    pub fn load_pattern_syntax();
}

make_code_point_set_property! {
    property: "Pattern_White_Space";
    marker: PatternWhiteSpaceProperty;
    keyed_data_marker: PatternWhiteSpaceV1Marker;
    func:
    /// Characters used as whitespace in patterns (such as regular expressions).  See
    /// [`Unicode Standard Annex #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for
    /// more details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let pattern_white_space = sets::pattern_white_space();
    ///
    /// assert!(pattern_white_space.contains(' '));
    /// assert!(pattern_white_space.contains32(0x2029));  // PARAGRAPH SEPARATOR
    /// assert!(pattern_white_space.contains32(0x000A));  // NEW LINE
    /// assert!(!pattern_white_space.contains32(0x00A0));  // NO-BREAK SPACE
    /// ```

    pub const fn pattern_white_space() => SINGLETON_PROPS_PAT_WS_V1;
    pub fn load_pattern_white_space();
}

make_code_point_set_property! {
    property: "Prepended_Concatenation_Mark";
    marker: PrependedConcatenationMarkProperty;
    keyed_data_marker: PrependedConcatenationMarkV1Marker;
    func:
    /// A small class of visible format controls, which precede and then span a sequence of
    /// other characters, usually digits.

    pub const fn prepended_concatenation_mark() => SINGLETON_PROPS_PCM_V1;
    pub fn load_prepended_concatenation_mark();
}

make_code_point_set_property! {
    property: "Print";
    marker: PrintProperty;
    keyed_data_marker: PrintV1Marker;
    func:
    /// Printable characters (visible characters and whitespace).
    /// This is defined for POSIX compatibility.

    pub const fn print() => SINGLETON_PROPS_PRINT_V1;
    pub fn load_print();
}

make_code_point_set_property! {
    property: "Quotation_Mark";
    marker: QuotationMarkProperty;
    keyed_data_marker: QuotationMarkV1Marker;
    func:
    /// Punctuation characters that function as quotation marks.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let quotation_mark = sets::quotation_mark();
    ///
    /// assert!(quotation_mark.contains('\''));
    /// assert!(quotation_mark.contains('„'));  // U+201E DOUBLE LOW-9 QUOTATION MARK
    /// assert!(!quotation_mark.contains('<'));
    /// ```

    pub const fn quotation_mark() => SINGLETON_PROPS_QMARK_V1;
    pub fn load_quotation_mark();
}

make_code_point_set_property! {
    property: "Radical";
    marker: RadicalProperty;
    keyed_data_marker: RadicalV1Marker;
    func:
    /// Characters used in the definition of Ideographic Description Sequences
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let radical = sets::radical();
    ///
    /// assert!(radical.contains('⺆'));  // U+2E86 CJK RADICAL BOX
    /// assert!(!radical.contains('丹'));  // U+F95E CJK COMPATIBILITY IDEOGRAPH-F95E
    /// ```

    pub const fn radical() => SINGLETON_PROPS_RADICAL_V1;
    pub fn load_radical();
}

make_code_point_set_property! {
    property: "Regional_Indicator";
    marker: RegionalIndicatorProperty;
    keyed_data_marker: RegionalIndicatorV1Marker;
    func:
    /// Regional indicator characters, U+1F1E6..U+1F1FF
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let regional_indicator = sets::regional_indicator();
    ///
    /// assert!(regional_indicator.contains('🇹'));  // U+1F1F9 REGIONAL INDICATOR SYMBOL LETTER T
    /// assert!(!regional_indicator.contains('Ⓣ'));  // U+24C9 CIRCLED LATIN CAPITAL LETTER T
    /// assert!(!regional_indicator.contains('T'));
    /// ```

    pub const fn regional_indicator() => SINGLETON_PROPS_RI_V1;
    pub fn load_regional_indicator();
}

make_code_point_set_property! {
    property: "Soft_Dotted";
    marker: SoftDottedProperty;
    keyed_data_marker: SoftDottedV1Marker;
    func:
    /// Characters with a "soft dot", like i or j. An accent placed on these characters causes
    /// the dot to disappear.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let soft_dotted = sets::soft_dotted();
    ///
    /// assert!(soft_dotted.contains('і'));  //U+0456 CYRILLIC SMALL LETTER BYELORUSSIAN-UKRAINIAN I
    /// assert!(!soft_dotted.contains('ı'));  // U+0131 LATIN SMALL LETTER DOTLESS I
    /// ```

    pub const fn soft_dotted() => SINGLETON_PROPS_SD_V1;
    pub fn load_soft_dotted();
}

make_code_point_set_property! {
    property: "Segment_Starter";
    marker: SegmentStarterProperty;
    keyed_data_marker: SegmentStarterV1Marker;
    func:
    /// Characters that are starters in terms of Unicode normalization and combining character
    /// sequences

    pub const fn segment_starter() => SINGLETON_PROPS_SEGSTART_V1;
    pub fn load_segment_starter();
}

make_code_point_set_property! {
    property: "Case_Sensitive";
    marker: CaseSensitiveProperty;
    keyed_data_marker: CaseSensitiveV1Marker;
    func:
    /// Characters that are either the source of a case mapping or in the target of a case
    /// mapping

    pub const fn case_sensitive() => SINGLETON_PROPS_SENSITIVE_V1;
    pub fn load_case_sensitive();
}

make_code_point_set_property! {
    property: "Sentence_Terminal";
    marker: SentenceTerminalProperty;
    keyed_data_marker: SentenceTerminalV1Marker;
    func:
    /// Punctuation characters that generally mark the end of sentences
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let sentence_terminal = sets::sentence_terminal();
    ///
    /// assert!(sentence_terminal.contains('.'));
    /// assert!(sentence_terminal.contains('?'));
    /// assert!(sentence_terminal.contains('᪨'));  // U+1AA8 TAI THAM SIGN KAAN
    /// assert!(!sentence_terminal.contains(','));
    /// assert!(!sentence_terminal.contains('¿'));  // U+00BF INVERTED QUESTION MARK
    /// ```

    pub const fn sentence_terminal() => SINGLETON_PROPS_STERM_V1;
    pub fn load_sentence_terminal();
}

make_code_point_set_property! {
    property: "Terminal_Punctuation";
    marker: TerminalPunctuationProperty;
    keyed_data_marker: TerminalPunctuationV1Marker;
    func:
    /// Punctuation characters that generally mark the end of textual units
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let terminal_punctuation = sets::terminal_punctuation();
    ///
    /// assert!(terminal_punctuation.contains('.'));
    /// assert!(terminal_punctuation.contains('?'));
    /// assert!(terminal_punctuation.contains('᪨'));  // U+1AA8 TAI THAM SIGN KAAN
    /// assert!(terminal_punctuation.contains(','));
    /// assert!(!terminal_punctuation.contains('¿'));  // U+00BF INVERTED QUESTION MARK
    /// ```

    pub const fn terminal_punctuation() => SINGLETON_PROPS_TERM_V1;
    pub fn load_terminal_punctuation();
}

make_code_point_set_property! {
    property: "Unified_Ideograph";
    marker: UnifiedIdeographProperty;
    keyed_data_marker: UnifiedIdeographV1Marker;
    func:
    /// A property which specifies the exact set of Unified CJK Ideographs in the standard
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let unified_ideograph = sets::unified_ideograph();
    ///
    /// assert!(unified_ideograph.contains('川'));  // U+5DDD CJK UNIFIED IDEOGRAPH-5DDD
    /// assert!(unified_ideograph.contains('木'));  // U+6728 CJK UNIFIED IDEOGRAPH-6728
    /// assert!(!unified_ideograph.contains('𛅸'));  // U+1B178 NUSHU CHARACTER-1B178
    /// ```

    pub const fn unified_ideograph() => SINGLETON_PROPS_UIDEO_V1;
    pub fn load_unified_ideograph();
}

make_code_point_set_property! {
    property: "Uppercase";
    marker: UppercaseProperty;
    keyed_data_marker: UppercaseV1Marker;
    func:
    /// Uppercase characters
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let uppercase = sets::uppercase();
    ///
    /// assert!(uppercase.contains('U'));
    /// assert!(!uppercase.contains('u'));
    /// ```

    pub const fn uppercase() => SINGLETON_PROPS_UPPER_V1;
    pub fn load_uppercase();
}

make_code_point_set_property! {
    property: "Variation_Selector";
    marker: VariationSelectorProperty;
    keyed_data_marker: VariationSelectorV1Marker;
    func:
    /// Characters that are Variation Selectors.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let variation_selector = sets::variation_selector();
    ///
    /// assert!(variation_selector.contains32(0x180D));  // MONGOLIAN FREE VARIATION SELECTOR THREE
    /// assert!(!variation_selector.contains32(0x303E));  // IDEOGRAPHIC VARIATION INDICATOR
    /// assert!(variation_selector.contains32(0xFE0F));  // VARIATION SELECTOR-16
    /// assert!(!variation_selector.contains32(0xFE10));  // PRESENTATION FORM FOR VERTICAL COMMA
    /// assert!(variation_selector.contains32(0xE01EF));  // VARIATION SELECTOR-256
    /// ```

    pub const fn variation_selector() => SINGLETON_PROPS_VS_V1;
    pub fn load_variation_selector();
}

make_code_point_set_property! {
    property: "White_Space";
    marker: WhiteSpaceProperty;
    keyed_data_marker: WhiteSpaceV1Marker;
    func:
    /// Spaces, separator characters and other control characters which should be treated by
    /// programming languages as "white space" for the purpose of parsing elements
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let white_space = sets::white_space();
    ///
    /// assert!(white_space.contains(' '));
    /// assert!(white_space.contains32(0x000A));  // NEW LINE
    /// assert!(white_space.contains32(0x00A0));  // NO-BREAK SPACE
    /// assert!(!white_space.contains32(0x200B));  // ZERO WIDTH SPACE
    /// ```

    pub const fn white_space() => SINGLETON_PROPS_WSPACE_V1;
    pub fn load_white_space();
}

make_code_point_set_property! {
    property: "Xdigit";
    marker: XdigitProperty;
    keyed_data_marker: XdigitV1Marker;
    func:
    /// Hexadecimal digits
    /// This is defined for POSIX compatibility.

    pub const fn xdigit() => SINGLETON_PROPS_XDIGIT_V1;
    pub fn load_xdigit();
}

make_code_point_set_property! {
    property: "XID_Continue";
    marker: XidContinueProperty;
    keyed_data_marker: XidContinueV1Marker;
    func:
    /// Characters that can come after the first character in an identifier.  See [`Unicode Standard Annex
    /// #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for more details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let xid_continue = sets::xid_continue();
    ///
    /// assert!(xid_continue.contains('x'));
    /// assert!(xid_continue.contains('1'));
    /// assert!(xid_continue.contains('_'));
    /// assert!(xid_continue.contains('ߝ'));  // U+07DD NKO LETTER FA
    /// assert!(!xid_continue.contains('ⓧ'));  // U+24E7 CIRCLED LATIN SMALL LETTER X
    /// assert!(!xid_continue.contains32(0xFC5E));  // ARABIC LIGATURE SHADDA WITH DAMMATAN ISOLATED FORM
    /// ```

    pub const fn xid_continue() => SINGLETON_PROPS_XIDC_V1;
    pub fn load_xid_continue();
}

make_code_point_set_property! {
    property: "XID_Start";
    marker: XidStartProperty;
    keyed_data_marker: XidStartV1Marker;
    func:
    /// Characters that can begin an identifier. See [`Unicode
    /// Standard Annex #31`](https://www.unicode.org/reports/tr31/tr31-35.html) for more
    /// details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let xid_start = sets::xid_start();
    ///
    /// assert!(xid_start.contains('x'));
    /// assert!(!xid_start.contains('1'));
    /// assert!(!xid_start.contains('_'));
    /// assert!(xid_start.contains('ߝ'));  // U+07DD NKO LETTER FA
    /// assert!(!xid_start.contains('ⓧ'));  // U+24E7 CIRCLED LATIN SMALL LETTER X
    /// assert!(!xid_start.contains32(0xFC5E));  // ARABIC LIGATURE SHADDA WITH DAMMATAN ISOLATED FORM
    /// ```

    pub const fn xid_start() => SINGLETON_PROPS_XIDS_V1;
    pub fn load_xid_start();
}

//
// Binary property getter fns
// (data as sets of strings + code points)
//

macro_rules! make_unicode_set_property {
    (
        // currently unused
        property: $property:expr;
        // currently unused
        marker: $marker_name:ident;
        keyed_data_marker: $keyed_data_marker:ty;
        func:
        $(#[$doc:meta])+
        $cvis:vis const fn $constname:ident() => $singleton:ident;
        $vis:vis fn $funcname:ident();
    ) => {
        #[doc = concat!("A version of [`", stringify!($constname), "()`] that uses custom data provided by a [`DataProvider`].")]
        $vis fn $funcname(
            provider: &(impl DataProvider<$keyed_data_marker> + ?Sized)
        ) -> Result<UnicodeSetData, PropertiesError> {
            Ok(provider.load(Default::default()).and_then(DataResponse::take_payload).map(UnicodeSetData::from_data)?)
        }
        $(#[$doc])*
        #[cfg(feature = "compiled_data")]
        $cvis const fn $constname() -> UnicodeSetDataBorrowed<'static> {
            UnicodeSetDataBorrowed {
                set: crate::provider::Baked::$singleton
            }
        }
    }
}

make_unicode_set_property! {
    property: "Basic_Emoji";
    marker: BasicEmojiProperty;
    keyed_data_marker: BasicEmojiV1Marker;
    func:
    /// Characters and character sequences intended for general-purpose, independent, direct input.
    /// See [`Unicode Technical Standard #51`](https://unicode.org/reports/tr51/) for more
    /// details.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::sets;
    ///
    /// let basic_emoji = sets::basic_emoji();
    ///
    /// assert!(!basic_emoji.contains32(0x0020));
    /// assert!(!basic_emoji.contains_char('\n'));
    /// assert!(basic_emoji.contains_char('🦃')); // U+1F983 TURKEY
    /// assert!(basic_emoji.contains("\u{1F983}"));
    /// assert!(basic_emoji.contains("\u{1F6E4}\u{FE0F}")); // railway track
    /// assert!(!basic_emoji.contains("\u{0033}\u{FE0F}\u{20E3}"));  // Emoji_Keycap_Sequence, keycap 3
    /// ```
    pub const fn basic_emoji() => SINGLETON_PROPS_BASIC_EMOJI_V1;
    pub fn load_basic_emoji();
}

//
// Enumerated property getter fns
//

/// A version of [`for_general_category_group()`] that uses custom data provided by a [`DataProvider`].
///
/// [📚 Help choosing a constructor](icu_provider::constructors)
pub fn load_for_general_category_group(
    provider: &(impl DataProvider<GeneralCategoryV1Marker> + ?Sized),
    enum_val: GeneralCategoryGroup,
) -> Result<CodePointSetData, PropertiesError> {
    let gc_map_payload = maps::load_general_category(provider)?;
    let gc_map = gc_map_payload.as_borrowed();
    let matching_gc_ranges = gc_map
        .iter_ranges()
        .filter(|cpm_range| (1 << cpm_range.value as u32) & enum_val.0 != 0)
        .map(|cpm_range| cpm_range.range);
    let set = CodePointInversionList::from_iter(matching_gc_ranges);
    Ok(CodePointSetData::from_code_point_inversion_list(set))
}

/// Return a [`CodePointSetData`] for a value or a grouping of values of the General_Category property. See [`GeneralCategoryGroup`].
///
/// ✨ *Enabled with the `compiled_data` Cargo feature.*
///
/// [📚 Help choosing a constructor](icu_provider::constructors)
#[cfg(feature = "compiled_data")]
pub fn for_general_category_group(enum_val: GeneralCategoryGroup) -> CodePointSetData {
    let matching_gc_ranges = maps::general_category()
        .iter_ranges()
        .filter(|cpm_range| (1 << cpm_range.value as u32) & enum_val.0 != 0)
        .map(|cpm_range| cpm_range.range);
    let set = CodePointInversionList::from_iter(matching_gc_ranges);
    CodePointSetData::from_code_point_inversion_list(set)
}

/// Returns a type capable of looking up values for a property specified as a string, as long as it is a
/// [binary property listed in ECMA-262][ecma], using strict matching on the names in the spec.
///
/// This handles every property required by ECMA-262 `/u` regular expressions, except for:
///
/// - `Script` and `General_Category`: handle these directly with [`maps::load_general_category()`] and
///    [`maps::load_script()`].
///    using property values parsed via [`GeneralCategory::get_name_to_enum_mapper()`] and [`Script::get_name_to_enum_mapper()`]
///    if necessary.
/// - `Script_Extensions`: handle this directly using APIs from [`crate::script`], like [`script::load_script_with_extensions_unstable()`]
/// - `General_Category` mask values: Handle this alongside `General_Category` using [`GeneralCategoryGroup`],
///    using property values parsed via [`GeneralCategoryGroup::get_name_to_enum_mapper()`] if necessary
/// - `Assigned`, `All`, and `ASCII` pseudoproperties: Handle these using their equivalent sets:
///    - `Any` can be expressed as the range `[\u{0}-\u{10FFFF}]`
///    - `Assigned` can be expressed as the inverse of the set `gc=Cn` (i.e., `\P{gc=Cn}`).
///    - `ASCII` can be expressed as the range `[\u{0}-\u{7F}]`
/// - `General_Category` property values can themselves be treated like properties using a shorthand in ECMA262,
///    simply create the corresponding `GeneralCategory` set.
///
/// ✨ *Enabled with the `compiled_data` Cargo feature.*
///
/// [📚 Help choosing a constructor](icu_provider::constructors)
///
/// ```
/// use icu::properties::sets;
///
/// let emoji = sets::load_for_ecma262("Emoji").expect("loading data failed");
///
/// assert!(emoji.contains('🔥')); // U+1F525 FIRE
/// assert!(!emoji.contains('V'));
/// ```
///
/// [ecma]: https://tc39.es/ecma262/#table-binary-unicode-properties
#[cfg(feature = "compiled_data")]
pub fn load_for_ecma262(name: &str) -> Result<CodePointSetDataBorrowed<'static>, PropertiesError> {
    use crate::runtime::UnicodeProperty;

    let prop = if let Some(prop) = UnicodeProperty::parse_ecma262_name(name) {
        prop
    } else {
        return Err(PropertiesError::UnexpectedPropertyName);
    };
    Ok(match prop {
        UnicodeProperty::AsciiHexDigit => ascii_hex_digit(),
        UnicodeProperty::Alphabetic => alphabetic(),
        UnicodeProperty::BidiControl => bidi_control(),
        UnicodeProperty::BidiMirrored => bidi_mirrored(),
        UnicodeProperty::CaseIgnorable => case_ignorable(),
        UnicodeProperty::Cased => cased(),
        UnicodeProperty::ChangesWhenCasefolded => changes_when_casefolded(),
        UnicodeProperty::ChangesWhenCasemapped => changes_when_casemapped(),
        UnicodeProperty::ChangesWhenLowercased => changes_when_lowercased(),
        UnicodeProperty::ChangesWhenNfkcCasefolded => changes_when_nfkc_casefolded(),
        UnicodeProperty::ChangesWhenTitlecased => changes_when_titlecased(),
        UnicodeProperty::ChangesWhenUppercased => changes_when_uppercased(),
        UnicodeProperty::Dash => dash(),
        UnicodeProperty::DefaultIgnorableCodePoint => default_ignorable_code_point(),
        UnicodeProperty::Deprecated => deprecated(),
        UnicodeProperty::Diacritic => diacritic(),
        UnicodeProperty::Emoji => emoji(),
        UnicodeProperty::EmojiComponent => emoji_component(),
        UnicodeProperty::EmojiModifier => emoji_modifier(),
        UnicodeProperty::EmojiModifierBase => emoji_modifier_base(),
        UnicodeProperty::EmojiPresentation => emoji_presentation(),
        UnicodeProperty::ExtendedPictographic => extended_pictographic(),
        UnicodeProperty::Extender => extender(),
        UnicodeProperty::GraphemeBase => grapheme_base(),
        UnicodeProperty::GraphemeExtend => grapheme_extend(),
        UnicodeProperty::HexDigit => hex_digit(),
        UnicodeProperty::IdsBinaryOperator => ids_binary_operator(),
        UnicodeProperty::IdsTrinaryOperator => ids_trinary_operator(),
        UnicodeProperty::IdContinue => id_continue(),
        UnicodeProperty::IdStart => id_start(),
        UnicodeProperty::Ideographic => ideographic(),
        UnicodeProperty::JoinControl => join_control(),
        UnicodeProperty::LogicalOrderException => logical_order_exception(),
        UnicodeProperty::Lowercase => lowercase(),
        UnicodeProperty::Math => math(),
        UnicodeProperty::NoncharacterCodePoint => noncharacter_code_point(),
        UnicodeProperty::PatternSyntax => pattern_syntax(),
        UnicodeProperty::PatternWhiteSpace => pattern_white_space(),
        UnicodeProperty::QuotationMark => quotation_mark(),
        UnicodeProperty::Radical => radical(),
        UnicodeProperty::RegionalIndicator => regional_indicator(),
        UnicodeProperty::SentenceTerminal => sentence_terminal(),
        UnicodeProperty::SoftDotted => soft_dotted(),
        UnicodeProperty::TerminalPunctuation => terminal_punctuation(),
        UnicodeProperty::UnifiedIdeograph => unified_ideograph(),
        UnicodeProperty::Uppercase => uppercase(),
        UnicodeProperty::VariationSelector => variation_selector(),
        UnicodeProperty::WhiteSpace => white_space(),
        UnicodeProperty::XidContinue => xid_continue(),
        UnicodeProperty::XidStart => xid_start(),
        _ => return Err(PropertiesError::UnexpectedPropertyName),
    })
}

icu_provider::gen_any_buffer_data_constructors!(
    locale: skip,
    name: &str,
    result: Result<CodePointSetData, PropertiesError>,
    #[cfg(skip)]
    functions: [
        load_for_ecma262,
        load_for_ecma262_with_any_provider,
        load_for_ecma262_with_buffer_provider,
        load_for_ecma262_unstable,
    ]
);

#[doc = icu_provider::gen_any_buffer_unstable_docs!(UNSTABLE, load_for_ecma262)]
pub fn load_for_ecma262_unstable<P>(
    provider: &P,
    name: &str,
) -> Result<CodePointSetData, PropertiesError>
where
    P: ?Sized
        + DataProvider<AsciiHexDigitV1Marker>
        + DataProvider<AlphabeticV1Marker>
        + DataProvider<BidiControlV1Marker>
        + DataProvider<BidiMirroredV1Marker>
        + DataProvider<CaseIgnorableV1Marker>
        + DataProvider<CasedV1Marker>
        + DataProvider<ChangesWhenCasefoldedV1Marker>
        + DataProvider<ChangesWhenCasemappedV1Marker>
        + DataProvider<ChangesWhenLowercasedV1Marker>
        + DataProvider<ChangesWhenNfkcCasefoldedV1Marker>
        + DataProvider<ChangesWhenTitlecasedV1Marker>
        + DataProvider<ChangesWhenUppercasedV1Marker>
        + DataProvider<DashV1Marker>
        + DataProvider<DefaultIgnorableCodePointV1Marker>
        + DataProvider<DeprecatedV1Marker>
        + DataProvider<DiacriticV1Marker>
        + DataProvider<EmojiV1Marker>
        + DataProvider<EmojiComponentV1Marker>
        + DataProvider<EmojiModifierV1Marker>
        + DataProvider<EmojiModifierBaseV1Marker>
        + DataProvider<EmojiPresentationV1Marker>
        + DataProvider<ExtendedPictographicV1Marker>
        + DataProvider<ExtenderV1Marker>
        + DataProvider<GraphemeBaseV1Marker>
        + DataProvider<GraphemeExtendV1Marker>
        + DataProvider<HexDigitV1Marker>
        + DataProvider<IdsBinaryOperatorV1Marker>
        + DataProvider<IdsTrinaryOperatorV1Marker>
        + DataProvider<IdContinueV1Marker>
        + DataProvider<IdStartV1Marker>
        + DataProvider<IdeographicV1Marker>
        + DataProvider<JoinControlV1Marker>
        + DataProvider<LogicalOrderExceptionV1Marker>
        + DataProvider<LowercaseV1Marker>
        + DataProvider<MathV1Marker>
        + DataProvider<NoncharacterCodePointV1Marker>
        + DataProvider<PatternSyntaxV1Marker>
        + DataProvider<PatternWhiteSpaceV1Marker>
        + DataProvider<QuotationMarkV1Marker>
        + DataProvider<RadicalV1Marker>
        + DataProvider<RegionalIndicatorV1Marker>
        + DataProvider<SentenceTerminalV1Marker>
        + DataProvider<SoftDottedV1Marker>
        + DataProvider<TerminalPunctuationV1Marker>
        + DataProvider<UnifiedIdeographV1Marker>
        + DataProvider<UppercaseV1Marker>
        + DataProvider<VariationSelectorV1Marker>
        + DataProvider<WhiteSpaceV1Marker>
        + DataProvider<XidContinueV1Marker>
        + DataProvider<XidStartV1Marker>,
{
    use crate::runtime::UnicodeProperty;

    let prop = if let Some(prop) = UnicodeProperty::parse_ecma262_name(name) {
        prop
    } else {
        return Err(PropertiesError::UnexpectedPropertyName);
    };
    match prop {
        UnicodeProperty::AsciiHexDigit => load_ascii_hex_digit(provider),
        UnicodeProperty::Alphabetic => load_alphabetic(provider),
        UnicodeProperty::BidiControl => load_bidi_control(provider),
        UnicodeProperty::BidiMirrored => load_bidi_mirrored(provider),
        UnicodeProperty::CaseIgnorable => load_case_ignorable(provider),
        UnicodeProperty::Cased => load_cased(provider),
        UnicodeProperty::ChangesWhenCasefolded => load_changes_when_casefolded(provider),
        UnicodeProperty::ChangesWhenCasemapped => load_changes_when_casemapped(provider),
        UnicodeProperty::ChangesWhenLowercased => load_changes_when_lowercased(provider),
        UnicodeProperty::ChangesWhenNfkcCasefolded => load_changes_when_nfkc_casefolded(provider),
        UnicodeProperty::ChangesWhenTitlecased => load_changes_when_titlecased(provider),
        UnicodeProperty::ChangesWhenUppercased => load_changes_when_uppercased(provider),
        UnicodeProperty::Dash => load_dash(provider),
        UnicodeProperty::DefaultIgnorableCodePoint => load_default_ignorable_code_point(provider),
        UnicodeProperty::Deprecated => load_deprecated(provider),
        UnicodeProperty::Diacritic => load_diacritic(provider),
        UnicodeProperty::Emoji => load_emoji(provider),
        UnicodeProperty::EmojiComponent => load_emoji_component(provider),
        UnicodeProperty::EmojiModifier => load_emoji_modifier(provider),
        UnicodeProperty::EmojiModifierBase => load_emoji_modifier_base(provider),
        UnicodeProperty::EmojiPresentation => load_emoji_presentation(provider),
        UnicodeProperty::ExtendedPictographic => load_extended_pictographic(provider),
        UnicodeProperty::Extender => load_extender(provider),
        UnicodeProperty::GraphemeBase => load_grapheme_base(provider),
        UnicodeProperty::GraphemeExtend => load_grapheme_extend(provider),
        UnicodeProperty::HexDigit => load_hex_digit(provider),
        UnicodeProperty::IdsBinaryOperator => load_ids_binary_operator(provider),
        UnicodeProperty::IdsTrinaryOperator => load_ids_trinary_operator(provider),
        UnicodeProperty::IdContinue => load_id_continue(provider),
        UnicodeProperty::IdStart => load_id_start(provider),
        UnicodeProperty::Ideographic => load_ideographic(provider),
        UnicodeProperty::JoinControl => load_join_control(provider),
        UnicodeProperty::LogicalOrderException => load_logical_order_exception(provider),
        UnicodeProperty::Lowercase => load_lowercase(provider),
        UnicodeProperty::Math => load_math(provider),
        UnicodeProperty::NoncharacterCodePoint => load_noncharacter_code_point(provider),
        UnicodeProperty::PatternSyntax => load_pattern_syntax(provider),
        UnicodeProperty::PatternWhiteSpace => load_pattern_white_space(provider),
        UnicodeProperty::QuotationMark => load_quotation_mark(provider),
        UnicodeProperty::Radical => load_radical(provider),
        UnicodeProperty::RegionalIndicator => load_regional_indicator(provider),
        UnicodeProperty::SentenceTerminal => load_sentence_terminal(provider),
        UnicodeProperty::SoftDotted => load_soft_dotted(provider),
        UnicodeProperty::TerminalPunctuation => load_terminal_punctuation(provider),
        UnicodeProperty::UnifiedIdeograph => load_unified_ideograph(provider),
        UnicodeProperty::Uppercase => load_uppercase(provider),
        UnicodeProperty::VariationSelector => load_variation_selector(provider),
        UnicodeProperty::WhiteSpace => load_white_space(provider),
        UnicodeProperty::XidContinue => load_xid_continue(provider),
        UnicodeProperty::XidStart => load_xid_start(provider),
        _ => Err(PropertiesError::UnexpectedPropertyName),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_general_category() {
        use icu::properties::sets;
        use icu::properties::GeneralCategoryGroup;

        let digits_data = sets::for_general_category_group(GeneralCategoryGroup::Number);
        let digits = digits_data.as_borrowed();

        assert!(digits.contains('5'));
        assert!(digits.contains('\u{0665}')); // U+0665 ARABIC-INDIC DIGIT FIVE
        assert!(digits.contains('\u{096b}')); // U+0969 DEVANAGARI DIGIT FIVE

        assert!(!digits.contains('A'));
    }

    #[test]
    fn test_script() {
        use icu::properties::maps;
        use icu::properties::Script;

        let thai_data = maps::script().get_set_for_value(Script::Thai);
        let thai = thai_data.as_borrowed();

        assert!(thai.contains('\u{0e01}')); // U+0E01 THAI CHARACTER KO KAI
        assert!(thai.contains('\u{0e50}')); // U+0E50 THAI DIGIT ZERO

        assert!(!thai.contains('A'));
        assert!(!thai.contains('\u{0e3f}')); // U+0E50 THAI CURRENCY SYMBOL BAHT
    }

    #[test]
    fn test_gc_groupings() {
        use icu::properties::{maps, sets};
        use icu::properties::{GeneralCategory, GeneralCategoryGroup};
        use icu_collections::codepointinvlist::CodePointInversionListBuilder;

        let test_group = |category: GeneralCategoryGroup, subcategories: &[GeneralCategory]| {
            let category_set = sets::for_general_category_group(category);
            let category_set = category_set
                .as_code_point_inversion_list()
                .expect("The data should be valid");

            let mut builder = CodePointInversionListBuilder::new();
            for subcategory in subcategories {
                let gc_set_data = &maps::general_category().get_set_for_value(*subcategory);
                let gc_set = gc_set_data.as_borrowed();
                for range in gc_set.iter_ranges() {
                    builder.add_range32(&range);
                }
            }
            let combined_set = builder.build();
            println!("{category:?} {subcategories:?}");
            assert_eq!(
                category_set.get_inversion_list_vec(),
                combined_set.get_inversion_list_vec()
            );
        };

        test_group(
            GeneralCategoryGroup::Letter,
            &[
                GeneralCategory::UppercaseLetter,
                GeneralCategory::LowercaseLetter,
                GeneralCategory::TitlecaseLetter,
                GeneralCategory::ModifierLetter,
                GeneralCategory::OtherLetter,
            ],
        );
        test_group(
            GeneralCategoryGroup::Other,
            &[
                GeneralCategory::Control,
                GeneralCategory::Format,
                GeneralCategory::Unassigned,
                GeneralCategory::PrivateUse,
                GeneralCategory::Surrogate,
            ],
        );
        test_group(
            GeneralCategoryGroup::Mark,
            &[
                GeneralCategory::SpacingMark,
                GeneralCategory::EnclosingMark,
                GeneralCategory::NonspacingMark,
            ],
        );
        test_group(
            GeneralCategoryGroup::Number,
            &[
                GeneralCategory::DecimalNumber,
                GeneralCategory::LetterNumber,
                GeneralCategory::OtherNumber,
            ],
        );
        test_group(
            GeneralCategoryGroup::Punctuation,
            &[
                GeneralCategory::ConnectorPunctuation,
                GeneralCategory::DashPunctuation,
                GeneralCategory::ClosePunctuation,
                GeneralCategory::FinalPunctuation,
                GeneralCategory::InitialPunctuation,
                GeneralCategory::OtherPunctuation,
                GeneralCategory::OpenPunctuation,
            ],
        );
        test_group(
            GeneralCategoryGroup::Symbol,
            &[
                GeneralCategory::CurrencySymbol,
                GeneralCategory::ModifierSymbol,
                GeneralCategory::MathSymbol,
                GeneralCategory::OtherSymbol,
            ],
        );
        test_group(
            GeneralCategoryGroup::Separator,
            &[
                GeneralCategory::LineSeparator,
                GeneralCategory::ParagraphSeparator,
                GeneralCategory::SpaceSeparator,
            ],
        );
    }

    #[test]
    fn test_gc_surrogate() {
        use icu::properties::maps;
        use icu::properties::GeneralCategory;

        let surrogates_data =
            maps::general_category().get_set_for_value(GeneralCategory::Surrogate);
        let surrogates = surrogates_data.as_borrowed();

        assert!(surrogates.contains32(0xd800));
        assert!(surrogates.contains32(0xd900));
        assert!(surrogates.contains32(0xdfff));

        assert!(!surrogates.contains('A'));
    }
}
