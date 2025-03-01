// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

pub use super::errors::ParserError;
use crate::extensions::unicode::{Attribute, Key, Value};
use crate::extensions::ExtensionType;
use crate::parser::SubtagIterator;
use crate::shortvec::ShortBoxSlice;
use crate::LanguageIdentifier;
use crate::{extensions, subtags};
use tinystr::TinyAsciiStr;

#[derive(PartialEq, Clone, Copy)]
pub enum ParserMode {
    LanguageIdentifier,
    Locale,
    Partial,
}

#[derive(PartialEq, Clone, Copy)]
enum ParserPosition {
    Script,
    Region,
    Variant,
}

pub fn parse_language_identifier_from_iter(
    iter: &mut SubtagIterator,
    mode: ParserMode,
) -> Result<LanguageIdentifier, ParserError> {
    let mut script = None;
    let mut region = None;
    let mut variants = ShortBoxSlice::new();

    let language = if let Some(subtag) = iter.next() {
        subtags::Language::try_from_bytes(subtag)?
    } else {
        return Err(ParserError::InvalidLanguage);
    };

    let mut position = ParserPosition::Script;

    while let Some(subtag) = iter.peek() {
        if mode != ParserMode::LanguageIdentifier && subtag.len() == 1 {
            break;
        }

        if position == ParserPosition::Script {
            if let Ok(s) = subtags::Script::try_from_bytes(subtag) {
                script = Some(s);
                position = ParserPosition::Region;
            } else if let Ok(s) = subtags::Region::try_from_bytes(subtag) {
                region = Some(s);
                position = ParserPosition::Variant;
            } else if let Ok(v) = subtags::Variant::try_from_bytes(subtag) {
                if let Err(idx) = variants.binary_search(&v) {
                    variants.insert(idx, v);
                }
                position = ParserPosition::Variant;
            } else if mode == ParserMode::Partial {
                break;
            } else {
                return Err(ParserError::InvalidSubtag);
            }
        } else if position == ParserPosition::Region {
            if let Ok(s) = subtags::Region::try_from_bytes(subtag) {
                region = Some(s);
                position = ParserPosition::Variant;
            } else if let Ok(v) = subtags::Variant::try_from_bytes(subtag) {
                if let Err(idx) = variants.binary_search(&v) {
                    variants.insert(idx, v);
                }
                position = ParserPosition::Variant;
            } else if mode == ParserMode::Partial {
                break;
            } else {
                return Err(ParserError::InvalidSubtag);
            }
        } else if let Ok(v) = subtags::Variant::try_from_bytes(subtag) {
            if let Err(idx) = variants.binary_search(&v) {
                variants.insert(idx, v);
            } else {
                return Err(ParserError::InvalidSubtag);
            }
        } else if mode == ParserMode::Partial {
            break;
        } else {
            return Err(ParserError::InvalidSubtag);
        }
        iter.next();
    }

    Ok(LanguageIdentifier {
        language,
        script,
        region,
        variants: subtags::Variants::from_short_slice_unchecked(variants),
    })
}

pub fn parse_language_identifier(
    t: &[u8],
    mode: ParserMode,
) -> Result<LanguageIdentifier, ParserError> {
    let mut iter = SubtagIterator::new(t);
    parse_language_identifier_from_iter(&mut iter, mode)
}

#[allow(clippy::type_complexity)]
pub const fn parse_locale_with_single_variant_single_keyword_unicode_extension_from_iter(
    mut iter: SubtagIterator,
    mode: ParserMode,
) -> Result<
    (
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
        Option<subtags::Variant>,
        Option<(extensions::unicode::Key, Option<TinyAsciiStr<8>>)>,
    ),
    ParserError,
> {
    let language;
    let mut script = None;
    let mut region = None;
    let mut variant = None;
    let mut keyword = None;

    if let (i, Some((start, end))) = iter.next_manual() {
        iter = i;
        match subtags::Language::try_from_bytes_manual_slice(iter.slice, start, end) {
            Ok(l) => language = l,
            Err(e) => return Err(e),
        }
    } else {
        return Err(ParserError::InvalidLanguage);
    }

    let mut position = ParserPosition::Script;

    while let Some((start, end)) = iter.peek_manual() {
        if !matches!(mode, ParserMode::LanguageIdentifier) && end - start == 1 {
            break;
        }

        if matches!(position, ParserPosition::Script) {
            if let Ok(s) = subtags::Script::try_from_bytes_manual_slice(iter.slice, start, end) {
                script = Some(s);
                position = ParserPosition::Region;
            } else if let Ok(r) =
                subtags::Region::try_from_bytes_manual_slice(iter.slice, start, end)
            {
                region = Some(r);
                position = ParserPosition::Variant;
            } else if let Ok(v) =
                subtags::Variant::try_from_bytes_manual_slice(iter.slice, start, end)
            {
                // We cannot handle multiple variants in a const context
                debug_assert!(variant.is_none());
                variant = Some(v);
                position = ParserPosition::Variant;
            } else if matches!(mode, ParserMode::Partial) {
                break;
            } else {
                return Err(ParserError::InvalidSubtag);
            }
        } else if matches!(position, ParserPosition::Region) {
            if let Ok(s) = subtags::Region::try_from_bytes_manual_slice(iter.slice, start, end) {
                region = Some(s);
                position = ParserPosition::Variant;
            } else if let Ok(v) =
                subtags::Variant::try_from_bytes_manual_slice(iter.slice, start, end)
            {
                // We cannot handle multiple variants in a const context
                debug_assert!(variant.is_none());
                variant = Some(v);
                position = ParserPosition::Variant;
            } else if matches!(mode, ParserMode::Partial) {
                break;
            } else {
                return Err(ParserError::InvalidSubtag);
            }
        } else if let Ok(v) = subtags::Variant::try_from_bytes_manual_slice(iter.slice, start, end)
        {
            debug_assert!(matches!(position, ParserPosition::Variant));
            if variant.is_some() {
                // We cannot handle multiple variants in a const context
                return Err(ParserError::InvalidSubtag);
            }
            variant = Some(v);
        } else if matches!(mode, ParserMode::Partial) {
            break;
        } else {
            return Err(ParserError::InvalidSubtag);
        }

        iter = iter.next_manual().0;
    }

    if matches!(mode, ParserMode::Locale) {
        if let Some((start, end)) = iter.peek_manual() {
            match ExtensionType::try_from_bytes_manual_slice(iter.slice, start, end) {
                Ok(ExtensionType::Unicode) => {
                    iter = iter.next_manual().0;
                    if let Some((start, end)) = iter.peek_manual() {
                        if Attribute::try_from_bytes_manual_slice(iter.slice, start, end).is_ok() {
                            // We cannot handle Attributes in a const context
                            return Err(ParserError::InvalidSubtag);
                        }
                    }

                    let mut key = None;
                    let mut current_type = None;

                    while let Some((start, end)) = iter.peek_manual() {
                        let slen = end - start;
                        if slen == 2 {
                            if key.is_some() {
                                // We cannot handle more than one Key in a const context
                                return Err(ParserError::InvalidSubtag);
                            }
                            match Key::try_from_bytes_manual_slice(iter.slice, start, end) {
                                Ok(k) => key = Some(k),
                                Err(e) => return Err(e),
                            };
                        } else if key.is_some() {
                            match Value::parse_subtag_from_bytes_manual_slice(
                                iter.slice, start, end,
                            ) {
                                Ok(Some(t)) => {
                                    if current_type.is_some() {
                                        // We cannot handle more than one type in a const context
                                        return Err(ParserError::InvalidSubtag);
                                    }
                                    current_type = Some(t);
                                }
                                Ok(None) => {}
                                Err(e) => return Err(e),
                            }
                        } else {
                            break;
                        }
                        iter = iter.next_manual().0
                    }
                    if let Some(k) = key {
                        keyword = Some((k, current_type));
                    }
                }
                // We cannot handle Transform, Private, Other extensions in a const context
                Ok(_) => return Err(ParserError::InvalidSubtag),
                Err(e) => return Err(e),
            }
        }
    }

    Ok((language, script, region, variant, keyword))
}

#[allow(clippy::type_complexity)]
pub const fn parse_language_identifier_with_single_variant(
    t: &[u8],
    mode: ParserMode,
) -> Result<
    (
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
        Option<subtags::Variant>,
    ),
    ParserError,
> {
    let iter = SubtagIterator::new(t);
    match parse_locale_with_single_variant_single_keyword_unicode_extension_from_iter(iter, mode) {
        Ok((l, s, r, v, _)) => Ok((l, s, r, v)),
        Err(e) => Err(e),
    }
}
