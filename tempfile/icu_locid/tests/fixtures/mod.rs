// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};

use icu_locid::extensions::private;
use icu_locid::extensions::transform;
use icu_locid::extensions::unicode;
use icu_locid::extensions::Extensions;
use icu_locid::{subtags, LanguageIdentifier, Locale, ParserError};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleIdentifier {
    #[serde(rename = "type")]
    pub field_type: String,
    pub identifier: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleExtensionUnicode {
    #[serde(default)]
    keywords: HashMap<String, Option<String>>,
    #[serde(default)]
    attributes: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleExtensionTransform {
    tlang: Option<String>,
    #[serde(default)]
    tfields: HashMap<String, Option<String>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleExtensions {
    unicode: Option<LocaleExtensionUnicode>,
    transform: Option<LocaleExtensionTransform>,
    #[serde(default)]
    private: Vec<String>,
    _other: Option<String>,
}

impl TryFrom<LocaleExtensions> for Extensions {
    type Error = ParserError;

    fn try_from(input: LocaleExtensions) -> Result<Self, Self::Error> {
        let mut ext = Extensions::default();
        if let Some(unicode) = input.unicode {
            ext.unicode.keywords = unicode
                .keywords
                .iter()
                .map(|(k, v)| {
                    (
                        unicode::Key::try_from_bytes(k.as_bytes()).expect("Parsing key failed."),
                        v.as_ref().map_or(
                            unicode::Value::try_from_bytes(b"").expect("Failed to parse Value"),
                            |v| {
                                unicode::Value::try_from_bytes(v.as_bytes())
                                    .expect("Parsing type failed.")
                            },
                        ),
                    )
                })
                .collect();
            let v: Vec<unicode::Attribute> = unicode
                .attributes
                .iter()
                .map(|v| {
                    unicode::Attribute::try_from_bytes(v.as_bytes())
                        .expect("Parsing attribute failed.")
                })
                .collect();
            ext.unicode.attributes = unicode::Attributes::from_vec_unchecked(v);
        }
        if let Some(transform) = input.transform {
            ext.transform.fields = transform
                .tfields
                .iter()
                .map(|(k, v)| {
                    (
                        transform::Key::try_from_bytes(k.as_bytes()).expect("Parsing key failed."),
                        v.as_ref()
                            .map(|v| {
                                transform::Value::try_from_bytes(v.as_bytes())
                                    .expect("Parsing value failed.")
                            })
                            .expect("Value cannot be empty."),
                    )
                })
                .collect();

            if let Some(tlang) = transform.tlang {
                ext.transform.lang = Some(tlang.parse().expect("Failed to parse tlang."));
            }
        }
        let v: Vec<private::Subtag> = input
            .private
            .iter()
            .map(|v| private::Subtag::try_from_bytes(v.as_bytes()).expect("Failed to add field."))
            .collect();
        ext.private = private::Private::from_vec_unchecked(v);
        Ok(ext)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleSubtags {
    #[serde(rename = "type")]
    pub field_type: String,
    pub language: Option<String>,
    pub script: Option<String>,
    pub region: Option<String>,
    #[serde(default)]
    pub variants: Vec<String>,
    pub extensions: Option<LocaleExtensions>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocaleError {
    pub error: String,
    pub text: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // test code
pub enum LocaleInfo {
    String(String),
    Error(LocaleError),
    Identifier(LocaleIdentifier),
    Object(LocaleSubtags),
}

impl TryFrom<LocaleInfo> for LanguageIdentifier {
    type Error = ParserError;

    fn try_from(input: LocaleInfo) -> Result<Self, Self::Error> {
        match input {
            LocaleInfo::String(s) => s.parse(),
            LocaleInfo::Error(e) => Err(e.into()),
            LocaleInfo::Identifier(ident) => ident.try_into(),
            LocaleInfo::Object(o) => o.try_into(),
        }
    }
}

impl TryFrom<LocaleInfo> for Locale {
    type Error = ParserError;

    fn try_from(input: LocaleInfo) -> Result<Self, Self::Error> {
        match input {
            LocaleInfo::String(s) => s.parse(),
            LocaleInfo::Error(e) => Err(e.into()),
            LocaleInfo::Identifier(ident) => ident.try_into(),
            LocaleInfo::Object(o) => o.try_into(),
        }
    }
}

impl TryFrom<LocaleIdentifier> for LanguageIdentifier {
    type Error = ParserError;

    fn try_from(input: LocaleIdentifier) -> Result<Self, Self::Error> {
        LanguageIdentifier::try_from_locale_bytes(input.identifier.as_bytes())
    }
}

impl TryFrom<LocaleIdentifier> for Locale {
    type Error = ParserError;

    fn try_from(input: LocaleIdentifier) -> Result<Self, Self::Error> {
        Locale::try_from_bytes(input.identifier.as_bytes())
    }
}

impl TryFrom<LocaleSubtags> for LanguageIdentifier {
    type Error = ParserError;

    fn try_from(subtags: LocaleSubtags) -> Result<Self, Self::Error> {
        let language = if let Some(lang) = subtags.language {
            lang.parse().expect("Failed to parse language subtag")
        } else {
            subtags::Language::default()
        };
        let script = subtags
            .script
            .map(|s| s.parse().expect("Failed to parse script subtag."));
        let region = subtags
            .region
            .map(|s| s.parse().expect("Failed to parse region subtag."));
        let variants = subtags
            .variants
            .iter()
            .map(|v| v.parse().expect("Failed to parse variant subtag."))
            .collect::<Vec<_>>();
        Ok(LanguageIdentifier {
            language,
            script,
            region,
            variants: subtags::Variants::from_vec_unchecked(variants),
        })
    }
}

impl TryFrom<LocaleSubtags> for Locale {
    type Error = ParserError;

    fn try_from(subtags: LocaleSubtags) -> Result<Self, Self::Error> {
        let language = if let Some(lang) = subtags.language {
            lang.parse().expect("Failed to parse language subtag")
        } else {
            subtags::Language::default()
        };
        let script = subtags
            .script
            .map(|s| s.parse().expect("Failed to parse script subtag."));
        let region = subtags
            .region
            .map(|s| s.parse().expect("Failed to parse region subtag."));
        let variants = subtags
            .variants
            .iter()
            .map(|v| v.parse().expect("Failed to parse variant subtag."))
            .collect::<Vec<_>>();
        let extensions = if let Some(e) = subtags.extensions {
            e.try_into().expect("Failed to parse extensions.")
        } else {
            Extensions::default()
        };
        Ok(Locale {
            id: LanguageIdentifier {
                language,
                script,
                region,
                variants: subtags::Variants::from_vec_unchecked(variants),
            },
            extensions,
        })
    }
}

impl From<LocaleError> for ParserError {
    fn from(e: LocaleError) -> Self {
        match e.error.as_str() {
            "InvalidLanguage" => ParserError::InvalidLanguage,
            "InvalidSubtag" => ParserError::InvalidSubtag,
            "InvalidExtension" => ParserError::InvalidExtension,
            "DuplicatedExtension" => ParserError::DuplicatedExtension,
            _ => unreachable!("Unknown error name"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LocaleTest {
    pub input: LocaleInfo,
    pub output: LocaleInfo,
}
