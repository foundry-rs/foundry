// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

//! This module provides APIs for getting exemplar characters for a locale.
//!
//! Exemplars are characters used by a language, separated into different sets.
//! The sets are: main, auxiliary, punctuation, numbers, and index.
//!
//! The sets define, according to typical usage in the language,
//! which characters occur in which contexts with which frequency.
//! For more information, see the documentation in the
//! [Exemplars section in Unicode Technical Standard #35](https://unicode.org/reports/tr35/tr35-general.html#Exemplars)
//! of the LDML specification.
//!
//! # Examples
//!
//! ```
//! use icu::locid::locale;
//! use icu::properties::exemplar_chars;
//!
//! let locale = locale!("en-001").into();
//! let data = exemplar_chars::exemplars_main(&locale)
//!     .expect("locale should be present");
//! let exemplars_main = data.as_borrowed();
//!
//! assert!(exemplars_main.contains_char('a'));
//! assert!(exemplars_main.contains_char('z'));
//! assert!(exemplars_main.contains("a"));
//! assert!(!exemplars_main.contains("ä"));
//! assert!(!exemplars_main.contains("ng"));
//! ```

use crate::provider::*;
use crate::sets::UnicodeSetData;
use crate::PropertiesError;
use icu_provider::prelude::*;

macro_rules! make_exemplar_chars_unicode_set_property {
    (
        // currently unused
        marker: $marker_name:ident;
        keyed_data_marker: $keyed_data_marker:ty;
        func:
        $vis:vis fn $funcname:ident();
        $(#[$attr:meta])*
        $vis2:vis fn $constname:ident();
    ) => {
        #[doc = concat!("A version of [`", stringify!($constname), "()`] that uses custom data provided by a [`DataProvider`].")]
        ///
        /// [📚 Help choosing a constructor](icu_provider::constructors)
        $vis fn $funcname(
            provider: &(impl DataProvider<$keyed_data_marker> + ?Sized),
            locale: &DataLocale,
        ) -> Result<UnicodeSetData, PropertiesError> {
            Ok(provider.load(
                DataRequest {
                    locale,
                    metadata: Default::default(),
                })
                .and_then(DataResponse::take_payload)
                .map(UnicodeSetData::from_data)?
            )
        }
        $(#[$attr])*
        #[cfg(feature = "compiled_data")]
        $vis2 fn $constname(
            locale: &DataLocale,
        ) -> Result<UnicodeSetData, PropertiesError> {
            Ok(UnicodeSetData::from_data(
                DataProvider::<$keyed_data_marker>::load(
                    &crate::provider::Baked,
                    DataRequest {
                        locale,
                        metadata: Default::default(),
                    })
                    .and_then(DataResponse::take_payload)?
            ))
        }
    }
}

make_exemplar_chars_unicode_set_property!(
    marker: ExemplarCharactersMain;
    keyed_data_marker: ExemplarCharactersMainV1Marker;
    func:
    pub fn load_exemplars_main();

    /// Get the "main" set of exemplar characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::properties::exemplar_chars;
    ///
    /// let data = exemplar_chars::exemplars_main(&locale!("en").into())
    ///     .expect("locale should be present");
    /// let exemplars_main = data.as_borrowed();
    ///
    /// assert!(exemplars_main.contains_char('a'));
    /// assert!(exemplars_main.contains_char('z'));
    /// assert!(exemplars_main.contains("a"));
    /// assert!(!exemplars_main.contains("ä"));
    /// assert!(!exemplars_main.contains("ng"));
    /// assert!(!exemplars_main.contains("A"));
    /// ```
    pub fn exemplars_main();
);

make_exemplar_chars_unicode_set_property!(
    marker: ExemplarCharactersAuxiliary;
    keyed_data_marker: ExemplarCharactersAuxiliaryV1Marker;
    func:
    pub fn load_exemplars_auxiliary();

    /// Get the "auxiliary" set of exemplar characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::properties::exemplar_chars;
    ///
    /// let data =
    ///     exemplar_chars::exemplars_auxiliary(&locale!("en").into())
    ///     .expect("locale should be present");
    /// let exemplars_auxiliary = data.as_borrowed();
    ///
    /// assert!(!exemplars_auxiliary.contains_char('a'));
    /// assert!(!exemplars_auxiliary.contains_char('z'));
    /// assert!(!exemplars_auxiliary.contains("a"));
    /// assert!(exemplars_auxiliary.contains("ä"));
    /// assert!(!exemplars_auxiliary.contains("ng"));
    /// assert!(!exemplars_auxiliary.contains("A"));
    /// ```
    pub fn exemplars_auxiliary();
);

make_exemplar_chars_unicode_set_property!(
    marker: ExemplarCharactersPunctuation;
    keyed_data_marker: ExemplarCharactersPunctuationV1Marker;
    func:
    pub fn load_exemplars_punctuation();

    /// Get the "punctuation" set of exemplar characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::properties::exemplar_chars;
    ///
    /// let data =
    ///     exemplar_chars::exemplars_punctuation(&locale!("en").into())
    ///     .expect("locale should be present");
    /// let exemplars_punctuation = data.as_borrowed();
    ///
    /// assert!(!exemplars_punctuation.contains_char('0'));
    /// assert!(!exemplars_punctuation.contains_char('9'));
    /// assert!(!exemplars_punctuation.contains_char('%'));
    /// assert!(exemplars_punctuation.contains_char(','));
    /// assert!(exemplars_punctuation.contains_char('.'));
    /// assert!(exemplars_punctuation.contains_char('!'));
    /// assert!(exemplars_punctuation.contains_char('?'));
    /// ```
    pub fn exemplars_punctuation();
);

make_exemplar_chars_unicode_set_property!(
    marker: ExemplarCharactersNumbers;
    keyed_data_marker: ExemplarCharactersNumbersV1Marker;
    func:
    pub fn load_exemplars_numbers();

    /// Get the "numbers" set of exemplar characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::properties::exemplar_chars;
    ///
    /// let data =
    ///     exemplar_chars::exemplars_numbers(&locale!("en").into())
    ///     .expect("locale should be present");
    /// let exemplars_numbers = data.as_borrowed();
    ///
    /// assert!(exemplars_numbers.contains_char('0'));
    /// assert!(exemplars_numbers.contains_char('9'));
    /// assert!(exemplars_numbers.contains_char('%'));
    /// assert!(exemplars_numbers.contains_char(','));
    /// assert!(exemplars_numbers.contains_char('.'));
    /// assert!(!exemplars_numbers.contains_char('!'));
    /// assert!(!exemplars_numbers.contains_char('?'));
    /// ```
    pub fn exemplars_numbers();
);

make_exemplar_chars_unicode_set_property!(
    marker: ExemplarCharactersIndex;
    keyed_data_marker: ExemplarCharactersIndexV1Marker;
    func:
    pub fn load_exemplars_index();

    /// Get the "index" set of exemplar characters.
    ///
    /// ✨ *Enabled with the `compiled_data` Cargo feature.*
    ///
    /// [📚 Help choosing a constructor](icu_provider::constructors)
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::locale;
    /// use icu::properties::exemplar_chars;
    ///
    /// let data =
    ///     exemplar_chars::exemplars_index(&locale!("en").into())
    ///     .expect("locale should be present");
    /// let exemplars_index = data.as_borrowed();
    ///
    /// assert!(!exemplars_index.contains_char('a'));
    /// assert!(!exemplars_index.contains_char('z'));
    /// assert!(!exemplars_index.contains("a"));
    /// assert!(!exemplars_index.contains("ä"));
    /// assert!(!exemplars_index.contains("ng"));
    /// assert!(exemplars_index.contains("A"));
    /// ```
    pub fn exemplars_index();
);
