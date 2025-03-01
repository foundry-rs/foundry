// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use displaydoc::Display;

/// List of parser errors that can be generated
/// while parsing [`LanguageIdentifier`](crate::LanguageIdentifier), [`Locale`](crate::Locale),
/// [`subtags`](crate::subtags) or [`extensions`](crate::extensions).
///
/// Re-exported as [`Error`](crate::Error).
#[derive(Display, Debug, PartialEq, Copy, Clone)]
#[non_exhaustive]
pub enum ParserError {
    /// Invalid language subtag.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::subtags::Language;
    /// use icu::locid::ParserError;
    ///
    /// assert_eq!("x2".parse::<Language>(), Err(ParserError::InvalidLanguage));
    /// ```
    #[displaydoc("The given language subtag is invalid")]
    InvalidLanguage,

    /// Invalid script, region or variant subtag.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::subtags::Region;
    /// use icu::locid::ParserError;
    ///
    /// assert_eq!("#@2X".parse::<Region>(), Err(ParserError::InvalidSubtag));
    /// ```
    #[displaydoc("Invalid subtag")]
    InvalidSubtag,

    /// Invalid extension subtag.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::extensions::unicode::Key;
    /// use icu::locid::ParserError;
    ///
    /// assert_eq!("#@2X".parse::<Key>(), Err(ParserError::InvalidExtension));
    /// ```
    #[displaydoc("Invalid extension")]
    InvalidExtension,

    /// Duplicated extension.
    ///
    /// # Examples
    ///
    /// ```
    /// use icu::locid::Locale;
    /// use icu::locid::ParserError;
    ///
    /// assert_eq!(
    ///     "und-u-hc-h12-u-ca-calendar".parse::<Locale>(),
    ///     Err(ParserError::DuplicatedExtension)
    /// );
    /// ```
    #[displaydoc("Duplicated extension")]
    DuplicatedExtension,
}

#[cfg(feature = "std")]
impl std::error::Error for ParserError {}
