//! Configuration specific to the `forge fmt` command and the `forge_fmt` package

use serde::{Deserialize, Serialize};

/// Contains the config and rule set
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatterConfig {
    /// Maximum line length where formatter will try to wrap the line
    pub line_length: usize,
    /// Number of spaces per indentation level
    pub tab_width: usize,
    /// Print spaces between brackets
    pub bracket_spacing: bool,
    /// Style of uint/int256 types
    pub int_types: IntTypes,
    /// If function parameters are multiline then always put the function attributes on separate
    /// lines
    pub func_attrs_with_params_multiline: bool,
    /// Style of quotation marks
    pub quote_style: QuoteStyle,
    /// Style of underscores in number literals
    pub number_underscore: NumberUnderscore,
}

/// Style of uint/int256 types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntTypes {
    /// Print the explicit uint256 or int256
    Long,
    /// Print the implicit uint or int
    Short,
    /// Use the type defined in the source code
    Preserve,
}

/// Style of underscores in number literals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberUnderscore {
    /// Remove all underscores
    Remove,
    /// Add an underscore every thousand, if greater than 9999
    /// e.g. 1000 -> 1000 and 10000 -> 10_000
    Thousands,
    /// Use the underscores defined in the source code
    Preserve,
}

/// Style of string quotes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
    /// Use double quotes where possible
    Double,
    /// Use single quotes where possible
    Single,
    /// Use quotation mark defined in the source code
    Preserve,
}

impl QuoteStyle {
    /// Get associated quotation mark with option
    pub fn quote(self) -> Option<char> {
        match self {
            QuoteStyle::Double => Some('"'),
            QuoteStyle::Single => Some('\''),
            QuoteStyle::Preserve => None,
        }
    }
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig {
            line_length: 120,
            tab_width: 4,
            bracket_spacing: false,
            int_types: IntTypes::Long,
            func_attrs_with_params_multiline: true,
            quote_style: QuoteStyle::Double,
            number_underscore: NumberUnderscore::Preserve,
        }
    }
}
