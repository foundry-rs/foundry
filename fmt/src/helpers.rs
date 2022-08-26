use crate::{
    inline_config::{InlineConfig, InvalidInlineConfigItem},
    Comments, Formatter, FormatterConfig, FormatterError, Visitable,
};
use itertools::Itertools;
use solang_parser::pt::*;

/// Result of parsing the source code
#[derive(Debug)]
pub struct Parsed<'a> {
    /// The original source code
    pub src: &'a str,
    /// The Parse Tree via [`solang`]
    pub pt: SourceUnit,
    /// Parsed comments
    pub comments: Comments,
    /// Parsed inline config
    pub inline_config: InlineConfig,
    /// Invalid inline config items parsed
    pub invalid_inline_config_items: Vec<(Loc, InvalidInlineConfigItem)>,
}

/// Parse source code
pub fn parse(src: &str) -> Result<Parsed, Vec<solang_parser::diagnostics::Diagnostic>> {
    let (pt, comments) = solang_parser::parse(src, 0)?;
    let comments = Comments::new(comments, src);
    let (inline_config_items, invalid_inline_config_items): (Vec<_>, Vec<_>) =
        comments.parse_inline_config_items().partition_result();
    let inline_config = InlineConfig::new(inline_config_items, src);
    Ok(Parsed { src, pt, comments, inline_config, invalid_inline_config_items })
}

/// Format parsed code
pub fn format<W: std::fmt::Write>(
    writer: &mut W,
    mut parsed: Parsed,
    config: FormatterConfig,
) -> Result<(), FormatterError> {
    let mut formatter =
        Formatter::new(writer, parsed.src, parsed.comments, parsed.inline_config, config);
    parsed.pt.visit(&mut formatter)
}

/// Parse and format a string with default settings
pub fn fmt(src: &str) -> Result<String, FormatterError> {
    let parsed = parse(src).map_err(|_| FormatterError::Fmt(std::fmt::Error))?;

    let mut output = String::new();
    format(&mut output, parsed, FormatterConfig::default())?;

    Ok(output)
}
