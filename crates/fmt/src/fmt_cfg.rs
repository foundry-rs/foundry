use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatterConfig {
    pub line_length: usize,
    pub tab_width: usize,
    pub style: IndentStyle,
    pub bracket_spacing: bool,
    pub int_types: IntTypes,
    pub multiline_func_header: MultilineFuncHeaderStyle,
    pub quote_style: QuoteStyle,
    pub number_underscore: NumberUnderscore,
    pub hex_underscore: HexUnderscore,
    pub single_line_statement_blocks: SingleLineBlockStyle,
    pub override_spacing: bool,
    pub wrap_comments: bool,
    pub ignore: Vec<String>,
    pub contract_new_lines: bool,
    pub sort_imports: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntTypes {
    Long,
    Short,
    Preserve,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberUnderscore {
    Preserve,
    #[default]
    Remove,
    Thousands,
}

impl NumberUnderscore {
    #[inline]
    pub fn is_preserve(self) -> bool {
        matches!(self, Self::Preserve)
    }
    #[inline]
    pub fn is_remove(self) -> bool {
        matches!(self, Self::Remove)
    }
    #[inline]
    pub fn is_thousands(self) -> bool {
        matches!(self, Self::Thousands)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexUnderscore {
    Preserve,
    #[default]
    Remove,
    Bytes,
}

impl HexUnderscore {
    #[inline]
    pub fn is_preserve(self) -> bool {
        matches!(self, Self::Preserve)
    }
    #[inline]
    pub fn is_remove(self) -> bool {
        matches!(self, Self::Remove)
    }
    #[inline]
    pub fn is_bytes(self) -> bool {
        matches!(self, Self::Bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStyle {
    Double,
    Single,
    Preserve,
}

impl QuoteStyle {
    pub fn quote(self) -> Option<char> {
        match self {
            Self::Double => Some('"'),
            Self::Single => Some('\''),
            Self::Preserve => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleLineBlockStyle {
    Single,
    Multi,
    Preserve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultilineFuncHeaderStyle {
    ParamsFirst,
    ParamsFirstMulti,
    AttributesFirst,
    All,
    AllParams,
}

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
