// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! The functions in this module return a [`CodePointMapData`] representing, for
//! each code point in the entire range of code points, the property values
//! for a particular Unicode property.
//!
//! The descriptions of most properties are taken from [`TR44`], the documentation for the
//! Unicode Character Database.
//!
//! [`TR44`]: https://www.unicode.org/reports/tr44

#[cfg(doc)]
use super::*;
use crate::error::PropertiesError;
use crate::provider::*;
use crate::sets::CodePointSetData;
use core::marker::PhantomData;
use core::ops::RangeInclusive;
use icu_collections::codepointtrie::{CodePointMapRange, CodePointTrie, TrieValue};
use icu_provider::prelude::*;
use zerovec::ZeroVecError;

/// A wrapper around code point map data. It is returned by APIs that return Unicode
/// property data in a map-like form, ex: enumerated property value data keyed
/// by code point. Access its data via the borrowed version,
/// [`CodePointMapDataBorrowed`].
#[derive(Debug, Clone)]
pub struct CodePointMapData<T: TrieValue> {
    data: DataPayload<ErasedMaplikeMarker<T>>,
}

/// Private marker type for CodePointMapData
/// to work for all same-value map properties at once
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct ErasedMaplikeMarker<T>(PhantomData<T>);
impl<T: TrieValue> DataMarker for ErasedMaplikeMarker<T> {
    type Yokeable = PropertyCodePointMapV1<'static, T>;
}

impl<T: TrieValue> CodePointMapData<T> {
    /// Construct a borrowed version of this type that can be queried.
    ///
    /// This avoids a potential small underlying cost per API call (like `get()`) by consolidating it
    /// up front.
    ///
    /// This owned version if returned by functions that use a runtime data provider.
    #[inline]
    pub fn as_borrowed(&self) -> CodePointMapDataBorrowed<'_, T> {
        CodePointMapDataBorrowed {
            map: self.data.get(),
        }
    }

    /// Convert this map to a map around another type
    ///
    /// Typically useful for type-erasing maps into maps around integers.
    ///
    /// # Panics
    /// Will panic if T and P are different sizes
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GeneralCategory};
    ///
    /// let data = maps::general_category().static_to_owned();
    ///
    /// let gc = data.try_into_converted::<u8>().unwrap();
    /// let gc = gc.as_borrowed();
    ///
    /// assert_eq!(gc.get('木'), GeneralCategory::OtherLetter as u8);  // U+6728
    /// assert_eq!(gc.get('🎃'), GeneralCategory::OtherSymbol as u8);  // U+1F383 JACK-O-LANTERN
    /// ```
    pub fn try_into_converted<P>(self) -> Result<CodePointMapData<P>, ZeroVecError>
    where
        P: TrieValue,
    {
        self.data
            .try_map_project::<ErasedMaplikeMarker<P>, _, _>(move |data, _| {
                data.try_into_converted()
            })
            .map(CodePointMapData::from_data)
    }

    /// Construct a new one from loaded data
    ///
    /// Typically it is preferable to use getters like [`load_general_category()`] instead
    pub fn from_data<M>(data: DataPayload<M>) -> Self
    where
        M: DataMarker<Yokeable = PropertyCodePointMapV1<'static, T>>,
    {
        Self { data: data.cast() }
    }

    /// Construct a new one an owned [`CodePointTrie`]
    pub fn from_code_point_trie(trie: CodePointTrie<'static, T>) -> Self {
        let set = PropertyCodePointMapV1::from_code_point_trie(trie);
        CodePointMapData::from_data(DataPayload::<ErasedMaplikeMarker<T>>::from_owned(set))
    }

    /// Convert this type to a [`CodePointTrie`] as a borrowed value.
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointTrie`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// This method returns an `Option` in order to return `None` when the backing data provider
    /// cannot return a [`CodePointTrie`], or cannot do so within the expected constant time
    /// constraint.
    pub fn as_code_point_trie(&self) -> Option<&CodePointTrie<'_, T>> {
        self.data.get().as_code_point_trie()
    }

    /// Convert this type to a [`CodePointTrie`], borrowing if possible,
    /// otherwise allocating a new [`CodePointTrie`].
    ///
    /// The data backing this is extensible and supports multiple implementations.
    /// Currently it is always [`CodePointTrie`]; however in the future more backends may be
    /// added, and users may select which at data generation time.
    ///
    /// The performance of the conversion to this specific return type will vary
    /// depending on the data structure that is backing `self`.
    pub fn to_code_point_trie(&self) -> CodePointTrie<'_, T> {
        self.data.get().to_code_point_trie()
    }
}

/// A borrowed wrapper around code point set data, returned by
/// [`CodePointSetData::as_borrowed()`]. More efficient to query.
#[derive(Clone, Copy, Debug)]
pub struct CodePointMapDataBorrowed<'a, T: TrieValue> {
    map: &'a PropertyCodePointMapV1<'a, T>,
}

impl<'a, T: TrieValue> CodePointMapDataBorrowed<'a, T> {
    /// Get the value this map has associated with code point `ch`
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GeneralCategory};
    ///
    /// let gc = maps::general_category();
    ///
    /// assert_eq!(gc.get('木'), GeneralCategory::OtherLetter);  // U+6728
    /// assert_eq!(gc.get('🎃'), GeneralCategory::OtherSymbol);  // U+1F383 JACK-O-LANTERN
    /// ```
    pub fn get(self, ch: char) -> T {
        self.map.get32(ch as u32)
    }

    /// Get the value this map has associated with code point `ch`
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GeneralCategory};
    ///
    /// let gc = maps::general_category();
    ///
    /// assert_eq!(gc.get32(0x6728), GeneralCategory::OtherLetter);  // U+6728 (木)
    /// assert_eq!(gc.get32(0x1F383), GeneralCategory::OtherSymbol);  // U+1F383 JACK-O-LANTERN
    /// ```
    pub fn get32(self, ch: u32) -> T {
        self.map.get32(ch)
    }

    /// Get a [`CodePointSetData`] for all elements corresponding to a particular value
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GeneralCategory};
    ///
    /// let gc = maps::general_category();
    ///
    /// let other_letter_set_data =
    ///     gc.get_set_for_value(GeneralCategory::OtherLetter);
    /// let other_letter_set = other_letter_set_data.as_borrowed();
    ///
    /// assert!(other_letter_set.contains('木')); // U+6728
    /// assert!(!other_letter_set.contains('🎃')); // U+1F383 JACK-O-LANTERN
    /// ```
    pub fn get_set_for_value(self, value: T) -> CodePointSetData {
        let set = self.map.get_set_for_value(value);
        CodePointSetData::from_code_point_inversion_list(set)
    }

    /// Yields an [`Iterator`] returning ranges of consecutive code points that
    /// share the same value in the [`CodePointMapData`].
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::properties::maps;
    /// use icu::properties::GeneralCategory;
    ///
    /// let gc = maps::general_category();
    /// let mut ranges = gc.iter_ranges();
    /// let next = ranges.next().unwrap();
    /// assert_eq!(next.range, 0..=31);
    /// assert_eq!(next.value, GeneralCategory::Control);
    /// let next = ranges.next().unwrap();
    /// assert_eq!(next.range, 32..=32);
    /// assert_eq!(next.value, GeneralCategory::SpaceSeparator);
    /// ```
    pub fn iter_ranges(self) -> impl Iterator<Item = CodePointMapRange<T>> + 'a {
        self.map.iter_ranges()
    }

    /// Yields an [`Iterator`] returning ranges of consecutive code points that
    /// share the same value `v` in the [`CodePointMapData`].
    ///
    /// # Examples
    ///
    ///
    /// ```
    /// use icu::properties::maps;
    /// use icu::properties::GeneralCategory;
    ///
    /// let gc = maps::general_category();
    /// let mut ranges = gc.iter_ranges_for_value(GeneralCategory::UppercaseLetter);
    /// assert_eq!(ranges.next().unwrap(), 'A' as u32..='Z' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'À' as u32..='Ö' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'Ø' as u32..='Þ' as u32);
    /// ```
    pub fn iter_ranges_for_value(self, val: T) -> impl Iterator<Item = RangeInclusive<u32>> + 'a {
        self.map
            .iter_ranges()
            .filter(move |r| r.value == val)
            .map(|r| r.range)
    }

    /// Yields an [`Iterator`] returning ranges of consecutive code points that
    /// do *not* have the value `v` in the [`CodePointMapData`].
    pub fn iter_ranges_for_value_complemented(
        self,
        val: T,
    ) -> impl Iterator<Item = RangeInclusive<u32>> + 'a {
        self.map
            .iter_ranges_mapped(move |value| value != val)
            .filter(|v| v.value)
            .map(|v| v.range)
    }

    /// Exposed for FFI needs, could be exposed in general in the future but we should
    /// have a use case first.
    ///
    /// FFI needs this since it operates on erased maps and can't use `iter_ranges_for_group()`
    #[doc(hidden)]
    pub fn iter_ranges_mapped<U: Eq + 'a>(
        self,
        predicate: impl FnMut(T) -> U + Copy + 'a,
    ) -> impl Iterator<Item = CodePointMapRange<U>> + 'a {
        self.map.iter_ranges_mapped(predicate)
    }
}

impl<T: TrieValue> CodePointMapDataBorrowed<'static, T> {
    /// Cheaply converts a [`CodePointMapDataBorrowed<'static>`] into a [`CodePointMapData`].
    ///
    /// Note: Due to branching and indirection, using [`CodePointMapData`] might inhibit some
    /// compile-time optimizations that are possible with [`CodePointMapDataBorrowed`].
    pub const fn static_to_owned(self) -> CodePointMapData<T> {
        CodePointMapData {
            data: DataPayload::from_static_ref(self.map),
        }
    }
}

impl<'a> CodePointMapDataBorrowed<'a, crate::GeneralCategory> {
    /// Yields an [`Iterator`] returning ranges of consecutive code points that
    /// have a `General_Category` value belonging to the specified [`GeneralCategoryGroup`]
    ///
    /// # Examples
    ///
    ///
    /// ```
    /// use core::ops::RangeInclusive;
    /// use icu::properties::maps::{self, CodePointMapData};
    /// use icu::properties::GeneralCategoryGroup;
    ///
    /// let gc = maps::general_category();
    /// let mut ranges = gc.iter_ranges_for_group(GeneralCategoryGroup::Letter);
    /// assert_eq!(ranges.next().unwrap(), 'A' as u32..='Z' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'a' as u32..='z' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'ª' as u32..='ª' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'µ' as u32..='µ' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'º' as u32..='º' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'À' as u32..='Ö' as u32);
    /// assert_eq!(ranges.next().unwrap(), 'Ø' as u32..='ö' as u32);
    /// ```
    pub fn iter_ranges_for_group(
        self,
        group: crate::GeneralCategoryGroup,
    ) -> impl Iterator<Item = RangeInclusive<u32>> + 'a {
        self.map
            .iter_ranges_mapped(move |value| group.contains(value))
            .filter(|v| v.value)
            .map(|v| v.range)
    }
}

macro_rules! make_map_property {
    (
        // currently unused
        property: $prop_name:expr;
        // currently unused
        marker: $marker_name:ident;
        value: $value_ty:path;
        keyed_data_marker: $keyed_data_marker:ty;
        func:
        $(#[$doc:meta])*
        $vis2:vis const $constname:ident => $singleton:ident;
        $vis:vis fn $name:ident();
    ) => {
        #[doc = concat!("A version of [`", stringify!($constname), "()`] that uses custom data provided by a [`DataProvider`].")]
        ///
        /// Note that this will return an owned version of the data. Functionality is available on
        /// the borrowed version, accessible through [`CodePointMapData::as_borrowed`].
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        $vis fn $name(
            provider: &(impl DataProvider<$keyed_data_marker> + ?Sized)
        ) -> Result<CodePointMapData<$value_ty>, PropertiesError> {
            Ok(provider.load(Default::default()).and_then(DataResponse::take_payload).map(CodePointMapData::from_data)?)
        }
        $(#[$doc])*
        #[cfg(feature = "compiled_data")]
        pub const fn $constname() -> CodePointMapDataBorrowed<'static, $value_ty> {
            CodePointMapDataBorrowed {
                map: crate::provider::Baked::$singleton
            }
        }
    };
}

make_map_property! {
    property: "General_Category";
    marker: GeneralCategoryProperty;
    value: crate::GeneralCategory;
    keyed_data_marker: GeneralCategoryV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the General_Category Unicode enumerated property. See [`GeneralCategory`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GeneralCategory};
    ///
    /// assert_eq!(maps::general_category().get('木'), GeneralCategory::OtherLetter);  // U+6728
    /// assert_eq!(maps::general_category().get('🎃'), GeneralCategory::OtherSymbol);  // U+1F383 JACK-O-LANTERN
    /// ```
    pub const general_category => SINGLETON_PROPS_GC_V1;
    pub fn load_general_category();
}

make_map_property! {
    property: "Bidi_Class";
    marker: BidiClassProperty;
    value: crate::BidiClass;
    keyed_data_marker: BidiClassV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Bidi_Class Unicode enumerated property. See [`BidiClass`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, BidiClass};
    ///
    /// assert_eq!(maps::bidi_class().get('y'), BidiClass::LeftToRight);  // U+0079
    /// assert_eq!(maps::bidi_class().get('ع'), BidiClass::ArabicLetter);  // U+0639
    /// ```
    pub const bidi_class => SINGLETON_PROPS_BC_V1;
    pub fn load_bidi_class();
}

make_map_property! {
    property: "Script";
    marker: ScriptProperty;
    value: crate::Script;
    keyed_data_marker: ScriptV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Script Unicode enumerated property. See [`Script`].
    ///
    /// **Note:** Some code points are associated with multiple scripts. If you are trying to
    /// determine whether a code point belongs to a certain script, you should use
    /// [`load_script_with_extensions_unstable`] and [`ScriptWithExtensionsBorrowed::has_script`]
    /// instead of this function.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, Script};
    ///
    /// assert_eq!(maps::script().get('木'), Script::Han);  // U+6728
    /// assert_eq!(maps::script().get('🎃'), Script::Common);  // U+1F383 JACK-O-LANTERN
    /// ```
    /// [`load_script_with_extensions_unstable`]: crate::script::load_script_with_extensions_unstable
    /// [`ScriptWithExtensionsBorrowed::has_script`]: crate::script::ScriptWithExtensionsBorrowed::has_script
    pub const script => SINGLETON_PROPS_SC_V1;
    pub fn load_script();
}

make_map_property! {
    property: "Hangul_Syllable_Type";
    marker: HangulSyllableTypeProperty;
    value: crate::HangulSyllableType;
    keyed_data_marker: HangulSyllableTypeV1Marker;
    func:
    /// Returns a [`CodePointMapDataBorrowed`] for the Hangul_Syllable_Type
    /// Unicode enumerated property. See [`HangulSyllableType`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, HangulSyllableType};
    ///
    /// assert_eq!(maps::hangul_syllable_type().get('ᄀ'), HangulSyllableType::LeadingJamo);  // U+1100
    /// assert_eq!(maps::hangul_syllable_type().get('가'), HangulSyllableType::LeadingVowelSyllable);  // U+AC00
    /// ```

    pub const hangul_syllable_type => SINGLETON_PROPS_HST_V1;
    pub fn load_hangul_syllable_type();
}

make_map_property! {
    property: "East_Asian_Width";
    marker: EastAsianWidthProperty;
    value: crate::EastAsianWidth;
    keyed_data_marker: EastAsianWidthV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the East_Asian_Width Unicode enumerated
    /// property. See [`EastAsianWidth`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, EastAsianWidth};
    ///
    /// assert_eq!(maps::east_asian_width().get('ｱ'), EastAsianWidth::Halfwidth); // U+FF71: Halfwidth Katakana Letter A
    /// assert_eq!(maps::east_asian_width().get('ア'), EastAsianWidth::Wide); //U+30A2: Katakana Letter A
    /// ```
    pub const east_asian_width => SINGLETON_PROPS_EA_V1;
    pub fn load_east_asian_width();
}

make_map_property! {
    property: "Line_Break";
    marker: LineBreakProperty;
    value: crate::LineBreak;
    keyed_data_marker: LineBreakV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Line_Break Unicode enumerated
    /// property. See [`LineBreak`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// **Note:** Use `icu::segmenter` for an all-in-one break iterator implementation.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, LineBreak};
    ///
    /// assert_eq!(maps::line_break().get(')'), LineBreak::CloseParenthesis); // U+0029: Right Parenthesis
    /// assert_eq!(maps::line_break().get('ぁ'), LineBreak::ConditionalJapaneseStarter); //U+3041: Hiragana Letter Small A
    /// ```
    pub const line_break => SINGLETON_PROPS_LB_V1;
    pub fn load_line_break();
}

make_map_property! {
    property: "Grapheme_Cluster_Break";
    marker: GraphemeClusterBreakProperty;
    value: crate::GraphemeClusterBreak;
    keyed_data_marker: GraphemeClusterBreakV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Grapheme_Cluster_Break Unicode enumerated
    /// property. See [`GraphemeClusterBreak`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// **Note:** Use `icu::segmenter` for an all-in-one break iterator implementation.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, GraphemeClusterBreak};
    ///
    /// assert_eq!(maps::grapheme_cluster_break().get('🇦'), GraphemeClusterBreak::RegionalIndicator); // U+1F1E6: Regional Indicator Symbol Letter A
    /// assert_eq!(maps::grapheme_cluster_break().get('ำ'), GraphemeClusterBreak::SpacingMark); //U+0E33: Thai Character Sara Am
    /// ```
    pub const grapheme_cluster_break => SINGLETON_PROPS_GCB_V1;
    pub fn load_grapheme_cluster_break();
}

make_map_property! {
    property: "Word_Break";
    marker: WordBreakProperty;
    value: crate::WordBreak;
    keyed_data_marker: WordBreakV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Word_Break Unicode enumerated
    /// property. See [`WordBreak`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// **Note:** Use `icu::segmenter` for an all-in-one break iterator implementation.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, WordBreak};
    ///
    /// assert_eq!(maps::word_break().get('.'), WordBreak::MidNumLet); // U+002E: Full Stop
    /// assert_eq!(maps::word_break().get('，'), WordBreak::MidNum); // U+FF0C: Fullwidth Comma
    /// ```
    pub const word_break => SINGLETON_PROPS_WB_V1;
    pub fn load_word_break();
}

make_map_property! {
    property: "Sentence_Break";
    marker: SentenceBreakProperty;
    value: crate::SentenceBreak;
    keyed_data_marker: SentenceBreakV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Sentence_Break Unicode enumerated
    /// property. See [`SentenceBreak`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// **Note:** Use `icu::segmenter` for an all-in-one break iterator implementation.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, SentenceBreak};
    ///
    /// assert_eq!(maps::sentence_break().get('９'), SentenceBreak::Numeric); // U+FF19: Fullwidth Digit Nine
    /// assert_eq!(maps::sentence_break().get(','), SentenceBreak::SContinue); // U+002C: Comma
    /// ```
    pub const sentence_break => SINGLETON_PROPS_SB_V1;
    pub fn load_sentence_break();
}

make_map_property! {
    property: "Canonical_Combining_Class";
    marker: CanonicalCombiningClassProperty;
    value: crate::CanonicalCombiningClass;
    keyed_data_marker: CanonicalCombiningClassV1Marker;
    func:
    /// Return a [`CodePointMapData`] for the Canonical_Combining_Class Unicode property. See
    /// [`CanonicalCombiningClass`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// **Note:** See `icu::normalizer::CanonicalCombiningClassMap` for the preferred API
    /// to look up the Canonical_Combining_Class property by scalar value.
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, CanonicalCombiningClass};
    ///
    /// assert_eq!(maps::canonical_combining_class().get('a'), CanonicalCombiningClass::NotReordered); // U+0061: LATIN SMALL LETTER A
    /// assert_eq!(maps::canonical_combining_class().get32(0x0301), CanonicalCombiningClass::Above); // U+0301: COMBINING ACUTE ACCENT
    /// ```
    pub const canonical_combining_class => SINGLETON_PROPS_CCC_V1;
    pub fn load_canonical_combining_class();
}

make_map_property! {
    property: "Indic_Syllabic_Category";
    marker: IndicSyllabicCategoryProperty;
    value: crate::IndicSyllabicCategory;
    keyed_data_marker: IndicSyllabicCategoryV1Marker;
    func:
    /// Return a [`CodePointMapData`] for the Indic_Syllabic_Category Unicode property. See
    /// [`IndicSyllabicCategory`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, IndicSyllabicCategory};
    ///
    /// assert_eq!(maps::indic_syllabic_category().get('a'), IndicSyllabicCategory::Other);
    /// assert_eq!(maps::indic_syllabic_category().get32(0x0900), IndicSyllabicCategory::Bindu); // U+0900: DEVANAGARI SIGN INVERTED CANDRABINDU
    /// ```
    pub const indic_syllabic_category => SINGLETON_PROPS_INSC_V1;
    pub fn load_indic_syllabic_category();
}

make_map_property! {
    property: "Joining_Type";
    marker: JoiningTypeProperty;
    value: crate::JoiningType;
    keyed_data_marker: JoiningTypeV1Marker;
    func:
    /// Return a [`CodePointMapDataBorrowed`] for the Joining_Type Unicode enumerated
    /// property. See [`JoiningType`].
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Example
    ///
    /// ```
    /// use icu::properties::{maps, JoiningType};
    ///
    /// assert_eq!(maps::joining_type().get('ؠ'), JoiningType::DualJoining); // U+0620: Arabic Letter Kashmiri Yeh
    /// assert_eq!(maps::joining_type().get('𐫍'), JoiningType::LeftJoining); // U+10ACD: Manichaean Letter Heth
    /// ```
    pub const joining_type => SINGLETON_PROPS_JT_V1;
    pub fn load_joining_type();
}
