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
    /// Style of uint/int256 types. Either "long" (int256), "short" (int) or "preserve (do not
    /// change where possible)
    pub int_types: IntTypes,
    /// If function parameters are multiline then always put the function attributes on separate
    /// lines
    pub func_attrs_with_params_multiline: bool,
}

/// Style of uint/int256 types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntTypes {
    /// Print the explicit uint256 or int256
    Long,
    /// Print the implicit uint or int
    Short,
    /// Use the source code to decide
    Preserve,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        FormatterConfig {
            line_length: 80,
            tab_width: 4,
            bracket_spacing: false,
            int_types: IntTypes::Long,
            func_attrs_with_params_multiline: true,
        }
    }
}
