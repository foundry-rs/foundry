//! A wrapper around [solang_parser](solang_parser::parse) parser to generate a
//! [solang_parser::ast::SourceUnit](solang_parser::ast::SourceUnit) from a solidity source string.

use std::rc::Rc;

use serde::{Deserialize, Serialize, Serializer};
use solang_parser::diagnostics::Diagnostic;

/// Represents a parsed snippet of Solidity code.
#[derive(Debug, Deserialize)]
pub struct ParsedSnippet {
    /// The parsed source unit
    #[serde(deserialize_with = "deserialize_source_unit")]
    pub source_unit: Option<(solang_parser::pt::SourceUnit, Vec<solang_parser::pt::Comment>)>,
    /// The raw source code
    #[serde(deserialize_with = "deserialize_raw")]
    pub raw: Rc<String>,
}

impl ParsedSnippet {
    /// Creates a new ParsedSnippet from a raw source string
    pub fn new(raw: &str) -> Self {
        Self { source_unit: None, raw: Rc::new(raw.to_string()) }
    }

    /// Parses the raw source string into a
    /// [solang_parser::pt::SourceUnit](solang_parser::pt::SourceUnit) and comments
    pub fn parse(&mut self) -> Result<(), Vec<Diagnostic>> {
        match solang_parser::parse(&self.raw, 0) {
            Ok((source_unit, comments)) => {
                self.source_unit = Some((source_unit, comments));
                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}

/// Deserialize a SourceUnit
pub fn deserialize_source_unit<'de, D>(
    deserializer: D,
) -> Result<Option<(solang_parser::pt::SourceUnit, Vec<solang_parser::pt::Comment>)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Grab the raw value
    let raw: Box<serde_json::value::RawValue> = match Box::deserialize(deserializer) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    // Parse the string, removing any quotes and adding them back in
    let raw_str = raw.get().trim_matches('"');

    // Parse the json value from string

    // Parse the serialized source unit string
    solang_parser::parse(raw_str, 0)
        .map(|(source_unit, comments)| Some((source_unit, comments)))
        .map_err(|_| serde::de::Error::custom("Failed to parse serialized string as source unit"))
}

/// Deserialize the raw source string
pub fn deserialize_raw<'de, D>(deserializer: D) -> Result<Rc<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Grab the raw value
    let raw: Box<serde_json::value::RawValue> = match Box::deserialize(deserializer) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    // Parse the string, removing any quotes and adding them back in
    let raw_str = raw.get().trim_matches('"');

    // Return a new Rc<String>
    Ok(Rc::new(raw_str.to_string()))
}

impl Serialize for ParsedSnippet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!(
            r#"{{
                    "source_unit": "{}",
                    "raw": "{}"
                }}"#,
            self.raw.as_str(),
            self.raw.as_str()
        ))
    }
}

/// Display impl for `SolToken`
impl std::fmt::Display for ParsedSnippet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}
