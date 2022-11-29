use crate::{
    comments::CommentWithMetadata,
    inline_config::{InlineConfig, InvalidInlineConfigItem},
    solang_ext::LineOfCode,
    Comments, Formatter, FormatterConfig, FormatterError, Visitable,
};
use itertools::{Either, Itertools};
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

/// TODO: docs
pub struct LinedItems<'a, V: Visitable + LineOfCode> {
    items: Vec<&'a mut V>,
    loc: Loc,
    can_extend_comments: bool,
}

impl<'a, V: Visitable + LineOfCode> LinedItems<'a, V> {
    pub fn new<I>(items: I, loc: Loc, can_extend_comments: bool) -> Self
    where
        I: Iterator<Item = &'a mut V>,
    {
        let mut items = items.collect::<Vec<_>>();
        items.reverse();
        Self { items, loc, can_extend_comments }
    }

    fn next_comment(&self, comments: &'a Comments) -> Option<&'a CommentWithMetadata> {
        comments.iter().next().filter(|comment| comment.loc.end() < self.loc.end())
    }

    pub fn last_byte_written(&self, comments: &Comments) -> Option<usize> {
        let loc = match (self.next_comment(comments), self.items.last()) {
            (Some(comment), Some(item)) => comment.loc.min(item.loc()),
            (None, Some(item)) => item.loc(),
            (Some(comment), None) => comment.loc,
            (None, None) => return None,
        };
        Some(loc.start())
    }

    pub fn last(&self) -> Option<&&mut V> {
        self.items.last()
    }

    pub fn next(
        &mut self,
        comments: &mut Comments,
    ) -> Option<Either<CommentWithMetadata, &'a mut V>> {
        match (self.next_comment(comments), self.items.last()) {
            (Some(comment), Some(item)) => {
                if comment.loc < item.loc() {
                    Some(Either::Left(self.extend_comment(comments, Some(item.loc()))))
                } else {
                    Some(Either::Right(self.items.pop()?))
                }
            }
            (Some(_comment), None) => Some(Either::Left(self.extend_comment(comments, None))),
            (None, Some(_item)) => Some(Either::Right(self.items.pop()?)),
            _ => None,
        }
    }

    fn extend_comment(
        &self,
        comments: &mut Comments,
        next_item_loc: Option<Loc>,
    ) -> CommentWithMetadata {
        let mut result = comments.pop().unwrap();
        if !self.can_extend_comments {
            return result
        }

        while let Some(comment) = self.next_comment(comments) {
            if next_item_loc.map(|loc| comment.loc >= loc).unwrap_or_default() ||
                !result.can_be_extended(comment)
            {
                return result
            }

            result = result.extend(&comments.pop().unwrap())
        }
        result
    }
}
