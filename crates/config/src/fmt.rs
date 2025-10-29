//! Configuration specific to the `forge fmt` command and the `forge_fmt` package

use serde::{Deserialize, Serialize};

/// Contains the config and rule set
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatterConfig {
    /// Maximum line length where formatter will try to wrap the line
    pub line_length: usize,
    /// Number of spaces per indentation level. Ignored if style is Tab
    pub tab_width: usize,
    /// Style of indent
    pub style: IndentStyle,
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
    /// Style of doc comments
    pub docs_style: DocCommentStyle,
    /// Globs to ignore
    pub ignore: Vec<String>,
    /// Add new line at start and end of contract declarations
    pub contract_new_lines: bool,
    /// Sort import statements alphabetically in groups (a group is separated by a newline).
    pub sort_imports: bool,
    /// Whether to suppress spaces around the power operator (`**`).
    pub pow_no_space: bool,
    /// Style that determines if a broken list, should keep its elements together on their own
    /// line, before breaking individually.
    pub prefer_compact: PreferCompact,
    /// Keep single imports on a single line even if they exceed line length.
    pub single_line_imports: bool,
}

/// Style of integer types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntTypes {
    /// Use the type defined in the source code.
    Preserve,
    /// Print the full length `uint256` or `int256`.
    #[default]
    Long,
    /// Print the alias `uint` or `int`.
    Short,
}

/// Style of underscores in number literals
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberUnderscore {
    /// Use the underscores defined in the source code
    #[default]
    Preserve,
    /// Remove all underscores
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

/// Style of doc comments
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocCommentStyle {
    /// Preserve the source code style
    #[default]
    Preserve,
    /// Use single-line style (`///`)
    Line,
    /// Use block style (`/** .. */`)
    Block,
}

/// Style of string quotes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
    /// Use quotation mark defined in the source code.
    Preserve,
    /// Use double quotes where possible.
    #[default]
    Double,
    /// Use single quotes where possible.
    Single,
}

impl QuoteStyle {
    /// Returns the associated quotation mark character.
    pub const fn quote(self) -> Option<char> {
        match self {
            Self::Preserve => None,
            Self::Double => Some('"'),
            Self::Single => Some('\''),
        }
    }
}

/// Style of single line blocks in statements
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleLineBlockStyle {
    /// Preserve the original style
    #[default]
    Preserve,
    /// Prefer single line block when possible
    Single,
    /// Always use multiline block
    Multi,
}

/// Style of function header in case it doesn't fit
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultilineFuncHeaderStyle {
    /// Always write function parameters multiline.
    #[serde(alias = "params_first")] // alias for backwards compatibility
    ParamsAlways,
    /// Write function parameters multiline first when there is more than one param.
    ParamsFirstMulti,
    /// Write function attributes multiline first.
    #[default]
    AttributesFirst,
    /// If function params or attrs are multiline.
    /// split the rest
    All,
    /// Same as `All` but writes function params multiline even when there is a single param.
    AllParams,
}

impl MultilineFuncHeaderStyle {
    pub fn all(&self) -> bool {
        matches!(self, Self::All | Self::AllParams)
    }

    pub fn params_first(&self) -> bool {
        matches!(self, Self::ParamsAlways | Self::ParamsFirstMulti)
    }

    pub fn attrib_first(&self) -> bool {
        matches!(self, Self::AttributesFirst)
    }
}

/// Style that determines if a broken list, should keep its elements together on their own line,
/// before breaking individually.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferCompact {
    /// All elements are preferred consistent.
    None,
    /// Calls are preferred compact. Events and errors break consistently.
    Calls,
    /// Events are preferred compact. Calls and errors break consistently.
    Events,
    /// Errors are preferred compact. Calls and events break consistently.
    Errors,
    /// Events and errors are preferred compact. Calls break consistently.
    EventsErrors,
    /// All elements are preferred compact.
    #[default]
    All,
}

impl PreferCompact {
    pub fn calls(&self) -> bool {
        matches!(self, Self::All | Self::Calls)
    }

    pub fn events(&self) -> bool {
        matches!(self, Self::All | Self::Events | Self::EventsErrors)
    }

    pub fn errors(&self) -> bool {
        matches!(self, Self::All | Self::Errors | Self::EventsErrors)
    }
}

/// Style of indent
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndentStyle {
    #[default]
    Space,
    Tab,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            line_length: 120,
            tab_width: 4,
            style: IndentStyle::Space,
            bracket_spacing: false,
            int_types: IntTypes::default(),
            multiline_func_header: MultilineFuncHeaderStyle::default(),
            quote_style: QuoteStyle::default(),
            number_underscore: NumberUnderscore::default(),
            hex_underscore: HexUnderscore::default(),
            single_line_statement_blocks: SingleLineBlockStyle::default(),
            override_spacing: false,
            wrap_comments: false,
            ignore: vec![],
            contract_new_lines: false,
            sort_imports: false,
            pow_no_space: false,
            prefer_compact: PreferCompact::default(),
            docs_style: DocCommentStyle::default(),
            single_line_imports: false,
        }
    }
}
