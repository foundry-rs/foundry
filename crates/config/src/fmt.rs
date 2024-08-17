//! Configuration specific to the `forge fmt` command and the `forge_fmt` package

use serde::{Deserialize, Serialize};

/// Contains the config and rule set
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatterConfig {
    /// Maximum line length where formatter will try to wrap the line
    pub line_length: usize,
    /// Number of spaces per indentation level
    pub tab_width: usize,
    /// Print spaces between brackets
    pub bracket_spacing: bool,
    /// Style of uint/int256 types
    pub int_types: IntTypes,
    /// Style of multiline function header in case it doesn't fit
    pub multiline_func_header: MultilineFuncHeaderStyle,
    /// Style of quotation marks
    pub quote_style: QuoteStyle,
    /// Style of underscores in number literals
    pub number_underscore: NumberUnderscore,
    /// Style of underscores in hex literals
    pub hex_underscore: HexUnderscore,
    /// Style of single line blocks in statements
    pub single_line_statement_blocks: SingleLineBlockStyle,
    /// Print space in state variable, function and modifier `override` attribute
    pub override_spacing: bool,
    /// Wrap comments on `line_length` reached
    pub wrap_comments: bool,
    /// Globs to ignore
    pub ignore: Vec<String>,
    /// Add new line at start and end of contract declarations
    pub contract_new_lines: bool,
    /// Sort import statements alphabetically in groups (a group is separated by a newline).
    pub sort_imports: bool,
}

/// Style of uint/int256 types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberUnderscore {
    /// Use the underscores defined in the source code
    Preserve,
    /// Remove all underscores
    #[default]
    Remove,
    /// Add an underscore every thousand, if greater than 9999
    /// e.g. 1000 -> 1000 and 10000 -> 10_000
    Thousands,
}

impl NumberUnderscore {
    /// Returns true if the option is `Preserve`
    #[inline]
    pub fn is_preserve(self) -> bool {
        matches!(self, Self::Preserve)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub fn is_remove(self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub fn is_thousands(self) -> bool {
        matches!(self, Self::Thousands)
    }
}

/// Style of underscores in hex literals
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexUnderscore {
    /// Use the underscores defined in the source code
    Preserve,
    /// Remove all underscores
    #[default]
    Remove,
    /// Add underscore as separator between byte boundaries
    Bytes,
}

impl HexUnderscore {
    /// Returns true if the option is `Preserve`
    #[inline]
    pub fn is_preserve(self) -> bool {
        matches!(self, Self::Preserve)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub fn is_remove(self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Returns true if the option is `Remove`
    #[inline]
    pub fn is_bytes(self) -> bool {
        matches!(self, Self::Bytes)
    }
}

/// Style of string quotes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            Self::Double => Some('"'),
            Self::Single => Some('\''),
            Self::Preserve => None,
        }
    }
}

/// Style of single line blocks in statements
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleLineBlockStyle {
    /// Prefer single line block when possible
    Single,
    /// Always use multiline block
    Multi,
    /// Preserve the original style
    Preserve,
}

/// Style of function header in case it doesn't fit
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultilineFuncHeaderStyle {
    /// Write function parameters multiline first
    ParamsFirst,
    /// Write function attributes multiline first
    AttributesFirst,
    /// If function params or attrs are multiline
    /// split the rest
    All,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            line_length: 120,
            tab_width: 4,
            bracket_spacing: false,
            int_types: IntTypes::Long,
            multiline_func_header: MultilineFuncHeaderStyle::AttributesFirst,
            quote_style: QuoteStyle::Double,
            number_underscore: NumberUnderscore::Preserve,
            hex_underscore: HexUnderscore::Remove,
            single_line_statement_blocks: SingleLineBlockStyle::Preserve,
            override_spacing: false,
            wrap_comments: false,
            ignore: vec![],
            contract_new_lines: false,
            sort_imports: false,
        }
    }
}
