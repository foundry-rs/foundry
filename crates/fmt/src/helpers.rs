use crate::{
    inline_config::{InlineConfig, InvalidInlineConfigItem},
    Comments, Formatter, FormatterConfig, FormatterError, Visitable,
};
use ariadne::{Color, Fmt, Label, Report, ReportKind, Source};
use itertools::Itertools;
use solang_parser::{diagnostics::Diagnostic, pt::*};
use std::{fmt::Write, path::Path};

/// Result of parsing the source code
#[derive(Debug)]
pub struct Parsed<'a> {
    /// The original source code.
    pub src: &'a str,
    /// The parse tree.
    pub pt: SourceUnit,
    /// Parsed comments.
    pub comments: Comments,
    /// Parsed inline config.
    pub inline_config: InlineConfig,
    /// Invalid inline config items parsed.
    pub invalid_inline_config_items: Vec<(Loc, InvalidInlineConfigItem)>,
}

/// Parse source code.
pub fn parse(src: &str) -> Result<Parsed<'_>, FormatterError> {
    parse_raw(src).map_err(|diag| FormatterError::Parse(src.to_string(), None, diag))
}

/// Parse source code with a path for diagnostics.
pub fn parse2<'s>(src: &'s str, path: Option<&Path>) -> Result<Parsed<'s>, FormatterError> {
    parse_raw(src)
        .map_err(|diag| FormatterError::Parse(src.to_string(), path.map(ToOwned::to_owned), diag))
}

/// Parse source code, returning a list of diagnostics on failure.
pub fn parse_raw(src: &str) -> Result<Parsed<'_>, Vec<Diagnostic>> {
    let (pt, comments) = solang_parser::parse(src, 0)?;
    let comments = Comments::new(comments, src);
    let (inline_config_items, invalid_inline_config_items): (Vec<_>, Vec<_>) =
        comments.parse_inline_config_items().partition_result();
    let inline_config = InlineConfig::new(inline_config_items, src);
    Ok(Parsed { src, pt, comments, inline_config, invalid_inline_config_items })
}

/// Format parsed code
pub fn format_to<W: Write>(
    writer: W,
    mut parsed: Parsed<'_>,
    config: FormatterConfig,
) -> Result<(), FormatterError> {
    trace!(?parsed, ?config, "Formatting");
    let mut formatter =
        Formatter::new(writer, parsed.src, parsed.comments, parsed.inline_config, config);
    parsed.pt.visit(&mut formatter)
}

/// Parse and format a string with default settings
pub fn format(src: &str) -> Result<String, FormatterError> {
    let parsed = parse(src)?;

    let mut output = String::new();
    format_to(&mut output, parsed, FormatterConfig::default())?;

    Ok(output)
}

/// Converts the start offset of a `Loc` to `(line, col)`
pub fn offset_to_line_column(content: &str, start: usize) -> (usize, usize) {
    debug_assert!(content.len() > start);

    // first line is `1`
    let mut line_counter = 1;
    for (offset, c) in content.chars().enumerate() {
        if c == '\n' {
            line_counter += 1;
        }
        if offset > start {
            return (line_counter, offset - start)
        }
    }

    unreachable!("content.len() > start")
}

/// Formats parser diagnostics
pub fn format_diagnostics_report(
    content: &str,
    path: Option<&Path>,
    diagnostics: &[Diagnostic],
) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let filename =
        path.map(|p| p.file_name().unwrap().to_string_lossy().to_string()).unwrap_or_default();
    let mut s = Vec::new();
    for diag in diagnostics {
        let (start, end) = (diag.loc.start(), diag.loc.end());
        let mut report = Report::build(ReportKind::Error, &filename, start)
            .with_message(format!("{:?}", diag.ty))
            .with_label(
                Label::new((&filename, start..end))
                    .with_color(Color::Red)
                    .with_message(format!("{}", diag.message.as_str().fg(Color::Red))),
            );

        for note in &diag.notes {
            report = report.with_note(&note.message);
        }

        report.finish().write((&filename, Source::from(content)), &mut s).unwrap();
    }
    String::from_utf8(s).unwrap()
}

pub fn import_path_string(path: &ImportPath) -> String {
    match path {
        ImportPath::Filename(s) => s.string.clone(),
        ImportPath::Path(p) => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // <https://github.com/foundry-rs/foundry/issues/6816>
    #[test]
    fn test_interface_format() {
        let s = "interface I {\n    function increment() external;\n    function number() external view returns (uint256);\n    function setNumber(uint256 newNumber) external;\n}";
        let _formatted = format(s).unwrap();
    }
}
