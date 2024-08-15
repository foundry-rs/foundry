//! A Solidity formatter

use crate::{
    buffer::*,
    chunk::*,
    comments::{
        CommentPosition, CommentState, CommentStringExt, CommentType, CommentWithMetadata, Comments,
    },
    format_diagnostics_report,
    helpers::import_path_string,
    macros::*,
    solang_ext::{pt::*, *},
    string::{QuoteState, QuotedStringExt},
    visit::{Visitable, Visitor},
    FormatterConfig, InlineConfig, IntTypes,
};
use alloy_primitives::Address;
use foundry_config::fmt::{HexUnderscore, MultilineFuncHeaderStyle, SingleLineBlockStyle};
use itertools::{Either, Itertools};
use solang_parser::diagnostics::Diagnostic;
use std::{fmt::Write, path::PathBuf, str::FromStr};
use thiserror::Error;

type Result<T, E = FormatterError> = std::result::Result<T, E>;

/// A custom Error thrown by the Formatter
#[derive(Debug, Error)]
pub enum FormatterError {
    /// Error thrown by `std::fmt::Write` interfaces
    #[error(transparent)]
    Fmt(#[from] std::fmt::Error),
    /// Encountered invalid parse tree item.
    #[error("encountered invalid parse tree item at {0:?}")]
    InvalidParsedItem(Loc),
    /// Failed to parse the source code
    #[error("failed to parse file:\n{}", format_diagnostics_report(_0, _1.as_deref(), _2))]
    Parse(String, Option<PathBuf>, Vec<Diagnostic>),
    /// All other errors
    #[error(transparent)]
    Custom(Box<dyn std::error::Error + Send + Sync>),
}

impl FormatterError {
    fn fmt() -> Self {
        Self::Fmt(std::fmt::Error)
    }

    fn custom(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Custom(Box::new(err))
    }
}

#[allow(unused_macros)]
macro_rules! format_err {
    ($msg:literal $(,)?) => {
        $crate::formatter::FormatterError::custom($msg.to_string())
    };
    ($err:expr $(,)?) => {
        $crate::formatter::FormatterError::custom($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::formatter::FormatterError::custom(format!($fmt, $($arg)*))
    };
}

#[allow(unused_macros)]
macro_rules! bail {
    ($msg:literal $(,)?) => {
        return Err($crate::formatter::format_err!($msg))
    };
    ($err:expr $(,)?) => {
        return Err($err)
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err($crate::formatter::format_err!($fmt, $(arg)*))
    };
}

// TODO: store context entities as references without copying
/// Current context of the Formatter (e.g. inside Contract or Function definition)
#[derive(Debug, Default)]
struct Context {
    contract: Option<ContractDefinition>,
    function: Option<FunctionDefinition>,
    if_stmt_single_line: Option<bool>,
}

impl Context {
    /// Returns true if the current function context is the constructor
    pub(crate) fn is_constructor_function(&self) -> bool {
        self.function.as_ref().map_or(false, |f| matches!(f.ty, FunctionTy::Constructor))
    }
}

/// A Solidity formatter
#[derive(Debug)]
pub struct Formatter<'a, W> {
    buf: FormatBuffer<W>,
    source: &'a str,
    config: FormatterConfig,
    temp_bufs: Vec<FormatBuffer<String>>,
    context: Context,
    comments: Comments,
    inline_config: InlineConfig,
}

impl<'a, W: Write> Formatter<'a, W> {
    pub fn new(
        w: W,
        source: &'a str,
        comments: Comments,
        inline_config: InlineConfig,
        config: FormatterConfig,
    ) -> Self {
        Self {
            buf: FormatBuffer::new(w, config.tab_width),
            source,
            config,
            temp_bufs: Vec::new(),
            context: Context::default(),
            comments,
            inline_config,
        }
    }

    /// Get the Write interface of the current temp buffer or the underlying Write
    fn buf(&mut self) -> &mut dyn Write {
        match &mut self.temp_bufs[..] {
            [] => &mut self.buf as &mut dyn Write,
            [.., buf] => buf as &mut dyn Write,
        }
    }

    /// Casts the current writer `w` as a `String` reference. Should only be used for debugging.
    #[allow(dead_code)]
    unsafe fn buf_contents(&self) -> &String {
        *(&self.buf.w as *const W as *const &mut String)
    }

    /// Casts the current `W` writer or the current temp buffer as a `String` reference.
    /// Should only be used for debugging.
    #[allow(dead_code)]
    unsafe fn temp_buf_contents(&self) -> &String {
        match &self.temp_bufs[..] {
            [] => self.buf_contents(),
            [.., buf] => &buf.w,
        }
    }

    buf_fn! { fn indent(&mut self, delta: usize) }
    buf_fn! { fn dedent(&mut self, delta: usize) }
    buf_fn! { fn start_group(&mut self) }
    buf_fn! { fn end_group(&mut self) }
    buf_fn! { fn create_temp_buf(&self) -> FormatBuffer<String> }
    buf_fn! { fn restrict_to_single_line(&mut self, restricted: bool) }
    buf_fn! { fn current_line_len(&self) -> usize }
    buf_fn! { fn total_indent_len(&self) -> usize }
    buf_fn! { fn is_beginning_of_line(&self) -> bool }
    buf_fn! { fn last_char(&self) -> Option<char> }
    buf_fn! { fn last_indent_group_skipped(&self) -> bool }
    buf_fn! { fn set_last_indent_group_skipped(&mut self, skip: bool) }
    buf_fn! { fn write_raw(&mut self, s: impl AsRef<str>) -> std::fmt::Result }

    /// Do the callback within the context of a temp buffer
    fn with_temp_buf(
        &mut self,
        mut fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<FormatBuffer<String>> {
        self.temp_bufs.push(self.create_temp_buf());
        let res = fun(self);
        let out = self.temp_bufs.pop().unwrap();
        res?;
        Ok(out)
    }

    /// Does the next written character require whitespace before
    fn next_char_needs_space(&self, next_char: char) -> bool {
        if self.is_beginning_of_line() {
            return false
        }
        let last_char =
            if let Some(last_char) = self.last_char() { last_char } else { return false };
        if last_char.is_whitespace() || next_char.is_whitespace() {
            return false
        }
        match last_char {
            '{' => match next_char {
                '{' | '[' | '(' => false,
                '/' => true,
                _ => self.config.bracket_spacing,
            },
            '(' | '.' | '[' => matches!(next_char, '/'),
            '/' => true,
            _ => match next_char {
                '}' => self.config.bracket_spacing,
                ')' | ',' | '.' | ';' | ']' => false,
                _ => true,
            },
        }
    }

    /// Is length of the `text` with respect to already written line <= `config.line_length`
    fn will_it_fit(&self, text: impl AsRef<str>) -> bool {
        let text = text.as_ref();
        if text.is_empty() {
            return true
        }
        if text.contains('\n') {
            return false
        }
        let space: usize = self.next_char_needs_space(text.chars().next().unwrap()).into();
        self.config.line_length >=
            self.total_indent_len()
                .saturating_add(self.current_line_len())
                .saturating_add(text.chars().count() + space)
    }

    /// Write empty brackets with respect to `config.bracket_spacing` setting:
    /// `"{ }"` if `true`, `"{}"` if `false`
    fn write_empty_brackets(&mut self) -> Result<()> {
        let brackets = if self.config.bracket_spacing { "{ }" } else { "{}" };
        write_chunk!(self, "{brackets}")?;
        Ok(())
    }

    /// Write semicolon to the buffer
    fn write_semicolon(&mut self) -> Result<()> {
        write!(self.buf(), ";")?;
        Ok(())
    }

    /// Write whitespace separator to the buffer
    /// `"\n"` if `multiline` is `true`, `" "` if `false`
    fn write_whitespace_separator(&mut self, multiline: bool) -> Result<()> {
        if !self.is_beginning_of_line() {
            write!(self.buf(), "{}", if multiline { "\n" } else { " " })?;
        }
        Ok(())
    }

    /// Write new line with preserved `last_indent_group_skipped` flag
    fn write_preserved_line(&mut self) -> Result<()> {
        let last_indent_group_skipped = self.last_indent_group_skipped();
        writeln!(self.buf())?;
        self.set_last_indent_group_skipped(last_indent_group_skipped);
        Ok(())
    }

    /// Write unformatted src and comments for given location.
    fn write_raw_src(&mut self, loc: Loc) -> Result<()> {
        let disabled_stmts_src = String::from_utf8(self.source.as_bytes()[loc.range()].to_vec())
            .map_err(FormatterError::custom)?;
        self.write_raw(disabled_stmts_src.trim_end())?;
        self.write_whitespace_separator(true)?;
        // Remove comments as they're already included in disabled src.
        let _ = self.comments.remove_all_comments_before(loc.end());
        Ok(())
    }

    /// Returns number of blank lines in source between two byte indexes
    fn blank_lines(&self, start: usize, end: usize) -> usize {
        // because of sorting import statements, start can be greater than end
        if start > end {
            return 0
        }
        self.source[start..end].trim_comments().matches('\n').count()
    }

    /// Get the byte offset of the next line
    fn find_next_line(&self, byte_offset: usize) -> Option<usize> {
        let mut iter = self.source[byte_offset..].char_indices();
        while let Some((_, ch)) = iter.next() {
            match ch {
                '\n' => return iter.next().map(|(idx, _)| byte_offset + idx),
                '\r' => {
                    return iter.next().and_then(|(idx, ch)| match ch {
                        '\n' => iter.next().map(|(idx, _)| byte_offset + idx),
                        _ => Some(byte_offset + idx),
                    })
                }
                _ => {}
            }
        }
        None
    }

    /// Find the next instance of the character in source excluding comments
    fn find_next_in_src(&self, byte_offset: usize, needle: char) -> Option<usize> {
        self.source[byte_offset..]
            .comment_state_char_indices()
            .position(|(state, _, ch)| needle == ch && state == CommentState::None)
            .map(|p| byte_offset + p)
    }

    /// Find the start of the next instance of a slice in source
    fn find_next_str_in_src(&self, byte_offset: usize, needle: &str) -> Option<usize> {
        let subset = &self.source[byte_offset..];
        needle.chars().next().and_then(|first_char| {
            subset
                .comment_state_char_indices()
                .position(|(state, idx, ch)| {
                    first_char == ch &&
                        state == CommentState::None &&
                        idx + needle.len() <= subset.len() &&
                        subset[idx..idx + needle.len()] == *needle
                })
                .map(|p| byte_offset + p)
        })
    }

    /// Extends the location to the next instance of a character. Returns true if the loc was
    /// extended
    fn extend_loc_until(&self, loc: &mut Loc, needle: char) -> bool {
        if let Some(end) = self.find_next_in_src(loc.end(), needle).map(|offset| offset + 1) {
            *loc = loc.with_end(end);
            true
        } else {
            false
        }
    }

    /// Return the flag whether the attempt should be made
    /// to write the block on a single line.
    /// If the block style is configured to [SingleLineBlockStyle::Preserve],
    /// lookup whether there was a newline introduced in `[start_from, end_at]` range
    /// where `end_at` is the start of the block.
    fn should_attempt_block_single_line(
        &mut self,
        stmt: &mut Statement,
        start_from: usize,
    ) -> bool {
        match self.config.single_line_statement_blocks {
            SingleLineBlockStyle::Single => true,
            SingleLineBlockStyle::Multi => false,
            SingleLineBlockStyle::Preserve => {
                let end_at = match stmt {
                    Statement::Block { statements, .. } if !statements.is_empty() => {
                        statements.first().as_ref().unwrap().loc().start()
                    }
                    Statement::Expression(loc, _) => loc.start(),
                    _ => stmt.loc().start(),
                };

                self.find_next_line(start_from).map_or(false, |loc| loc >= end_at)
            }
        }
    }

    /// Create a chunk given a string and the location information
    fn chunk_at(
        &mut self,
        byte_offset: usize,
        next_byte_offset: Option<usize>,
        needs_space: Option<bool>,
        content: impl std::fmt::Display,
    ) -> Chunk {
        Chunk {
            postfixes_before: self.comments.remove_postfixes_before(byte_offset),
            prefixes: self.comments.remove_prefixes_before(byte_offset),
            content: content.to_string(),
            postfixes: next_byte_offset
                .map(|byte_offset| self.comments.remove_postfixes_before(byte_offset))
                .unwrap_or_default(),
            needs_space,
        }
    }

    /// Create a chunk given a callback
    fn chunked(
        &mut self,
        byte_offset: usize,
        next_byte_offset: Option<usize>,
        fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<Chunk> {
        let postfixes_before = self.comments.remove_postfixes_before(byte_offset);
        let prefixes = self.comments.remove_prefixes_before(byte_offset);
        let content = self.with_temp_buf(fun)?.w;
        let postfixes = next_byte_offset
            .map(|byte_offset| self.comments.remove_postfixes_before(byte_offset))
            .unwrap_or_default();
        Ok(Chunk { postfixes_before, prefixes, content, postfixes, needs_space: None })
    }

    /// Create a chunk given a [Visitable] item
    fn visit_to_chunk(
        &mut self,
        byte_offset: usize,
        next_byte_offset: Option<usize>,
        visitable: &mut impl Visitable,
    ) -> Result<Chunk> {
        self.chunked(byte_offset, next_byte_offset, |fmt| {
            visitable.visit(fmt)?;
            Ok(())
        })
    }

    /// Transform [Visitable] items to the list of chunks
    fn items_to_chunks<'b>(
        &mut self,
        next_byte_offset: Option<usize>,
        items: impl Iterator<Item = (Loc, &'b mut (impl Visitable + 'b))> + 'b,
    ) -> Result<Vec<Chunk>> {
        let mut items = items.peekable();
        let mut out = Vec::with_capacity(items.size_hint().1.unwrap_or(0));
        while let Some((loc, item)) = items.next() {
            let chunk_next_byte_offset =
                items.peek().map(|(loc, _)| loc.start()).or(next_byte_offset);

            let chunk = if self.inline_config.is_disabled(loc) {
                // If item format is disabled, we determine last disabled line from item and create
                // chunk with raw src.
                let mut disabled_loc = loc;
                self.chunked(disabled_loc.start(), chunk_next_byte_offset, |fmt| {
                    while fmt.inline_config.is_disabled(disabled_loc) {
                        if let Some(next_line) = fmt.find_next_line(disabled_loc.end()) {
                            disabled_loc = disabled_loc.with_end(next_line);
                        } else {
                            break;
                        }
                    }
                    fmt.write_raw_src(disabled_loc)?;
                    Ok(())
                })?
            } else {
                self.visit_to_chunk(loc.start(), chunk_next_byte_offset, item)?
            };
            out.push(chunk);
        }
        Ok(out)
    }

    /// Transform [Visitable] items to a list of chunks and then sort those chunks.
    fn items_to_chunks_sorted<'b>(
        &mut self,
        next_byte_offset: Option<usize>,
        items: impl Iterator<Item = &'b mut (impl Visitable + CodeLocation + Ord + 'b)> + 'b,
    ) -> Result<Vec<Chunk>> {
        let mut items = items.peekable();
        let mut out = Vec::with_capacity(items.size_hint().1.unwrap_or(0));
        while let Some(item) = items.next() {
            let chunk_next_byte_offset =
                items.peek().map(|next| next.loc().start()).or(next_byte_offset);
            let chunk = self.visit_to_chunk(item.loc().start(), chunk_next_byte_offset, item)?;
            out.push((item, chunk));
        }
        out.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(out.into_iter().map(|(_, c)| c).collect())
    }

    /// Write a comment to the buffer formatted.
    /// WARNING: This may introduce a newline if the comment is a Line comment
    /// or if the comment are wrapped
    fn write_comment(&mut self, comment: &CommentWithMetadata, is_first: bool) -> Result<()> {
        if self.inline_config.is_disabled(comment.loc) {
            return self.write_raw_comment(comment)
        }

        match comment.position {
            CommentPosition::Prefix => self.write_prefix_comment(comment, is_first),
            CommentPosition::Postfix => self.write_postfix_comment(comment),
        }
    }

    /// Write a comment with position [CommentPosition::Prefix]
    fn write_prefix_comment(
        &mut self,
        comment: &CommentWithMetadata,
        is_first: bool,
    ) -> Result<()> {
        if !self.is_beginning_of_line() {
            self.write_preserved_line()?;
        }
        if !is_first && comment.has_newline_before {
            self.write_preserved_line()?;
        }

        if matches!(comment.ty, CommentType::DocBlock) {
            let mut lines = comment.contents().trim().lines();
            writeln!(self.buf(), "{}", comment.start_token())?;
            lines.try_for_each(|l| self.write_doc_block_line(comment, l))?;
            write!(self.buf(), " {}", comment.end_token().unwrap())?;
            self.write_preserved_line()?;
            return Ok(())
        }

        write!(self.buf(), "{}", comment.start_token())?;

        let mut wrapped = false;
        let contents = comment.contents();
        let mut lines = contents.lines().peekable();
        while let Some(line) = lines.next() {
            wrapped |= self.write_comment_line(comment, line)?;
            if lines.peek().is_some() {
                self.write_preserved_line()?;
            }
        }

        if let Some(end) = comment.end_token() {
            // Check if the end token in the original comment was on the separate line
            if !wrapped && comment.comment.lines().count() > contents.lines().count() {
                self.write_preserved_line()?;
            }
            write!(self.buf(), "{end}")?;
        }
        if self.find_next_line(comment.loc.end()).is_some() {
            self.write_preserved_line()?;
        }

        Ok(())
    }

    /// Write a comment with position [CommentPosition::Postfix]
    fn write_postfix_comment(&mut self, comment: &CommentWithMetadata) -> Result<()> {
        let indented = self.is_beginning_of_line();
        self.indented_if(indented, 1, |fmt| {
            if !indented && fmt.next_char_needs_space('/') {
                fmt.write_whitespace_separator(false)?;
            }

            write!(fmt.buf(), "{}", comment.start_token())?;
            let start_token_pos = fmt.current_line_len();

            let mut lines = comment.contents().lines().peekable();
            fmt.grouped(|fmt| {
                while let Some(line) = lines.next() {
                    fmt.write_comment_line(comment, line)?;
                    if lines.peek().is_some() {
                        fmt.write_whitespace_separator(true)?;
                    }
                }
                Ok(())
            })?;

            if let Some(end) = comment.end_token() {
                // If comment is not multiline, end token has to be aligned with the start
                if fmt.is_beginning_of_line() {
                    write!(fmt.buf(), "{}{end}", " ".repeat(start_token_pos))?;
                } else {
                    write!(fmt.buf(), "{end}")?;
                }
            }

            if comment.is_line() {
                fmt.write_whitespace_separator(true)?;
            }
            Ok(())
        })
    }

    /// Write the line of a doc block comment line
    fn write_doc_block_line(&mut self, comment: &CommentWithMetadata, line: &str) -> Result<()> {
        if line.trim().starts_with('*') {
            let line = line.trim().trim_start_matches('*');
            let needs_space = line.chars().next().map_or(false, |ch| !ch.is_whitespace());
            write!(self.buf(), " *{}", if needs_space { " " } else { "" })?;
            self.write_comment_line(comment, line)?;
            self.write_whitespace_separator(true)?;
            return Ok(())
        }

        let indent_whitespace_count = line
            .char_indices()
            .take_while(|(idx, ch)| ch.is_whitespace() && *idx <= self.buf.current_indent_len())
            .count();
        let to_skip = indent_whitespace_count - indent_whitespace_count % self.config.tab_width;
        write!(self.buf(), " *")?;
        let content = &line[to_skip..];
        if !content.trim().is_empty() {
            write!(self.buf(), " ")?;
            self.write_comment_line(comment, &line[to_skip..])?;
        }
        self.write_whitespace_separator(true)?;
        Ok(())
    }

    /// Write a comment line that might potentially overflow the maximum line length
    /// and, if configured, will be wrapped to the next line.
    fn write_comment_line(&mut self, comment: &CommentWithMetadata, line: &str) -> Result<bool> {
        if self.will_it_fit(line) || !self.config.wrap_comments {
            let start_with_ws =
                line.chars().next().map(|ch| ch.is_whitespace()).unwrap_or_default();
            if !self.is_beginning_of_line() || !start_with_ws {
                write!(self.buf(), "{line}")?;
                return Ok(false)
            }

            // if this is the beginning of the line,
            // the comment should start with at least an indent
            let indent = self.buf.current_indent_len();
            let mut chars = line
                .char_indices()
                .skip_while(|(idx, ch)| ch.is_whitespace() && *idx < indent)
                .map(|(_, ch)| ch);
            let padded = format!("{}{}", " ".repeat(indent), chars.join(""));
            self.write_raw(padded)?;
            return Ok(false)
        }

        let mut words = line.split(' ').peekable();
        while let Some(word) = words.next() {
            if self.is_beginning_of_line() {
                write!(self.buf(), "{}", word.trim_start())?;
            } else {
                self.write_raw(word)?;
            }

            if let Some(next) = words.peek() {
                if !word.is_empty() && !self.will_it_fit(next) {
                    // the next word doesn't fit on this line,
                    // write remaining words on the next
                    self.write_whitespace_separator(true)?;
                    // write newline wrap token
                    write!(self.buf(), "{}", comment.wrap_token())?;
                    self.write_comment_line(comment, &words.join(" "))?;
                    return Ok(true)
                }

                self.write_whitespace_separator(false)?;
            }
        }
        Ok(false)
    }

    /// Write a raw comment. This is like [`write_comment`](Self::write_comment) but won't do any
    /// formatting or worry about whitespace behind the comment.
    fn write_raw_comment(&mut self, comment: &CommentWithMetadata) -> Result<()> {
        self.write_raw(&comment.comment)?;
        if comment.is_line() {
            self.write_preserved_line()?;
        }
        Ok(())
    }

    // TODO handle whitespace between comments for disabled sections
    /// Write multiple comments
    fn write_comments<'b>(
        &mut self,
        comments: impl IntoIterator<Item = &'b CommentWithMetadata>,
    ) -> Result<()> {
        let mut comments = comments.into_iter().peekable();
        let mut last_byte_written = match comments.peek() {
            Some(comment) => comment.loc.start(),
            None => return Ok(()),
        };
        let mut is_first = true;
        for comment in comments {
            let unwritten_whitespace_loc =
                Loc::File(comment.loc.file_no(), last_byte_written, comment.loc.start());
            if self.inline_config.is_disabled(unwritten_whitespace_loc) {
                self.write_raw(&self.source[unwritten_whitespace_loc.range()])?;
                self.write_raw_comment(comment)?;
                last_byte_written = if comment.is_line() {
                    self.find_next_line(comment.loc.end()).unwrap_or_else(|| comment.loc.end())
                } else {
                    comment.loc.end()
                };
            } else {
                self.write_comment(comment, is_first)?;
            }
            is_first = false;
        }
        Ok(())
    }

    /// Write a postfix comments before a given location
    fn write_postfix_comments_before(&mut self, byte_end: usize) -> Result<()> {
        let comments = self.comments.remove_postfixes_before(byte_end);
        self.write_comments(&comments)
    }

    /// Write all prefix comments before a given location
    fn write_prefix_comments_before(&mut self, byte_end: usize) -> Result<()> {
        let comments = self.comments.remove_prefixes_before(byte_end);
        self.write_comments(&comments)
    }

    /// Check if a chunk will fit on the current line
    fn will_chunk_fit(&mut self, format_string: &str, chunk: &Chunk) -> Result<bool> {
        if let Some(chunk_str) = self.simulate_to_single_line(|fmt| fmt.write_chunk(chunk))? {
            Ok(self.will_it_fit(format_string.replacen("{}", &chunk_str, 1)))
        } else {
            Ok(false)
        }
    }

    /// Check if a separated list of chunks will fit on the current line
    fn are_chunks_separated_multiline<'b>(
        &mut self,
        format_string: &str,
        items: impl IntoIterator<Item = &'b Chunk>,
        separator: &str,
    ) -> Result<bool> {
        let items = items.into_iter().collect_vec();
        if let Some(chunks) = self.simulate_to_single_line(|fmt| {
            fmt.write_chunks_separated(items.iter().copied(), separator, false)
        })? {
            Ok(!self.will_it_fit(format_string.replacen("{}", &chunks, 1)))
        } else {
            Ok(true)
        }
    }

    /// Write the chunk and any surrounding comments into the buffer
    /// This will automatically add whitespace before the chunk given the rule set in
    /// `next_char_needs_space`. If the chunk does not fit on the current line it will be put on
    /// to the next line
    fn write_chunk(&mut self, chunk: &Chunk) -> Result<()> {
        // handle comments before chunk
        self.write_comments(&chunk.postfixes_before)?;
        self.write_comments(&chunk.prefixes)?;

        // trim chunk start
        let content = if chunk.content.starts_with('\n') {
            let mut chunk = chunk.content.trim_start().to_string();
            chunk.insert(0, '\n');
            chunk
        } else if chunk.content.starts_with(' ') {
            let mut chunk = chunk.content.trim_start().to_string();
            chunk.insert(0, ' ');
            chunk
        } else {
            chunk.content.clone()
        };

        if !content.is_empty() {
            // add whitespace if necessary
            let needs_space = chunk
                .needs_space
                .unwrap_or_else(|| self.next_char_needs_space(content.chars().next().unwrap()));
            if needs_space {
                if self.will_it_fit(&content) {
                    write!(self.buf(), " ")?;
                } else {
                    writeln!(self.buf())?;
                }
            }

            // write chunk
            write!(self.buf(), "{content}")?;
        }

        // write any postfix comments
        self.write_comments(&chunk.postfixes)?;

        Ok(())
    }

    /// Write chunks separated by a separator. If `multiline`, each chunk will be written to a
    /// separate line
    fn write_chunks_separated<'b>(
        &mut self,
        chunks: impl IntoIterator<Item = &'b Chunk>,
        separator: &str,
        multiline: bool,
    ) -> Result<()> {
        let mut chunks = chunks.into_iter().peekable();
        while let Some(chunk) = chunks.next() {
            let mut chunk = chunk.clone();

            // handle postfixes before and add newline if necessary
            self.write_comments(&std::mem::take(&mut chunk.postfixes_before))?;
            if multiline && !self.is_beginning_of_line() {
                writeln!(self.buf())?;
            }

            // remove postfixes so we can add separator between
            let postfixes = std::mem::take(&mut chunk.postfixes);

            self.write_chunk(&chunk)?;

            // add separator
            if chunks.peek().is_some() {
                write!(self.buf(), "{separator}")?;
                self.write_comments(&postfixes)?;
                if multiline && !self.is_beginning_of_line() {
                    writeln!(self.buf())?;
                }
            } else {
                self.write_comments(&postfixes)?;
            }
        }
        Ok(())
    }

    /// Apply the callback indented by the indent size
    fn indented(&mut self, delta: usize, fun: impl FnMut(&mut Self) -> Result<()>) -> Result<()> {
        self.indented_if(true, delta, fun)
    }

    /// Apply the callback indented by the indent size if the condition is true
    fn indented_if(
        &mut self,
        condition: bool,
        delta: usize,
        mut fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<()> {
        if condition {
            self.indent(delta);
        }
        let res = fun(self);
        if condition {
            self.dedent(delta);
        }
        res?;
        Ok(())
    }

    /// Apply the callback into an indent group. The first line of the indent group is not
    /// indented but lines thereafter are
    fn grouped(&mut self, mut fun: impl FnMut(&mut Self) -> Result<()>) -> Result<bool> {
        self.start_group();
        let res = fun(self);
        let indented = !self.last_indent_group_skipped();
        self.end_group();
        res?;
        Ok(indented)
    }

    /// Add a function context around a procedure and revert the context at the end of the procedure
    /// regardless of the response
    fn with_function_context(
        &mut self,
        context: FunctionDefinition,
        mut fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<()> {
        self.context.function = Some(context);
        let res = fun(self);
        self.context.function = None;
        res
    }

    /// Add a contract context around a procedure and revert the context at the end of the procedure
    /// regardless of the response
    fn with_contract_context(
        &mut self,
        context: ContractDefinition,
        mut fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<()> {
        self.context.contract = Some(context);
        let res = fun(self);
        self.context.contract = None;
        res
    }

    /// Create a transaction. The result of the transaction is not applied to the buffer unless
    /// `Transacton::commit` is called
    fn transact<'b>(
        &'b mut self,
        fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<Transaction<'b, 'a, W>> {
        Transaction::new(self, fun)
    }

    /// Do the callback and return the result on the buffer as a string
    fn simulate_to_string(&mut self, fun: impl FnMut(&mut Self) -> Result<()>) -> Result<String> {
        Ok(self.transact(fun)?.buffer)
    }

    /// Turn a chunk and its surrounding comments into a string
    fn chunk_to_string(&mut self, chunk: &Chunk) -> Result<String> {
        self.simulate_to_string(|fmt| fmt.write_chunk(chunk))
    }

    /// Try to create a string based on a callback. If the string does not fit on a single line
    /// this will return `None`
    fn simulate_to_single_line(
        &mut self,
        mut fun: impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<Option<String>> {
        let mut single_line = false;
        let tx = self.transact(|fmt| {
            fmt.restrict_to_single_line(true);
            single_line = match fun(fmt) {
                Ok(()) => true,
                Err(FormatterError::Fmt(_)) => false,
                Err(err) => bail!(err),
            };
            Ok(())
        })?;
        Ok(if single_line && tx.will_it_fit(&tx.buffer) { Some(tx.buffer) } else { None })
    }

    /// Try to apply a callback to a single line. If the callback cannot be applied to a single
    /// line the callback will not be applied to the buffer and `false` will be returned. Otherwise
    /// `true` will be returned
    fn try_on_single_line(&mut self, mut fun: impl FnMut(&mut Self) -> Result<()>) -> Result<bool> {
        let mut single_line = false;
        let tx = self.transact(|fmt| {
            fmt.restrict_to_single_line(true);
            single_line = match fun(fmt) {
                Ok(()) => true,
                Err(FormatterError::Fmt(_)) => false,
                Err(err) => bail!(err),
            };
            Ok(())
        })?;
        Ok(if single_line && tx.will_it_fit(&tx.buffer) {
            tx.commit()?;
            true
        } else {
            false
        })
    }

    /// Surrounds a callback with parentheses. The callback will try to be applied to a single
    /// line. If the callback cannot be applied to a single line the callback will applied to the
    /// nextline indented. The callback receives a `multiline` hint as the second argument which
    /// receives `true` in the latter case
    fn surrounded(
        &mut self,
        first: SurroundingChunk,
        last: SurroundingChunk,
        mut fun: impl FnMut(&mut Self, bool) -> Result<()>,
    ) -> Result<()> {
        let first_chunk =
            self.chunk_at(first.loc_before(), first.loc_next(), first.spaced, first.content);
        self.write_chunk(&first_chunk)?;

        let multiline = !self.try_on_single_line(|fmt| {
            fun(fmt, false)?;
            let last_chunk =
                fmt.chunk_at(last.loc_before(), last.loc_next(), last.spaced, &last.content);
            fmt.write_chunk(&last_chunk)?;
            Ok(())
        })?;

        if multiline {
            self.indented(1, |fmt| {
                fmt.write_whitespace_separator(true)?;
                let stringified = fmt.with_temp_buf(|fmt| fun(fmt, true))?.w;
                write_chunk!(fmt, "{}", stringified.trim_start())
            })?;
            if !last.content.trim_start().is_empty() {
                self.indented(1, |fmt| fmt.write_whitespace_separator(true))?;
            }
            let last_chunk =
                self.chunk_at(last.loc_before(), last.loc_next(), last.spaced, &last.content);
            self.write_chunk(&last_chunk)?;
        }

        Ok(())
    }

    /// Write each [Visitable] item on a separate line. The function will check if there are any
    /// blank lines between each visitable statement and will apply a single blank line if there
    /// exists any. The `needs_space` callback can force a newline and is given the last_item if
    /// any and the next item as arguments
    fn write_lined_visitable<'b, I, V, F>(
        &mut self,
        loc: Loc,
        items: I,
        needs_space_fn: F,
    ) -> Result<()>
    where
        I: Iterator<Item = &'b mut V> + 'b,
        V: Visitable + CodeLocation + 'b,
        F: Fn(&V, &V) -> bool,
    {
        let mut items = items.collect::<Vec<_>>();
        items.reverse();
        // get next item
        let pop_next = |fmt: &mut Self, items: &mut Vec<&'b mut V>| {
            let comment =
                fmt.comments.iter().next().filter(|comment| comment.loc.end() < loc.end());
            let item = items.last();
            if let (Some(comment), Some(item)) = (comment, item) {
                if comment.loc < item.loc() {
                    Some(Either::Left(fmt.comments.pop().unwrap()))
                } else {
                    Some(Either::Right(items.pop().unwrap()))
                }
            } else if comment.is_some() {
                Some(Either::Left(fmt.comments.pop().unwrap()))
            } else if item.is_some() {
                Some(Either::Right(items.pop().unwrap()))
            } else {
                None
            }
        };
        // get whitespace between to offsets. this needs to account for possible left over
        // semicolons which are not included in the `Loc`
        let unwritten_whitespace = |from: usize, to: usize| {
            let to = to.max(from);
            let mut loc = Loc::File(loc.file_no(), from, to);
            let src = &self.source[from..to];
            if let Some(semi) = src.find(';') {
                loc = loc.with_start(from + semi + 1);
            }
            (loc, &self.source[loc.range()])
        };

        let mut last_byte_written = match (
            self.comments.iter().next().filter(|comment| comment.loc.end() < loc.end()),
            items.last(),
        ) {
            (Some(comment), Some(item)) => comment.loc.min(item.loc()),
            (None, Some(item)) => item.loc(),
            (Some(comment), None) => comment.loc,
            (None, None) => return Ok(()),
        }
        .start();

        let mut last_loc: Option<Loc> = None;
        let mut visited_locs: Vec<Loc> = Vec::new();

        // marker for whether the next item needs additional space
        let mut needs_space = false;
        let mut last_comment = None;

        while let Some(mut line_item) = pop_next(self, &mut items) {
            let loc = line_item.as_ref().either(|c| c.loc, |i| i.loc());
            let (unwritten_whitespace_loc, unwritten_whitespace) =
                unwritten_whitespace(last_byte_written, loc.start());
            let ignore_whitespace = if self.inline_config.is_disabled(unwritten_whitespace_loc) {
                trace!("Unwritten whitespace: {unwritten_whitespace:?}");
                self.write_raw(unwritten_whitespace)?;
                true
            } else {
                false
            };
            match line_item.as_mut() {
                Either::Left(comment) => {
                    if ignore_whitespace {
                        self.write_raw_comment(comment)?;
                        if unwritten_whitespace.contains('\n') {
                            needs_space = false;
                        }
                    } else {
                        self.write_comment(comment, last_loc.is_none())?;
                        if last_loc.is_some() && comment.has_newline_before {
                            needs_space = false;
                        }
                    }
                }
                Either::Right(item) => {
                    if !ignore_whitespace {
                        self.write_whitespace_separator(true)?;
                        if let Some(mut last_loc) = last_loc {
                            // here's an edge case when we reordered items so the last_loc isn't
                            // necessarily the item that directly precedes the current item because
                            // the order might have changed, so we need to find the last item that
                            // is before the current item by checking the recorded locations
                            if let Some(last_item) = visited_locs
                                .iter()
                                .rev()
                                .find(|prev_item| prev_item.start() > last_loc.end())
                            {
                                last_loc = *last_item;
                            }

                            // The blank lines check is susceptible additional trailing new lines
                            // because the block docs can contain
                            // multiple lines, but the function def should follow directly after the
                            // block comment
                            let is_last_doc_comment = matches!(
                                last_comment,
                                Some(CommentWithMetadata { ty: CommentType::DocBlock, .. })
                            );

                            if needs_space ||
                                (!is_last_doc_comment &&
                                    self.blank_lines(last_loc.end(), loc.start()) > 1)
                            {
                                writeln!(self.buf())?;
                            }
                        }
                    }
                    if let Some(next_item) = items.last() {
                        needs_space = needs_space_fn(item, next_item);
                    }
                    trace!("Visiting {}", {
                        let n = std::any::type_name::<V>();
                        n.strip_prefix("solang_parser::pt::").unwrap_or(n)
                    });
                    item.visit(self)?;
                }
            }

            last_loc = Some(loc);
            visited_locs.push(loc);

            last_comment = None;

            last_byte_written = loc.end();
            if let Some(comment) = line_item.left() {
                if comment.is_line() {
                    last_byte_written =
                        self.find_next_line(last_byte_written).unwrap_or(last_byte_written);
                }
                last_comment = Some(comment);
            }
        }

        // write manually to avoid eof comment being detected as first
        let comments = self.comments.remove_prefixes_before(loc.end());
        for comment in comments {
            self.write_comment(&comment, false)?;
        }

        let (unwritten_src_loc, mut unwritten_whitespace) =
            unwritten_whitespace(last_byte_written, loc.end());
        if self.inline_config.is_disabled(unwritten_src_loc) {
            if unwritten_src_loc.end() == self.source.len() {
                // remove EOF line ending
                unwritten_whitespace = unwritten_whitespace
                    .strip_suffix('\n')
                    .map(|w| w.strip_suffix('\r').unwrap_or(w))
                    .unwrap_or(unwritten_whitespace);
            }
            trace!("Unwritten whitespace: {unwritten_whitespace:?}");
            self.write_raw(unwritten_whitespace)?;
        }

        Ok(())
    }

    /// Visit the right side of an assignment. The function will try to write the assignment on a
    /// single line or indented on the next line. If it can't do this it resorts to letting the
    /// expression decide how to split itself on multiple lines
    fn visit_assignment(&mut self, expr: &mut Expression) -> Result<()> {
        if self.try_on_single_line(|fmt| expr.visit(fmt))? {
            return Ok(())
        }

        self.write_postfix_comments_before(expr.loc().start())?;
        self.write_prefix_comments_before(expr.loc().start())?;

        if self.try_on_single_line(|fmt| fmt.indented(1, |fmt| expr.visit(fmt)))? {
            return Ok(())
        }

        let mut fit_on_next_line = false;
        self.indented(1, |fmt| {
            let tx = fmt.transact(|fmt| {
                writeln!(fmt.buf())?;
                fit_on_next_line = fmt.try_on_single_line(|fmt| expr.visit(fmt))?;
                Ok(())
            })?;
            if fit_on_next_line {
                tx.commit()?;
            }
            Ok(())
        })?;

        if !fit_on_next_line {
            self.indented_if(expr.is_unsplittable(), 1, |fmt| expr.visit(fmt))?;
        }

        Ok(())
    }

    /// Visit the list of comma separated items.
    /// If the prefix is not empty, then the function will write
    /// the whitespace before the parentheses (if they are required).
    fn visit_list<T>(
        &mut self,
        prefix: &str,
        items: &mut [T],
        start_offset: Option<usize>,
        end_offset: Option<usize>,
        paren_required: bool,
    ) -> Result<()>
    where
        T: Visitable + CodeLocation,
    {
        write_chunk!(self, "{}", prefix)?;
        let whitespace = if !prefix.is_empty() { " " } else { "" };
        let next_after_start_offset = items.first().map(|item| item.loc().start());
        let first_surrounding = SurroundingChunk::new("", start_offset, next_after_start_offset);
        let last_surronding = SurroundingChunk::new(")", None, end_offset);
        if items.is_empty() {
            if paren_required {
                write!(self.buf(), "{whitespace}(")?;
                self.surrounded(first_surrounding, last_surronding, |fmt, _| {
                    // write comments before the list end
                    write_chunk!(fmt, end_offset.unwrap_or_default(), "")?;
                    Ok(())
                })?;
            }
        } else {
            write!(self.buf(), "{whitespace}(")?;
            self.surrounded(first_surrounding, last_surronding, |fmt, multiline| {
                let args =
                    fmt.items_to_chunks(end_offset, items.iter_mut().map(|arg| (arg.loc(), arg)))?;
                let multiline =
                    multiline && fmt.are_chunks_separated_multiline("{}", &args, ",")?;
                fmt.write_chunks_separated(&args, ",", multiline)?;
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Visit the block item. Attempt to write it on the single
    /// line if requested. Surround by curly braces and indent
    /// each line otherwise. Returns `true` if the block fit
    /// on a single line
    fn visit_block<T>(
        &mut self,
        loc: Loc,
        statements: &mut [T],
        attempt_single_line: bool,
        attempt_omit_braces: bool,
    ) -> Result<bool>
    where
        T: Visitable + CodeLocation,
    {
        if attempt_single_line && statements.len() == 1 {
            let fits_on_single = self.try_on_single_line(|fmt| {
                if !attempt_omit_braces {
                    write!(fmt.buf(), "{{ ")?;
                }
                statements.first_mut().unwrap().visit(fmt)?;
                if !attempt_omit_braces {
                    write!(fmt.buf(), " }}")?;
                }
                Ok(())
            })?;

            if fits_on_single {
                return Ok(true)
            }
        }

        // Determine if any of start / end of the block is disabled and block lines boundaries.
        let is_start_disabled = self.inline_config.is_disabled(loc.with_end(loc.start()));
        let is_end_disabled = self.inline_config.is_disabled(loc.with_start(loc.end()));
        let end_of_first_line = self.find_next_line(loc.start()).unwrap_or_default();
        let end_of_last_line = self.find_next_line(loc.end()).unwrap_or_default();

        // Write first line of the block:
        // - as it is until the end of line, if format disabled
        // - start block if line formatted
        if is_start_disabled {
            self.write_raw_src(loc.with_end(end_of_first_line))?;
        } else {
            write_chunk!(self, "{{")?;
        }

        // Write comments and close block if no statement.
        if statements.is_empty() {
            self.indented(1, |fmt| {
                fmt.write_prefix_comments_before(loc.end())?;
                fmt.write_postfix_comments_before(loc.end())?;
                Ok(())
            })?;

            write_chunk!(self, "}}")?;
            return Ok(true)
        }

        // Determine writable statements by excluding statements from disabled start / end lines.
        // We check the position of last statement from first line (if disabled) and position of
        // first statement from last line (if disabled) and slice accordingly.
        let writable_statments = match (
            statements.iter().rposition(|stmt| {
                is_start_disabled &&
                    self.find_next_line(stmt.loc().end()).unwrap_or_default() ==
                        end_of_first_line
            }),
            statements.iter().position(|stmt| {
                is_end_disabled &&
                    self.find_next_line(stmt.loc().end()).unwrap_or_default() == end_of_last_line
            }),
        ) {
            // We have statements on both disabled start / end lines.
            (Some(start), Some(end)) => {
                if start == end || start + 1 == end {
                    None
                } else {
                    Some(&mut statements[start + 1..end])
                }
            }
            // We have statements only on disabled start line.
            (Some(start), None) => {
                if start + 1 == statements.len() {
                    None
                } else {
                    Some(&mut statements[start + 1..])
                }
            }
            // We have statements only on disabled end line.
            (None, Some(end)) => {
                if end == 0 {
                    None
                } else {
                    Some(&mut statements[..end])
                }
            }
            // No statements on disabled start / end line.
            (None, None) => Some(statements),
        };

        // Write statements that are not on any disabled first / last block line.
        let mut statements_loc = loc;
        if let Some(writable_statements) = writable_statments {
            if let Some(first_statement) = writable_statements.first() {
                statements_loc = statements_loc.with_start(first_statement.loc().start());
                self.write_whitespace_separator(true)?;
                self.write_postfix_comments_before(statements_loc.start())?;
            }
            // If last line is disabled then statements location ends where last block line starts.
            if is_end_disabled {
                if let Some(last_statement) = writable_statements.last() {
                    statements_loc = statements_loc.with_end(
                        self.find_next_line(last_statement.loc().end()).unwrap_or_default(),
                    );
                }
            }
            self.indented(1, |fmt| {
                fmt.write_lined_visitable(
                    statements_loc,
                    writable_statements.iter_mut(),
                    |_, _| false,
                )?;
                Ok(())
            })?;
            self.write_whitespace_separator(true)?;
        }

        // Write last line of the block:
        // - as it is from where statements location ends until the end of last line, if format
        // disabled
        // - close block if line formatted
        if is_end_disabled {
            self.write_raw_src(loc.with_start(statements_loc.end()).with_end(end_of_last_line))?;
        } else {
            if end_of_first_line != end_of_last_line {
                self.write_whitespace_separator(true)?;
            }
            write_chunk!(self, loc.end(), "}}")?;
        }

        Ok(false)
    }

    /// Visit statement as `Statement::Block`.
    fn visit_stmt_as_block(
        &mut self,
        stmt: &mut Statement,
        attempt_single_line: bool,
    ) -> Result<bool> {
        match stmt {
            Statement::Block { loc, statements, .. } => {
                self.visit_block(*loc, statements, attempt_single_line, true)
            }
            _ => self.visit_block(stmt.loc(), &mut [stmt], attempt_single_line, true),
        }
    }

    /// Visit the generic member access expression and
    /// attempt flatten it by checking if the inner expression
    /// matches a given member access variant.
    fn visit_member_access<'b, T, M>(
        &mut self,
        expr: &'b mut Box<T>,
        ident: &mut Identifier,
        mut matcher: M,
    ) -> Result<()>
    where
        T: CodeLocation + Visitable,
        M: FnMut(&mut Self, &'b mut Box<T>) -> Result<Option<(&'b mut Box<T>, &'b mut Identifier)>>,
    {
        let chunk_member_access = |fmt: &mut Self, ident: &mut Identifier, expr: &mut Box<T>| {
            fmt.chunked(ident.loc.start(), Some(expr.loc().start()), |fmt| ident.visit(fmt))
        };

        let mut chunks: Vec<Chunk> = vec![chunk_member_access(self, ident, expr)?];
        let mut remaining = expr;
        while let Some((inner_expr, inner_ident)) = matcher(self, remaining)? {
            chunks.push(chunk_member_access(self, inner_ident, inner_expr)?);
            remaining = inner_expr;
        }

        chunks.reverse();
        chunks.iter_mut().for_each(|chunk| chunk.content.insert(0, '.'));

        if !self.try_on_single_line(|fmt| fmt.write_chunks_separated(&chunks, "", false))? {
            self.grouped(|fmt| fmt.write_chunks_separated(&chunks, "", true))?;
        }
        Ok(())
    }

    /// Visit the yul string with an optional identifier.
    /// If the identifier is present, write the value in the format `<val>:<ident>`.
    ///
    /// Ref: <https://docs.soliditylang.org/en/v0.8.15/yul.html#variable-declarations>
    fn visit_yul_string_with_ident(
        &mut self,
        loc: Loc,
        val: &str,
        ident: &mut Option<Identifier>,
    ) -> Result<()> {
        let ident =
            if let Some(ident) = ident { format!(":{}", ident.name) } else { String::new() };
        write_chunk!(self, loc.start(), loc.end(), "{val}{ident}")?;
        Ok(())
    }

    /// Format a quoted string as `prefix"string"` where the quote character is handled
    /// by the configuration `quote_style`
    fn quote_str(&self, loc: Loc, prefix: Option<&str>, string: &str) -> String {
        let get_og_quote = || {
            self.source[loc.range()]
                .quote_state_char_indices()
                .find_map(
                    |(state, _, ch)| {
                        if matches!(state, QuoteState::Opening(_)) {
                            Some(ch)
                        } else {
                            None
                        }
                    },
                )
                .expect("Could not find quote character for quoted string")
        };
        let mut quote = self.config.quote_style.quote().unwrap_or_else(get_og_quote);
        let mut quoted = format!("{quote}{string}{quote}");
        if !quoted.is_quoted() {
            quote = get_og_quote();
            quoted = format!("{quote}{string}{quote}");
        }
        let prefix = prefix.unwrap_or("");
        format!("{prefix}{quoted}")
    }

    /// Write a quoted string. See `Formatter::quote_str` for more information
    fn write_quoted_str(&mut self, loc: Loc, prefix: Option<&str>, string: &str) -> Result<()> {
        write_chunk!(self, loc.start(), loc.end(), "{}", self.quote_str(loc, prefix, string))
    }

    /// Write and format numbers. This will fix underscores as well as remove unnecessary 0's and
    /// exponents
    fn write_num_literal(
        &mut self,
        loc: Loc,
        value: &str,
        fractional: Option<&str>,
        exponent: &str,
        unit: &mut Option<Identifier>,
    ) -> Result<()> {
        let config = self.config.number_underscore;

        // get source if we preserve underscores
        let (value, fractional, exponent) = if config.is_preserve() {
            let source = &self.source[loc.start()..loc.end()];
            // Strip unit
            let (source, _) = source.split_once(' ').unwrap_or((source, ""));
            let (val, exp) = source.split_once(['e', 'E']).unwrap_or((source, ""));
            let (val, fract) =
                val.split_once('.').map(|(val, fract)| (val, Some(fract))).unwrap_or((val, None));
            (
                val.trim().to_string(),
                fract.map(|fract| fract.trim().to_string()),
                exp.trim().to_string(),
            )
        } else {
            // otherwise strip underscores
            (
                value.trim().replace('_', ""),
                fractional.map(|fract| fract.trim().replace('_', "")),
                exponent.trim().replace('_', ""),
            )
        };

        // strip any padded 0's
        let val = value.trim_start_matches('0');
        let fract = fractional.as_ref().map(|fract| fract.trim_end_matches('0'));
        let (exp_sign, mut exp) = if let Some(exp) = exponent.strip_prefix('-') {
            ("-", exp)
        } else {
            ("", exponent.as_str())
        };
        exp = exp.trim().trim_start_matches('0');

        let add_underscores = |string: &str, reversed: bool| -> String {
            if !config.is_thousands() || string.len() < 5 {
                return string.to_string()
            }
            if reversed {
                Box::new(string.as_bytes().chunks(3)) as Box<dyn Iterator<Item = &[u8]>>
            } else {
                Box::new(string.as_bytes().rchunks(3).rev()) as Box<dyn Iterator<Item = &[u8]>>
            }
            .map(|chunk| std::str::from_utf8(chunk).expect("valid utf8 content."))
            .collect::<Vec<_>>()
            .join("_")
        };

        let mut out = String::new();
        if val.is_empty() {
            out.push('0');
        } else {
            out.push_str(&add_underscores(val, false));
        }
        if let Some(fract) = fract {
            out.push('.');
            if fract.is_empty() {
                out.push('0');
            } else {
                // TODO re-enable me on the next solang-parser v0.1.18
                // currently disabled because of the following bug
                // https://github.com/hyperledger-labs/solang/pull/954
                // out.push_str(&add_underscores(fract, true));
                out.push_str(fract)
            }
        }
        if !exp.is_empty() {
            out.push('e');
            out.push_str(exp_sign);
            out.push_str(&add_underscores(exp, false));
        }

        write_chunk!(self, loc.start(), loc.end(), "{out}")?;
        self.write_unit(unit)
    }

    /// Write and hex literals according to the configuration.
    fn write_hex_literal(&mut self, lit: &HexLiteral) -> Result<()> {
        let HexLiteral { loc, hex } = lit;
        match self.config.hex_underscore {
            HexUnderscore::Remove => self.write_quoted_str(*loc, Some("hex"), hex),
            HexUnderscore::Preserve => {
                let quote = &self.source[loc.start()..loc.end()].trim_start_matches("hex");
                // source is always quoted so we remove the quotes first so we can adhere to the
                // configured quoting style
                let hex = &quote[1..quote.len() - 1];
                self.write_quoted_str(*loc, Some("hex"), hex)
            }
            HexUnderscore::Bytes => {
                // split all bytes
                let hex = hex
                    .chars()
                    .chunks(2)
                    .into_iter()
                    .map(|chunk| chunk.collect::<String>())
                    .collect::<Vec<_>>()
                    .join("_");
                self.write_quoted_str(*loc, Some("hex"), &hex)
            }
        }
    }

    /// Write built-in unit.
    fn write_unit(&mut self, unit: &mut Option<Identifier>) -> Result<()> {
        if let Some(unit) = unit {
            write_chunk!(self, unit.loc.start(), unit.loc.end(), "{}", unit.name)?;
        }
        Ok(())
    }

    /// Write the function header
    fn write_function_header(
        &mut self,
        func: &mut FunctionDefinition,
        body_loc: Option<Loc>,
        header_multiline: bool,
    ) -> Result<bool> {
        let func_name = if let Some(ident) = &func.name {
            format!("{} {}", func.ty, ident.name)
        } else {
            func.ty.to_string()
        };

        // calculate locations of chunk groups
        let attrs_loc = func.attributes.first().map(|attr| attr.loc());
        let returns_loc = func.returns.first().map(|param| param.0);

        let params_next_offset = attrs_loc
            .as_ref()
            .or(returns_loc.as_ref())
            .or(body_loc.as_ref())
            .map(|loc| loc.start());
        let attrs_end = returns_loc.as_ref().or(body_loc.as_ref()).map(|loc| loc.start());
        let returns_end = body_loc.as_ref().map(|loc| loc.start());

        let mut params_multiline = false;

        let params_loc = {
            let mut loc = func.loc.with_end(func.loc.start());
            self.extend_loc_until(&mut loc, ')');
            loc
        };
        let params_disabled = self.inline_config.is_disabled(params_loc);
        if params_disabled {
            let chunk = self.chunked(func.loc.start(), None, |fmt| fmt.visit_source(params_loc))?;
            params_multiline = chunk.content.contains('\n');
            self.write_chunk(&chunk)?;
        } else {
            let first_surrounding = SurroundingChunk::new(
                format!("{func_name}("),
                Some(func.loc.start()),
                Some(
                    func.params
                        .first()
                        .map(|param| param.0.start())
                        .unwrap_or_else(|| params_loc.end()),
                ),
            );
            self.surrounded(
                first_surrounding,
                SurroundingChunk::new(")", None, params_next_offset),
                |fmt, multiline| {
                    let params = fmt.items_to_chunks(
                        params_next_offset,
                        func.params
                            .iter_mut()
                            .filter_map(|(loc, param)| param.as_mut().map(|param| (*loc, param))),
                    )?;
                    let after_params = if !func.attributes.is_empty() || !func.returns.is_empty() {
                        ""
                    } else if func.body.is_some() {
                        " {"
                    } else {
                        ";"
                    };
                    let should_multiline = header_multiline &&
                        matches!(
                            fmt.config.multiline_func_header,
                            MultilineFuncHeaderStyle::ParamsFirst | MultilineFuncHeaderStyle::All
                        );
                    params_multiline = should_multiline ||
                        multiline ||
                        fmt.are_chunks_separated_multiline(
                            &format!("{{}}){after_params}"),
                            &params,
                            ",",
                        )?;
                    // Write new line if we have only one parameter and params on multiline set.
                    if params.len() == 1 && params_multiline {
                        writeln!(fmt.buf())?;
                    }
                    fmt.write_chunks_separated(&params, ",", params_multiline)?;
                    Ok(())
                },
            )?;
        }

        let mut write_attributes = |fmt: &mut Self, multiline: bool| -> Result<()> {
            // write attributes
            if !func.attributes.is_empty() {
                let attrs_loc = func
                    .attributes
                    .first()
                    .unwrap()
                    .loc()
                    .with_end_from(&func.attributes.last().unwrap().loc());
                if fmt.inline_config.is_disabled(attrs_loc) {
                    // If params are also disabled then write functions attributes on the same line.
                    if params_disabled {
                        fmt.write_whitespace_separator(false)?;
                        let attrs_src =
                            String::from_utf8(self.source.as_bytes()[attrs_loc.range()].to_vec())
                                .map_err(FormatterError::custom)?;
                        fmt.write_raw(attrs_src)?;
                    } else {
                        fmt.indented(1, |fmt| fmt.visit_source(attrs_loc))?;
                    }
                } else {
                    fmt.write_postfix_comments_before(attrs_loc.start())?;
                    fmt.write_whitespace_separator(multiline)?;
                    let attributes =
                        fmt.items_to_chunks_sorted(attrs_end, func.attributes.iter_mut())?;
                    fmt.indented(1, |fmt| {
                        fmt.write_chunks_separated(&attributes, "", multiline)?;
                        Ok(())
                    })?;
                }
            }

            // write returns
            if !func.returns.is_empty() {
                let returns_start_loc = func.returns.first().unwrap().0;
                let returns_loc = returns_start_loc.with_end_from(&func.returns.last().unwrap().0);
                if fmt.inline_config.is_disabled(returns_loc) {
                    fmt.write_whitespace_separator(false)?;
                    let returns_src =
                        String::from_utf8(self.source.as_bytes()[returns_loc.range()].to_vec())
                            .map_err(FormatterError::custom)?;
                    fmt.write_raw(format!("returns ({returns_src})"))?;
                } else {
                    let mut returns = fmt.items_to_chunks(
                        returns_end,
                        func.returns
                            .iter_mut()
                            .filter_map(|(loc, param)| param.as_mut().map(|param| (*loc, param))),
                    )?;

                    // there's an issue with function return value that would lead to indent issues because those can be formatted with line breaks <https://github.com/foundry-rs/foundry/issues/4080>
                    for function_chunk in
                        returns.iter_mut().filter(|chunk| chunk.content.starts_with("function("))
                    {
                        // this will bypass the recursive indent that was applied when the function
                        // content was formatted in the chunk
                        function_chunk.content = function_chunk
                            .content
                            .split('\n')
                            .map(|s| s.trim_start())
                            .collect::<Vec<_>>()
                            .join("\n");
                    }

                    fmt.write_postfix_comments_before(returns_loc.start())?;
                    fmt.write_whitespace_separator(multiline)?;
                    fmt.indented(1, |fmt| {
                        fmt.surrounded(
                            SurroundingChunk::new("returns (", Some(returns_loc.start()), None),
                            SurroundingChunk::new(")", None, returns_end),
                            |fmt, multiline_hint| {
                                fmt.write_chunks_separated(&returns, ",", multiline_hint)?;
                                Ok(())
                            },
                        )?;
                        Ok(())
                    })?;
                }
            }
            Ok(())
        };

        let should_multiline = header_multiline &&
            if params_multiline {
                matches!(self.config.multiline_func_header, MultilineFuncHeaderStyle::All)
            } else {
                matches!(
                    self.config.multiline_func_header,
                    MultilineFuncHeaderStyle::AttributesFirst
                )
            };
        let attrs_multiline = should_multiline ||
            !self.try_on_single_line(|fmt| {
                write_attributes(fmt, false)?;
                if !fmt.will_it_fit(if func.body.is_some() { " {" } else { ";" }) {
                    bail!(FormatterError::fmt())
                }
                Ok(())
            })?;
        if attrs_multiline {
            write_attributes(self, true)?;
        }
        Ok(attrs_multiline)
    }

    /// Write potentially nested `if statements`
    fn write_if_stmt(
        &mut self,
        loc: Loc,
        cond: &mut Expression,
        if_branch: &mut Box<Statement>,
        else_branch: &mut Option<Box<Statement>>,
    ) -> Result<(), FormatterError> {
        let single_line_stmt_wide = self.context.if_stmt_single_line.unwrap_or_default();

        visit_source_if_disabled_else!(self, loc.with_end(if_branch.loc().start()), {
            self.surrounded(
                SurroundingChunk::new("if (", Some(loc.start()), Some(cond.loc().start())),
                SurroundingChunk::new(")", None, Some(if_branch.loc().start())),
                |fmt, _| {
                    fmt.write_prefix_comments_before(cond.loc().end())?;
                    cond.visit(fmt)?;
                    fmt.write_postfix_comments_before(if_branch.loc().start())
                },
            )?;
        });

        let cond_close_paren_loc =
            self.find_next_in_src(cond.loc().end(), ')').unwrap_or_else(|| cond.loc().end());
        let attempt_single_line = single_line_stmt_wide &&
            self.should_attempt_block_single_line(if_branch.as_mut(), cond_close_paren_loc);
        let if_branch_is_single_line = self.visit_stmt_as_block(if_branch, attempt_single_line)?;
        if single_line_stmt_wide && !if_branch_is_single_line {
            bail!(FormatterError::fmt())
        }

        if let Some(else_branch) = else_branch {
            self.write_postfix_comments_before(else_branch.loc().start())?;
            if if_branch_is_single_line {
                writeln!(self.buf())?;
            }
            write_chunk!(self, else_branch.loc().start(), "else")?;
            if let Statement::If(loc, cond, if_branch, else_branch) = else_branch.as_mut() {
                self.visit_if(*loc, cond, if_branch, else_branch, false)?;
            } else {
                let else_branch_is_single_line =
                    self.visit_stmt_as_block(else_branch, attempt_single_line)?;
                if single_line_stmt_wide && !else_branch_is_single_line {
                    bail!(FormatterError::fmt())
                }
            }
        }
        Ok(())
    }

    /// Sorts grouped import statement alphabetically.
    fn sort_imports(&self, source_unit: &mut SourceUnit) {
        // first we need to find the grouped import statements
        // A group is defined as a set of import statements that are separated by a blank line
        let mut import_groups = Vec::new();
        let mut current_group = Vec::new();
        let mut source_unit_parts = source_unit.0.iter().enumerate().peekable();
        while let Some((i, part)) = source_unit_parts.next() {
            if let SourceUnitPart::ImportDirective(_) = part {
                current_group.push(i);
                let current_loc = part.loc();
                if let Some((_, next_part)) = source_unit_parts.peek() {
                    let next_loc = next_part.loc();
                    // import statements are followed by a new line, so if there are more than one
                    // we have a group
                    if self.blank_lines(current_loc.end(), next_loc.start()) > 1 {
                        import_groups.push(std::mem::take(&mut current_group));
                    }
                }
            } else if !current_group.is_empty() {
                import_groups.push(std::mem::take(&mut current_group));
            }
        }

        if !current_group.is_empty() {
            import_groups.push(current_group);
        }

        if import_groups.is_empty() {
            // nothing to sort
            return
        }

        // order all groups alphabetically
        for group in import_groups.iter() {
            // SAFETY: group is not empty
            let first = group[0];
            let last = group.last().copied().expect("group is not empty");
            let import_directives = &mut source_unit.0[first..=last];

            // sort rename style imports alphabetically based on the actual import and not the
            // rename
            for source_unit_part in import_directives.iter_mut() {
                if let SourceUnitPart::ImportDirective(Import::Rename(_, renames, _)) =
                    source_unit_part
                {
                    renames.sort_by_cached_key(|(og_ident, _)| og_ident.name.clone());
                }
            }

            import_directives.sort_by_cached_key(|item| match item {
                SourceUnitPart::ImportDirective(import) => match import {
                    Import::Plain(path, _) => path.to_string(),
                    Import::GlobalSymbol(path, _, _) => path.to_string(),
                    Import::Rename(path, _, _) => path.to_string(),
                },
                _ => {
                    unreachable!("import group contains non-import statement")
                }
            });
        }
    }
}

// Traverse the Solidity Parse Tree and write to the code formatter
impl<'a, W: Write> Visitor for Formatter<'a, W> {
    type Error = FormatterError;

    #[instrument(name = "source", skip(self))]
    fn visit_source(&mut self, loc: Loc) -> Result<()> {
        let source = String::from_utf8(self.source.as_bytes()[loc.range()].to_vec())
            .map_err(FormatterError::custom)?;
        let mut lines = source.splitn(2, '\n');

        write_chunk!(self, loc.start(), "{}", lines.next().unwrap())?;
        if let Some(remainder) = lines.next() {
            // Call with `self.write_str` and not `write!`, so we can have `\n` at the beginning
            // without triggering an indentation
            self.write_raw(format!("\n{remainder}"))?;
        }

        let _ = self.comments.remove_all_comments_before(loc.end());

        Ok(())
    }

    #[instrument(name = "SU", skip_all)]
    fn visit_source_unit(&mut self, source_unit: &mut SourceUnit) -> Result<()> {
        if self.config.sort_imports {
            self.sort_imports(source_unit);
        }
        // TODO: do we need to put pragma and import directives at the top of the file?
        // source_unit.0.sort_by_key(|item| match item {
        //     SourceUnitPart::PragmaDirective(_, _, _) => 0,
        //     SourceUnitPart::ImportDirective(_, _) => 1,
        //     _ => usize::MAX,
        // });
        let loc = Loc::File(
            source_unit
                .loc_opt()
                .or_else(|| self.comments.iter().next().map(|comment| comment.loc))
                .map(|loc| loc.file_no())
                .unwrap_or_default(),
            0,
            self.source.len(),
        );

        self.write_lined_visitable(
            loc,
            source_unit.0.iter_mut(),
            |last_unit, unit| match last_unit {
                SourceUnitPart::PragmaDirective(..) => {
                    !matches!(unit, SourceUnitPart::PragmaDirective(..))
                }
                SourceUnitPart::ImportDirective(_) => {
                    !matches!(unit, SourceUnitPart::ImportDirective(_))
                }
                SourceUnitPart::ErrorDefinition(_) => {
                    !matches!(unit, SourceUnitPart::ErrorDefinition(_))
                }
                SourceUnitPart::Using(_) => !matches!(unit, SourceUnitPart::Using(_)),
                SourceUnitPart::VariableDefinition(_) => {
                    !matches!(unit, SourceUnitPart::VariableDefinition(_))
                }
                SourceUnitPart::Annotation(_) => false,
                _ => true,
            },
        )?;

        // EOF newline
        if self.last_char().map_or(true, |char| char != '\n') {
            writeln!(self.buf())?;
        }

        Ok(())
    }

    #[instrument(name = "contract", skip_all)]
    fn visit_contract(&mut self, contract: &mut ContractDefinition) -> Result<()> {
        return_source_if_disabled!(self, contract.loc);

        self.with_contract_context(contract.clone(), |fmt| {
            let contract_name = contract.name.safe_unwrap();

            visit_source_if_disabled_else!(
                fmt,
                contract.loc.with_end_from(
                    &contract.base.first().map(|b| b.loc).unwrap_or(contract_name.loc)
                ),
                {
                    fmt.grouped(|fmt| {
                        write_chunk!(fmt, contract.loc.start(), "{}", contract.ty)?;
                        write_chunk!(fmt, contract_name.loc.end(), "{}", contract_name.name)?;
                        if !contract.base.is_empty() {
                            write_chunk!(
                                fmt,
                                contract_name.loc.end(),
                                contract.base.first().unwrap().loc.start(),
                                "is"
                            )?;
                        }
                        Ok(())
                    })?;
                }
            );

            if !contract.base.is_empty() {
                visit_source_if_disabled_else!(
                    fmt,
                    contract
                        .base
                        .first()
                        .unwrap()
                        .loc
                        .with_end_from(&contract.base.last().unwrap().loc),
                    {
                        fmt.indented(1, |fmt| {
                            let base_end = contract.parts.first().map(|part| part.loc().start());
                            let bases = fmt.items_to_chunks(
                                base_end,
                                contract.base.iter_mut().map(|base| (base.loc, base)),
                            )?;
                            let multiline =
                                fmt.are_chunks_separated_multiline("{}", &bases, ",")?;
                            fmt.write_chunks_separated(&bases, ",", multiline)?;
                            fmt.write_whitespace_separator(multiline)?;
                            Ok(())
                        })?;
                    }
                );
            }

            write_chunk!(fmt, "{{")?;

            fmt.indented(1, |fmt| {
                if let Some(first) = contract.parts.first() {
                    fmt.write_postfix_comments_before(first.loc().start())?;
                    fmt.write_whitespace_separator(true)?;
                } else {
                    return Ok(())
                }

                if fmt.config.contract_new_lines {
                    write_chunk!(fmt, "\n")?;
                }

                fmt.write_lined_visitable(
                    contract.loc,
                    contract.parts.iter_mut(),
                    |last_part, part| match last_part {
                        ContractPart::ErrorDefinition(_) => {
                            !matches!(part, ContractPart::ErrorDefinition(_))
                        }
                        ContractPart::EventDefinition(_) => {
                            !matches!(part, ContractPart::EventDefinition(_))
                        }
                        ContractPart::VariableDefinition(_) => {
                            !matches!(part, ContractPart::VariableDefinition(_))
                        }
                        ContractPart::TypeDefinition(_) => {
                            !matches!(part, ContractPart::TypeDefinition(_))
                        }
                        ContractPart::EnumDefinition(_) => {
                            !matches!(part, ContractPart::EnumDefinition(_))
                        }
                        ContractPart::Using(_) => !matches!(part, ContractPart::Using(_)),
                        ContractPart::FunctionDefinition(last_def) => {
                            if last_def.is_empty() {
                                match part {
                                    ContractPart::FunctionDefinition(def) => !def.is_empty(),
                                    _ => true,
                                }
                            } else {
                                true
                            }
                        }
                        ContractPart::Annotation(_) => false,
                        _ => true,
                    },
                )
            })?;

            if !contract.parts.is_empty() {
                fmt.write_whitespace_separator(true)?;

                if fmt.config.contract_new_lines {
                    write_chunk!(fmt, "\n")?;
                }
            }

            write_chunk!(fmt, contract.loc.end(), "}}")?;

            Ok(())
        })?;

        Ok(())
    }

    // Support extension for Solana/Substrate
    #[instrument(name = "annotation", skip_all)]
    fn visit_annotation(&mut self, annotation: &mut Annotation) -> Result<()> {
        return_source_if_disabled!(self, annotation.loc);
        let id = self.simulate_to_string(|fmt| annotation.id.visit(fmt))?;
        write!(self.buf(), "@{id}")?;
        write!(self.buf(), "(")?;
        annotation.value.visit(self)?;
        write!(self.buf(), ")")?;
        Ok(())
    }

    #[instrument(name = "pragma", skip_all)]
    fn visit_pragma(
        &mut self,
        loc: Loc,
        ident: &mut Option<Identifier>,
        string: &mut Option<StringLiteral>,
    ) -> Result<()> {
        let (ident, string) = (ident.safe_unwrap(), string.safe_unwrap());
        return_source_if_disabled!(self, loc, ';');

        #[allow(clippy::if_same_then_else)]
        let pragma_descriptor = if ident.name == "solidity" {
            // There are some issues with parsing Solidity's versions with crates like `semver`:
            // 1. Ranges like `>=0.4.21<0.6.0` or `>=0.4.21 <0.6.0` are not parseable at all.
            // 2. Versions like `0.8.10` got transformed into `^0.8.10` which is not the same.
            // TODO: semver-solidity crate :D
            &string.string
        } else {
            &string.string
        };

        write_chunk!(self, string.loc.end(), "pragma {} {};", &ident.name, pragma_descriptor)?;

        Ok(())
    }

    #[instrument(name = "import_plain", skip_all)]
    fn visit_import_plain(&mut self, loc: Loc, import: &mut ImportPath) -> Result<()> {
        return_source_if_disabled!(self, loc, ';');

        self.grouped(|fmt| {
            write_chunk!(fmt, loc.start(), import.loc().start(), "import")?;
            fmt.write_quoted_str(import.loc(), None, &import_path_string(import))?;
            fmt.write_semicolon()?;
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "import_global", skip_all)]
    fn visit_import_global(
        &mut self,
        loc: Loc,
        global: &mut ImportPath,
        alias: &mut Identifier,
    ) -> Result<()> {
        return_source_if_disabled!(self, loc, ';');

        self.grouped(|fmt| {
            write_chunk!(fmt, loc.start(), global.loc().start(), "import")?;
            fmt.write_quoted_str(global.loc(), None, &import_path_string(global))?;
            write_chunk!(fmt, loc.start(), alias.loc.start(), "as")?;
            alias.visit(fmt)?;
            fmt.write_semicolon()?;
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "import_renames", skip_all)]
    fn visit_import_renames(
        &mut self,
        loc: Loc,
        imports: &mut [(Identifier, Option<Identifier>)],
        from: &mut ImportPath,
    ) -> Result<()> {
        return_source_if_disabled!(self, loc, ';');

        if imports.is_empty() {
            self.grouped(|fmt| {
                write_chunk!(fmt, loc.start(), "import")?;
                fmt.write_empty_brackets()?;
                write_chunk!(fmt, loc.start(), from.loc().start(), "from")?;
                fmt.write_quoted_str(from.loc(), None, &import_path_string(from))?;
                fmt.write_semicolon()?;
                Ok(())
            })?;
            return Ok(())
        }

        let imports_start = imports.first().unwrap().0.loc.start();

        write_chunk!(self, loc.start(), imports_start, "import")?;

        self.surrounded(
            SurroundingChunk::new("{", Some(imports_start), None),
            SurroundingChunk::new("}", None, Some(from.loc().start())),
            |fmt, _multiline| {
                let mut imports = imports.iter_mut().peekable();
                let mut import_chunks = Vec::new();
                while let Some((ident, alias)) = imports.next() {
                    import_chunks.push(fmt.chunked(
                        ident.loc.start(),
                        imports.peek().map(|(ident, _)| ident.loc.start()),
                        |fmt| {
                            fmt.grouped(|fmt| {
                                ident.visit(fmt)?;
                                if let Some(alias) = alias {
                                    write_chunk!(fmt, ident.loc.end(), alias.loc.start(), "as")?;
                                    alias.visit(fmt)?;
                                }
                                Ok(())
                            })?;
                            Ok(())
                        },
                    )?);
                }

                let multiline = fmt.are_chunks_separated_multiline(
                    &format!("{{}} }} from \"{}\";", import_path_string(from)),
                    &import_chunks,
                    ",",
                )?;
                fmt.write_chunks_separated(&import_chunks, ",", multiline)?;
                Ok(())
            },
        )?;

        self.grouped(|fmt| {
            write_chunk!(fmt, imports_start, from.loc().start(), "from")?;
            fmt.write_quoted_str(from.loc(), None, &import_path_string(from))?;
            fmt.write_semicolon()?;
            Ok(())
        })?;

        Ok(())
    }

    #[instrument(name = "enum", skip_all)]
    fn visit_enum(&mut self, enumeration: &mut EnumDefinition) -> Result<()> {
        return_source_if_disabled!(self, enumeration.loc);

        let enum_name = enumeration.name.safe_unwrap_mut();
        let mut name =
            self.visit_to_chunk(enum_name.loc.start(), Some(enum_name.loc.end()), enum_name)?;
        name.content = format!("enum {} ", name.content);
        if enumeration.values.is_empty() {
            self.write_chunk(&name)?;
            self.write_empty_brackets()?;
        } else {
            name.content.push('{');
            self.write_chunk(&name)?;

            self.indented(1, |fmt| {
                let values = fmt.items_to_chunks(
                    Some(enumeration.loc.end()),
                    enumeration.values.iter_mut().map(|ident| {
                        let ident = ident.safe_unwrap_mut();
                        (ident.loc, ident)
                    }),
                )?;
                fmt.write_chunks_separated(&values, ",", true)?;
                writeln!(fmt.buf())?;
                Ok(())
            })?;
            write_chunk!(self, "}}")?;
        }

        Ok(())
    }

    #[instrument(name = "assembly", skip_all)]
    fn visit_assembly(
        &mut self,
        loc: Loc,
        dialect: &mut Option<StringLiteral>,
        block: &mut YulBlock,
        flags: &mut Option<Vec<StringLiteral>>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);

        write_chunk!(self, loc.start(), "assembly")?;
        if let Some(StringLiteral { loc, string, .. }) = dialect {
            write_chunk!(self, loc.start(), loc.end(), "\"{string}\"")?;
        }
        if let Some(flags) = flags {
            if !flags.is_empty() {
                let loc_start = flags.first().unwrap().loc.start();
                self.surrounded(
                    SurroundingChunk::new("(", Some(loc_start), None),
                    SurroundingChunk::new(")", None, Some(block.loc.start())),
                    |fmt, _| {
                        let mut flags = flags.iter_mut().peekable();
                        let mut chunks = vec![];
                        while let Some(flag) = flags.next() {
                            let next_byte_offset =
                                flags.peek().map(|next_flag| next_flag.loc.start());
                            chunks.push(fmt.chunked(
                                flag.loc.start(),
                                next_byte_offset,
                                |fmt| {
                                    write!(fmt.buf(), "\"{}\"", flag.string)?;
                                    Ok(())
                                },
                            )?);
                        }
                        fmt.write_chunks_separated(&chunks, ",", false)?;
                        Ok(())
                    },
                )?;
            }
        }

        block.visit(self)
    }

    #[instrument(name = "block", skip_all)]
    fn visit_block(
        &mut self,
        loc: Loc,
        unchecked: bool,
        statements: &mut Vec<Statement>,
    ) -> Result<()> {
        return_source_if_disabled!(self, loc);
        if unchecked {
            write_chunk!(self, loc.start(), "unchecked ")?;
        }

        self.visit_block(loc, statements, false, false)?;
        Ok(())
    }

    #[instrument(name = "args", skip_all)]
    fn visit_args(&mut self, loc: Loc, args: &mut Vec<NamedArgument>) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);

        write!(self.buf(), "{{")?;

        let mut args_iter = args.iter_mut().peekable();
        let mut chunks = Vec::new();
        while let Some(NamedArgument { loc: arg_loc, name, expr }) = args_iter.next() {
            let next_byte_offset = args_iter
                .peek()
                .map(|NamedArgument { loc: arg_loc, .. }| arg_loc.start())
                .unwrap_or_else(|| loc.end());
            chunks.push(self.chunked(arg_loc.start(), Some(next_byte_offset), |fmt| {
                fmt.grouped(|fmt| {
                    write_chunk!(fmt, name.loc.start(), "{}: ", name.name)?;
                    expr.visit(fmt)
                })?;
                Ok(())
            })?);
        }

        if let Some(first) = chunks.first_mut() {
            if first.prefixes.is_empty() &&
                first.postfixes_before.is_empty() &&
                !self.config.bracket_spacing
            {
                first.needs_space = Some(false);
            }
        }
        let multiline = self.are_chunks_separated_multiline("{}}", &chunks, ",")?;
        self.indented_if(multiline, 1, |fmt| fmt.write_chunks_separated(&chunks, ",", multiline))?;

        let prefix = if multiline && !self.is_beginning_of_line() {
            "\n"
        } else if self.config.bracket_spacing {
            " "
        } else {
            ""
        };
        let closing_bracket = format!("{prefix}{}", "}");
        let closing_bracket_loc = args.last().unwrap().loc.end();
        write_chunk!(self, closing_bracket_loc, "{closing_bracket}")?;

        Ok(())
    }

    #[instrument(name = "expr", skip_all)]
    fn visit_expr(&mut self, loc: Loc, expr: &mut Expression) -> Result<()> {
        return_source_if_disabled!(self, loc);

        match expr {
            Expression::Type(loc, typ) => match typ {
                Type::Address => write_chunk!(self, loc.start(), "address")?,
                Type::AddressPayable => write_chunk!(self, loc.start(), "address payable")?,
                Type::Payable => write_chunk!(self, loc.start(), "payable")?,
                Type::Bool => write_chunk!(self, loc.start(), "bool")?,
                Type::String => write_chunk!(self, loc.start(), "string")?,
                Type::Bytes(n) => write_chunk!(self, loc.start(), "bytes{}", n)?,
                Type::Rational => write_chunk!(self, loc.start(), "rational")?,
                Type::DynamicBytes => write_chunk!(self, loc.start(), "bytes")?,
                Type::Int(ref n) | Type::Uint(ref n) => {
                    let int = if matches!(typ, Type::Int(_)) { "int" } else { "uint" };
                    match n {
                        256 => match self.config.int_types {
                            IntTypes::Long => write_chunk!(self, loc.start(), "{int}{n}")?,
                            IntTypes::Short => write_chunk!(self, loc.start(), "{int}")?,
                            IntTypes::Preserve => self.visit_source(*loc)?,
                        },
                        _ => write_chunk!(self, loc.start(), "{int}{n}")?,
                    }
                }
                Type::Mapping { loc, key, key_name, value, value_name } => {
                    let arrow_loc = self.find_next_str_in_src(loc.start(), "=>");
                    let close_paren_loc =
                        self.find_next_in_src(value.loc().end(), ')').unwrap_or(loc.end());
                    let first = SurroundingChunk::new(
                        "mapping(",
                        Some(loc.start()),
                        Some(key.loc().start()),
                    );
                    let last = SurroundingChunk::new(")", Some(close_paren_loc), Some(loc.end()))
                        .non_spaced();
                    self.surrounded(first, last, |fmt, multiline| {
                        fmt.grouped(|fmt| {
                            key.visit(fmt)?;

                            if let Some(name) = key_name {
                                let end_loc = arrow_loc.unwrap_or(value.loc().start());
                                write_chunk!(fmt, name.loc.start(), end_loc, " {}", name)?;
                            } else if let Some(arrow_loc) = arrow_loc {
                                fmt.write_postfix_comments_before(arrow_loc)?;
                            }

                            let mut write_arrow_and_value = |fmt: &mut Self| {
                                write!(fmt.buf(), "=> ")?;
                                value.visit(fmt)?;
                                if let Some(name) = value_name {
                                    write_chunk!(fmt, name.loc.start(), " {}", name)?;
                                }
                                Ok(())
                            };

                            let rest_str = fmt.simulate_to_string(&mut write_arrow_and_value)?;
                            let multiline = multiline && !fmt.will_it_fit(rest_str);
                            fmt.write_whitespace_separator(multiline)?;

                            write_arrow_and_value(fmt)?;

                            fmt.write_postfix_comments_before(close_paren_loc)?;
                            fmt.write_prefix_comments_before(close_paren_loc)
                        })?;
                        Ok(())
                    })?;
                }
                Type::Function { .. } => self.visit_source(*loc)?,
            },
            Expression::BoolLiteral(loc, val) => {
                write_chunk!(self, loc.start(), loc.end(), "{val}")?;
            }
            Expression::NumberLiteral(loc, val, exp, unit) => {
                self.write_num_literal(*loc, val, None, exp, unit)?;
            }
            Expression::HexNumberLiteral(loc, val, unit) => {
                // ref: https://docs.soliditylang.org/en/latest/types.html?highlight=address%20literal#address-literals
                let val = if val.len() == 42 {
                    Address::from_str(val).expect("").to_string()
                } else {
                    val.to_owned()
                };
                write_chunk!(self, loc.start(), loc.end(), "{val}")?;
                self.write_unit(unit)?;
            }
            Expression::RationalNumberLiteral(loc, val, fraction, exp, unit) => {
                self.write_num_literal(*loc, val, Some(fraction), exp, unit)?;
            }
            Expression::StringLiteral(vals) => {
                for StringLiteral { loc, string, unicode } in vals {
                    let prefix = if *unicode { Some("unicode") } else { None };
                    self.write_quoted_str(*loc, prefix, string)?;
                }
            }
            Expression::HexLiteral(vals) => {
                for val in vals {
                    self.write_hex_literal(val)?;
                }
            }
            Expression::AddressLiteral(loc, val) => {
                // support of solana/substrate address literals
                self.write_quoted_str(*loc, Some("address"), val)?;
            }
            Expression::Parenthesis(loc, expr) => {
                self.surrounded(
                    SurroundingChunk::new("(", Some(loc.start()), None),
                    SurroundingChunk::new(")", None, Some(loc.end())),
                    |fmt, _| expr.visit(fmt),
                )?;
            }
            Expression::ArraySubscript(_, ty_exp, index_expr) => {
                ty_exp.visit(self)?;
                write!(self.buf(), "[")?;
                index_expr.as_mut().map(|index| index.visit(self)).transpose()?;
                write!(self.buf(), "]")?;
            }
            Expression::ArraySlice(loc, expr, start, end) => {
                expr.visit(self)?;
                write!(self.buf(), "[")?;
                let mut write_slice = |fmt: &mut Self, multiline| -> Result<()> {
                    if multiline {
                        fmt.write_whitespace_separator(true)?;
                    }
                    fmt.grouped(|fmt| {
                        start.as_mut().map(|start| start.visit(fmt)).transpose()?;
                        write!(fmt.buf(), ":")?;
                        if let Some(end) = end {
                            let mut chunk =
                                fmt.chunked(end.loc().start(), Some(loc.end()), |fmt| {
                                    end.visit(fmt)
                                })?;
                            if chunk.prefixes.is_empty() &&
                                chunk.postfixes_before.is_empty() &&
                                (start.is_none() || fmt.will_it_fit(&chunk.content))
                            {
                                chunk.needs_space = Some(false);
                            }
                            fmt.write_chunk(&chunk)?;
                        }
                        Ok(())
                    })?;
                    if multiline {
                        fmt.write_whitespace_separator(true)?;
                    }
                    Ok(())
                };

                if !self.try_on_single_line(|fmt| write_slice(fmt, false))? {
                    self.indented(1, |fmt| write_slice(fmt, true))?;
                }

                write!(self.buf(), "]")?;
            }
            Expression::ArrayLiteral(loc, exprs) => {
                write_chunk!(self, loc.start(), "[")?;
                let chunks = self.items_to_chunks(
                    Some(loc.end()),
                    exprs.iter_mut().map(|expr| (expr.loc(), expr)),
                )?;
                let multiline = self.are_chunks_separated_multiline("{}]", &chunks, ",")?;
                self.indented_if(multiline, 1, |fmt| {
                    fmt.write_chunks_separated(&chunks, ",", multiline)?;
                    if multiline {
                        fmt.write_postfix_comments_before(loc.end())?;
                        fmt.write_prefix_comments_before(loc.end())?;
                        fmt.write_whitespace_separator(true)?;
                    }
                    Ok(())
                })?;
                write_chunk!(self, loc.end(), "]")?;
            }
            Expression::PreIncrement(..) |
            Expression::PostIncrement(..) |
            Expression::PreDecrement(..) |
            Expression::PostDecrement(..) |
            Expression::Not(..) |
            Expression::UnaryPlus(..) |
            Expression::Add(..) |
            Expression::Negate(..) |
            Expression::Subtract(..) |
            Expression::Power(..) |
            Expression::Multiply(..) |
            Expression::Divide(..) |
            Expression::Modulo(..) |
            Expression::ShiftLeft(..) |
            Expression::ShiftRight(..) |
            Expression::BitwiseNot(..) |
            Expression::BitwiseAnd(..) |
            Expression::BitwiseXor(..) |
            Expression::BitwiseOr(..) |
            Expression::Less(..) |
            Expression::More(..) |
            Expression::LessEqual(..) |
            Expression::MoreEqual(..) |
            Expression::And(..) |
            Expression::Or(..) |
            Expression::Equal(..) |
            Expression::NotEqual(..) => {
                let spaced = expr.has_space_around();
                let op = expr.operator().unwrap();

                match expr.components_mut() {
                    (Some(left), Some(right)) => {
                        left.visit(self)?;

                        let right_chunk =
                            self.chunked(right.loc().start(), Some(loc.end()), |fmt| {
                                write_chunk!(fmt, right.loc().start(), "{op}")?;
                                right.visit(fmt)?;
                                Ok(())
                            })?;

                        self.grouped(|fmt| fmt.write_chunk(&right_chunk))?;
                    }
                    (Some(left), None) => {
                        left.visit(self)?;
                        write_chunk_spaced!(self, loc.end(), Some(spaced), "{op}")?;
                    }
                    (None, Some(right)) => {
                        write_chunk!(self, right.loc().start(), "{op}")?;
                        let mut right_chunk =
                            self.visit_to_chunk(right.loc().end(), Some(loc.end()), right)?;
                        right_chunk.needs_space = Some(spaced);
                        self.write_chunk(&right_chunk)?;
                    }
                    (None, None) => {}
                }
            }
            Expression::Assign(..) |
            Expression::AssignOr(..) |
            Expression::AssignAnd(..) |
            Expression::AssignXor(..) |
            Expression::AssignShiftLeft(..) |
            Expression::AssignShiftRight(..) |
            Expression::AssignAdd(..) |
            Expression::AssignSubtract(..) |
            Expression::AssignMultiply(..) |
            Expression::AssignDivide(..) |
            Expression::AssignModulo(..) => {
                let op = expr.operator().unwrap();
                let (left, right) = expr.components_mut();
                let (left, right) = (left.unwrap(), right.unwrap());

                left.visit(self)?;
                write_chunk!(self, "{op}")?;
                self.visit_assignment(right)?;
            }
            Expression::ConditionalOperator(loc, cond, first_expr, second_expr) => {
                cond.visit(self)?;

                let first_expr = self.chunked(
                    first_expr.loc().start(),
                    Some(second_expr.loc().start()),
                    |fmt| {
                        write_chunk!(fmt, "?")?;
                        first_expr.visit(fmt)
                    },
                )?;
                let second_expr =
                    self.chunked(second_expr.loc().start(), Some(loc.end()), |fmt| {
                        write_chunk!(fmt, ":")?;
                        second_expr.visit(fmt)
                    })?;

                let chunks = vec![first_expr, second_expr];
                if !self.try_on_single_line(|fmt| fmt.write_chunks_separated(&chunks, "", false))? {
                    self.grouped(|fmt| fmt.write_chunks_separated(&chunks, "", true))?;
                }
            }
            Expression::Variable(ident) => {
                write_chunk!(self, loc.end(), "{}", ident.name)?;
            }
            Expression::MemberAccess(_, expr, ident) => {
                self.visit_member_access(expr, ident, |fmt, expr| match expr.as_mut() {
                    Expression::MemberAccess(_, inner_expr, inner_ident) => {
                        Ok(Some((inner_expr, inner_ident)))
                    }
                    expr => {
                        expr.visit(fmt)?;
                        Ok(None)
                    }
                })?;
            }
            Expression::List(loc, items) => {
                self.surrounded(
                    SurroundingChunk::new(
                        "(",
                        Some(loc.start()),
                        items.first().map(|item| item.0.start()),
                    ),
                    SurroundingChunk::new(")", None, Some(loc.end())),
                    |fmt, _| {
                        let items = fmt.items_to_chunks(
                            Some(loc.end()),
                            items.iter_mut().map(|(loc, item)| (*loc, item)),
                        )?;
                        let write_items = |fmt: &mut Self, multiline| {
                            fmt.write_chunks_separated(&items, ",", multiline)
                        };
                        if !fmt.try_on_single_line(|fmt| write_items(fmt, false))? {
                            write_items(fmt, true)?;
                        }
                        Ok(())
                    },
                )?;
            }
            Expression::FunctionCall(loc, expr, exprs) => {
                self.visit_expr(expr.loc(), expr)?;
                self.visit_list("", exprs, Some(expr.loc().end()), Some(loc.end()), true)?;
            }
            Expression::NamedFunctionCall(loc, expr, args) => {
                self.visit_expr(expr.loc(), expr)?;
                write!(self.buf(), "(")?;
                self.visit_args(*loc, args)?;
                write!(self.buf(), ")")?;
            }
            Expression::FunctionCallBlock(_, expr, stmt) => {
                expr.visit(self)?;
                stmt.visit(self)?;
            }
            Expression::New(_, expr) => {
                write_chunk!(self, "new ")?;
                self.visit_expr(expr.loc(), expr)?;
            }
            _ => self.visit_source(loc)?,
        };

        Ok(())
    }

    #[instrument(name = "ident", skip_all)]
    fn visit_ident(&mut self, loc: Loc, ident: &mut Identifier) -> Result<()> {
        return_source_if_disabled!(self, loc);
        write_chunk!(self, loc.end(), "{}", ident.name)?;
        Ok(())
    }

    #[instrument(name = "ident_path", skip_all)]
    fn visit_ident_path(&mut self, idents: &mut IdentifierPath) -> Result<(), Self::Error> {
        if idents.identifiers.is_empty() {
            return Ok(())
        }
        return_source_if_disabled!(self, idents.loc);

        idents.identifiers.iter_mut().skip(1).for_each(|chunk| {
            if !chunk.name.starts_with('.') {
                chunk.name.insert(0, '.')
            }
        });
        let chunks = self.items_to_chunks(
            Some(idents.loc.end()),
            idents.identifiers.iter_mut().map(|ident| (ident.loc, ident)),
        )?;
        self.grouped(|fmt| {
            let multiline = fmt.are_chunks_separated_multiline("{}", &chunks, "")?;
            fmt.write_chunks_separated(&chunks, "", multiline)
        })?;
        Ok(())
    }

    #[instrument(name = "emit", skip_all)]
    fn visit_emit(&mut self, loc: Loc, event: &mut Expression) -> Result<()> {
        return_source_if_disabled!(self, loc);
        write_chunk!(self, loc.start(), "emit")?;
        event.visit(self)?;
        self.write_semicolon()?;
        Ok(())
    }

    #[instrument(name = "var_definition", skip_all)]
    fn visit_var_definition(&mut self, var: &mut VariableDefinition) -> Result<()> {
        return_source_if_disabled!(self, var.loc, ';');

        var.ty.visit(self)?;

        let multiline = self.grouped(|fmt| {
            let var_name = var.name.safe_unwrap_mut();
            let name_start = var_name.loc.start();

            let attrs = fmt.items_to_chunks_sorted(Some(name_start), var.attrs.iter_mut())?;
            if !fmt.try_on_single_line(|fmt| fmt.write_chunks_separated(&attrs, "", false))? {
                fmt.write_chunks_separated(&attrs, "", true)?;
            }

            let mut name = fmt.visit_to_chunk(name_start, Some(var_name.loc.end()), var_name)?;
            if var.initializer.is_some() {
                name.content.push_str(" =");
            }
            fmt.write_chunk(&name)?;

            Ok(())
        })?;

        var.initializer
            .as_mut()
            .map(|init| self.indented_if(multiline, 1, |fmt| fmt.visit_assignment(init)))
            .transpose()?;

        self.write_semicolon()?;

        Ok(())
    }

    #[instrument(name = "var_definition_stmt", skip_all)]
    fn visit_var_definition_stmt(
        &mut self,
        loc: Loc,
        declaration: &mut VariableDeclaration,
        expr: &mut Option<Expression>,
    ) -> Result<()> {
        return_source_if_disabled!(self, loc, ';');

        let declaration = self
            .chunked(declaration.loc.start(), None, |fmt| fmt.visit_var_declaration(declaration))?;
        let multiline = declaration.content.contains('\n');
        self.write_chunk(&declaration)?;

        if let Some(expr) = expr {
            write!(self.buf(), " =")?;
            self.indented_if(multiline, 1, |fmt| fmt.visit_assignment(expr))?;
        }

        self.write_semicolon()
    }

    #[instrument(name = "var_declaration", skip_all)]
    fn visit_var_declaration(&mut self, var: &mut VariableDeclaration) -> Result<()> {
        return_source_if_disabled!(self, var.loc);
        self.grouped(|fmt| {
            var.ty.visit(fmt)?;
            if let Some(storage) = &var.storage {
                write_chunk!(fmt, storage.loc().end(), "{storage}")?;
            }
            let var_name = var.name.safe_unwrap();
            write_chunk!(fmt, var_name.loc.end(), "{var_name}")
        })?;
        Ok(())
    }

    #[instrument(name = "return", skip_all)]
    fn visit_return(&mut self, loc: Loc, expr: &mut Option<Expression>) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc, ';');

        self.write_postfix_comments_before(loc.start())?;
        self.write_prefix_comments_before(loc.start())?;

        if expr.is_none() {
            write_chunk!(self, loc.end(), "return;")?;
            return Ok(())
        }

        let expr = expr.as_mut().unwrap();
        let expr_loc_start = expr.loc().start();
        let write_return = |fmt: &mut Self| -> Result<()> {
            write_chunk!(fmt, loc.start(), "return")?;
            fmt.write_postfix_comments_before(expr_loc_start)?;
            Ok(())
        };

        let mut write_return_with_expr = |fmt: &mut Self| -> Result<()> {
            let fits_on_single = fmt.try_on_single_line(|fmt| {
                write_return(fmt)?;
                expr.visit(fmt)
            })?;
            if fits_on_single {
                return Ok(())
            }

            let mut fit_on_next_line = false;
            let tx = fmt.transact(|fmt| {
                fmt.grouped(|fmt| {
                    write_return(fmt)?;
                    if !fmt.is_beginning_of_line() {
                        fmt.write_whitespace_separator(true)?;
                    }
                    fit_on_next_line = fmt.try_on_single_line(|fmt| expr.visit(fmt))?;
                    Ok(())
                })?;
                Ok(())
            })?;
            if fit_on_next_line {
                tx.commit()?;
                return Ok(())
            }

            write_return(fmt)?;
            expr.visit(fmt)?;
            Ok(())
        };

        write_return_with_expr(self)?;
        write_chunk!(self, loc.end(), ";")?;
        Ok(())
    }

    #[instrument(name = "revert", skip_all)]
    fn visit_revert(
        &mut self,
        loc: Loc,
        error: &mut Option<IdentifierPath>,
        args: &mut Vec<Expression>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc, ';');
        write_chunk!(self, loc.start(), "revert")?;
        if let Some(error) = error {
            error.visit(self)?;
        }
        self.visit_list("", args, None, Some(loc.end()), true)?;
        self.write_semicolon()?;

        Ok(())
    }

    #[instrument(name = "revert_named_args", skip_all)]
    fn visit_revert_named_args(
        &mut self,
        loc: Loc,
        error: &mut Option<IdentifierPath>,
        args: &mut Vec<NamedArgument>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc, ';');

        write_chunk!(self, loc.start(), "revert")?;
        let mut error_indented = false;
        if let Some(error) = error {
            if !self.try_on_single_line(|fmt| error.visit(fmt))? {
                error.visit(self)?;
                error_indented = true;
            }
        }

        if args.is_empty() {
            write!(self.buf(), "({{}});")?;
            return Ok(())
        }

        write!(self.buf(), "(")?;
        self.indented_if(error_indented, 1, |fmt| fmt.visit_args(loc, args))?;
        write!(self.buf(), ")")?;
        self.write_semicolon()?;

        Ok(())
    }

    #[instrument(name = "break", skip_all)]
    fn visit_break(&mut self, loc: Loc, semicolon: bool) -> Result<()> {
        if semicolon {
            return_source_if_disabled!(self, loc, ';');
        } else {
            return_source_if_disabled!(self, loc);
        }
        write_chunk!(self, loc.start(), loc.end(), "break{}", if semicolon { ";" } else { "" })
    }

    #[instrument(name = "continue", skip_all)]
    fn visit_continue(&mut self, loc: Loc, semicolon: bool) -> Result<()> {
        if semicolon {
            return_source_if_disabled!(self, loc, ';');
        } else {
            return_source_if_disabled!(self, loc);
        }
        write_chunk!(self, loc.start(), loc.end(), "continue{}", if semicolon { ";" } else { "" })
    }

    #[instrument(name = "try", skip_all)]
    fn visit_try(
        &mut self,
        loc: Loc,
        expr: &mut Expression,
        returns: &mut Option<(Vec<(Loc, Option<Parameter>)>, Box<Statement>)>,
        clauses: &mut Vec<CatchClause>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);

        let try_next_byte = clauses.first().map(|c| match c {
            CatchClause::Simple(loc, ..) => loc.start(),
            CatchClause::Named(loc, ..) => loc.start(),
        });
        let try_chunk = self.chunked(loc.start(), try_next_byte, |fmt| {
            write_chunk!(fmt, loc.start(), expr.loc().start(), "try")?;
            expr.visit(fmt)?;
            if let Some((params, stmt)) = returns {
                let mut params =
                    params.iter_mut().filter(|(_, param)| param.is_some()).collect::<Vec<_>>();
                let byte_offset = params.first().map_or(stmt.loc().start(), |p| p.0.start());
                fmt.surrounded(
                    SurroundingChunk::new("returns (", Some(byte_offset), None),
                    SurroundingChunk::new(")", None, params.last().map(|p| p.0.end())),
                    |fmt, _| {
                        let chunks = fmt.items_to_chunks(
                            Some(stmt.loc().start()),
                            params.iter_mut().map(|(loc, ref mut ident)| (*loc, ident)),
                        )?;
                        let multiline = fmt.are_chunks_separated_multiline("{})", &chunks, ",")?;
                        fmt.write_chunks_separated(&chunks, ",", multiline)?;
                        Ok(())
                    },
                )?;
                stmt.visit(fmt)?;
            }
            Ok(())
        })?;

        let mut chunks = vec![try_chunk];
        for clause in clauses {
            let (loc, ident, mut param, stmt) = match clause {
                CatchClause::Simple(loc, param, stmt) => (loc, None, param.as_mut(), stmt),
                CatchClause::Named(loc, ident, param, stmt) => {
                    (loc, Some(ident), Some(param), stmt)
                }
            };

            let chunk = self.chunked(loc.start(), Some(stmt.loc().start()), |fmt| {
                write_chunk!(fmt, "catch")?;
                if let Some(ident) = ident.as_ref() {
                    fmt.write_postfix_comments_before(
                        param.as_ref().map(|p| p.loc.start()).unwrap_or_else(|| ident.loc.end()),
                    )?;
                    write_chunk!(fmt, ident.loc.start(), "{}", ident.name)?;
                }
                if let Some(param) = param.as_mut() {
                    write_chunk_spaced!(fmt, param.loc.start(), Some(ident.is_none()), "(")?;
                    fmt.surrounded(
                        SurroundingChunk::new("", Some(param.loc.start()), None),
                        SurroundingChunk::new(")", None, Some(stmt.loc().start())),
                        |fmt, _| param.visit(fmt),
                    )?;
                }

                stmt.visit(fmt)?;
                Ok(())
            })?;

            chunks.push(chunk);
        }

        let multiline = self.are_chunks_separated_multiline("{}", &chunks, "")?;
        if !multiline {
            self.write_chunks_separated(&chunks, "", false)?;
            return Ok(())
        }

        let mut chunks = chunks.iter_mut().peekable();
        let mut prev_multiline = false;

        // write try chunk first
        if let Some(chunk) = chunks.next() {
            let chunk_str = self.simulate_to_string(|fmt| fmt.write_chunk(chunk))?;
            write!(self.buf(), "{chunk_str}")?;
            prev_multiline = chunk_str.contains('\n');
        }

        while let Some(chunk) = chunks.next() {
            let chunk_str = self.simulate_to_string(|fmt| fmt.write_chunk(chunk))?;
            let multiline = chunk_str.contains('\n');
            self.indented_if(!multiline, 1, |fmt| {
                chunk.needs_space = Some(false);
                let on_same_line = prev_multiline && (multiline || chunks.peek().is_none());
                let prefix = if fmt.is_beginning_of_line() {
                    ""
                } else if on_same_line {
                    " "
                } else {
                    "\n"
                };
                let chunk_str = format!("{prefix}{chunk_str}");
                write!(fmt.buf(), "{chunk_str}")?;
                Ok(())
            })?;
            prev_multiline = multiline;
        }
        Ok(())
    }

    #[instrument(name = "if", skip_all)]
    fn visit_if(
        &mut self,
        loc: Loc,
        cond: &mut Expression,
        if_branch: &mut Box<Statement>,
        else_branch: &mut Option<Box<Statement>>,
        is_first_stmt: bool,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);

        if !is_first_stmt {
            self.write_if_stmt(loc, cond, if_branch, else_branch)?;
            return Ok(())
        }

        self.context.if_stmt_single_line = Some(true);
        let mut stmt_fits_on_single = false;
        let tx = self.transact(|fmt| {
            stmt_fits_on_single = match fmt.write_if_stmt(loc, cond, if_branch, else_branch) {
                Ok(()) => true,
                Err(FormatterError::Fmt(_)) => false,
                Err(err) => bail!(err),
            };
            Ok(())
        })?;

        if stmt_fits_on_single {
            tx.commit()?;
        } else {
            self.context.if_stmt_single_line = Some(false);
            self.write_if_stmt(loc, cond, if_branch, else_branch)?;
        }
        self.context.if_stmt_single_line = None;

        Ok(())
    }

    #[instrument(name = "do_while", skip_all)]
    fn visit_do_while(
        &mut self,
        loc: Loc,
        body: &mut Statement,
        cond: &mut Expression,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc, ';');
        write_chunk!(self, loc.start(), "do ")?;
        self.visit_stmt_as_block(body, false)?;
        visit_source_if_disabled_else!(self, loc.with_start(body.loc().end()), {
            self.surrounded(
                SurroundingChunk::new("while (", Some(cond.loc().start()), None),
                SurroundingChunk::new(");", None, Some(loc.end())),
                |fmt, _| cond.visit(fmt),
            )?;
        });
        Ok(())
    }

    #[instrument(name = "while", skip_all)]
    fn visit_while(
        &mut self,
        loc: Loc,
        cond: &mut Expression,
        body: &mut Statement,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);
        self.surrounded(
            SurroundingChunk::new("while (", Some(loc.start()), None),
            SurroundingChunk::new(")", None, Some(cond.loc().end())),
            |fmt, _| {
                cond.visit(fmt)?;
                fmt.write_postfix_comments_before(body.loc().start())
            },
        )?;

        let cond_close_paren_loc =
            self.find_next_in_src(cond.loc().end(), ')').unwrap_or_else(|| cond.loc().end());
        let attempt_single_line = self.should_attempt_block_single_line(body, cond_close_paren_loc);
        self.visit_stmt_as_block(body, attempt_single_line)?;
        Ok(())
    }

    #[instrument(name = "for", skip_all)]
    fn visit_for(
        &mut self,
        loc: Loc,
        init: &mut Option<Box<Statement>>,
        cond: &mut Option<Box<Expression>>,
        update: &mut Option<Box<Expression>>,
        body: &mut Option<Box<Statement>>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);

        let next_byte_end = update.as_ref().map(|u| u.loc().end());
        self.surrounded(
            SurroundingChunk::new("for (", Some(loc.start()), None),
            SurroundingChunk::new(")", None, next_byte_end),
            |fmt, _| {
                let mut write_for_loop_header = |fmt: &mut Self, multiline: bool| -> Result<()> {
                    match init {
                        Some(stmt) => stmt.visit(fmt),
                        None => fmt.write_semicolon(),
                    }?;
                    if multiline {
                        fmt.write_whitespace_separator(true)?;
                    }

                    cond.visit(fmt)?;
                    fmt.write_semicolon()?;
                    if multiline {
                        fmt.write_whitespace_separator(true)?;
                    }

                    match update {
                        Some(expr) => expr.visit(fmt),
                        None => Ok(()),
                    }
                };
                let multiline = !fmt.try_on_single_line(|fmt| write_for_loop_header(fmt, false))?;
                if multiline {
                    write_for_loop_header(fmt, true)?;
                }
                Ok(())
            },
        )?;
        match body {
            Some(body) => {
                self.visit_stmt_as_block(body, false)?;
            }
            None => {
                self.write_empty_brackets()?;
            }
        };
        Ok(())
    }

    #[instrument(name = "function", skip_all)]
    fn visit_function(&mut self, func: &mut FunctionDefinition) -> Result<()> {
        if func.body.is_some() {
            return_source_if_disabled!(self, func.loc());
        } else {
            return_source_if_disabled!(self, func.loc(), ';');
        }

        self.with_function_context(func.clone(), |fmt| {
            fmt.write_postfix_comments_before(func.loc.start())?;
            fmt.write_prefix_comments_before(func.loc.start())?;

            let body_loc = func.body.as_ref().map(CodeLocation::loc);
            let mut attrs_multiline = false;
            let fits_on_single = fmt.try_on_single_line(|fmt| {
                fmt.write_function_header(func, body_loc, false)?;
                Ok(())
            })?;
            if !fits_on_single {
                attrs_multiline = fmt.write_function_header(func, body_loc, true)?;
            }

            // write function body
            match &mut func.body {
                Some(body) => {
                    let body_loc = body.loc();
                    // Handle case where block / statements starts on disabled line.
                    if fmt.inline_config.is_disabled(body_loc.with_end(body_loc.start())) {
                        match body {
                            Statement::Block { statements, .. } if !statements.is_empty() => {
                                fmt.write_whitespace_separator(false)?;
                                fmt.visit_block(body_loc, statements, false, false)?;
                                return Ok(())
                            }
                            _ => {
                                // Attrs should be written on same line if first line is disabled
                                // and there's no statement.
                                attrs_multiline = false
                            }
                        }
                    }

                    let byte_offset = body_loc.start();
                    let body = fmt.visit_to_chunk(byte_offset, Some(body_loc.end()), body)?;
                    fmt.write_whitespace_separator(
                        attrs_multiline && !(func.attributes.is_empty() && func.returns.is_empty()),
                    )?;
                    fmt.write_chunk(&body)?;
                }
                None => fmt.write_semicolon()?,
            }
            Ok(())
        })?;

        Ok(())
    }

    #[instrument(name = "function_attribute", skip_all)]
    fn visit_function_attribute(&mut self, attribute: &mut FunctionAttribute) -> Result<()> {
        return_source_if_disabled!(self, attribute.loc());

        match attribute {
            FunctionAttribute::Mutability(mutability) => {
                write_chunk!(self, mutability.loc().end(), "{mutability}")?
            }
            FunctionAttribute::Visibility(visibility) => {
                // Visibility will always have a location in a Function attribute
                write_chunk!(self, visibility.loc_opt().unwrap().end(), "{visibility}")?
            }
            FunctionAttribute::Virtual(loc) => write_chunk!(self, loc.end(), "virtual")?,
            FunctionAttribute::Immutable(loc) => write_chunk!(self, loc.end(), "immutable")?,
            FunctionAttribute::Override(loc, args) => {
                write_chunk!(self, loc.start(), "override")?;
                if !args.is_empty() && self.config.override_spacing {
                    self.write_whitespace_separator(false)?;
                }
                self.visit_list("", args, None, Some(loc.end()), false)?
            }
            FunctionAttribute::BaseOrModifier(loc, base) => {
                // here we need to find out if this attribute belongs to the constructor because the
                // modifier need to include the trailing parenthesis
                // This is very ambiguous because the modifier can either by an inherited contract
                // or a modifier here: e.g.: This is valid constructor:
                // `constructor() public  Ownable() OnlyOwner {}`
                let is_constructor = self.context.is_constructor_function();
                // we can't make any decisions here regarding trailing `()` because we'd need to
                // find out if the `base` is a solidity modifier or an
                // interface/contract therefor we we its raw content.

                // we can however check if the contract `is` the `base`, this however also does
                // not cover all cases
                let is_contract_base = self.context.contract.as_ref().map_or(false, |contract| {
                    contract.base.iter().any(|contract_base| {
                        contract_base
                            .name
                            .identifiers
                            .iter()
                            .zip(&base.name.identifiers)
                            .all(|(l, r)| l.name == r.name)
                    })
                });

                if is_contract_base {
                    base.visit(self)?;
                } else if is_constructor {
                    // This is ambiguous because the modifier can either by an inherited
                    // contract modifiers with empty parenthesis are
                    // valid, but not required so we make the assumption
                    // here that modifiers are lowercase
                    let mut base_or_modifier =
                        self.visit_to_chunk(loc.start(), Some(loc.end()), base)?;
                    let is_lowercase =
                        base_or_modifier.content.chars().next().map_or(false, |c| c.is_lowercase());
                    if is_lowercase && base_or_modifier.content.ends_with("()") {
                        base_or_modifier.content.truncate(base_or_modifier.content.len() - 2);
                    }

                    self.write_chunk(&base_or_modifier)?;
                } else {
                    let mut base_or_modifier =
                        self.visit_to_chunk(loc.start(), Some(loc.end()), base)?;
                    if base_or_modifier.content.ends_with("()") {
                        base_or_modifier.content.truncate(base_or_modifier.content.len() - 2);
                    }
                    self.write_chunk(&base_or_modifier)?;
                }
            }
            FunctionAttribute::Error(loc) => self.visit_parser_error(*loc)?,
        };

        Ok(())
    }

    #[instrument(name = "var_attribute", skip_all)]
    fn visit_var_attribute(&mut self, attribute: &mut VariableAttribute) -> Result<()> {
        return_source_if_disabled!(self, attribute.loc());

        let token = match attribute {
            VariableAttribute::Visibility(visibility) => Some(visibility.to_string()),
            VariableAttribute::Constant(_) => Some("constant".to_string()),
            VariableAttribute::Immutable(_) => Some("immutable".to_string()),
            VariableAttribute::Override(loc, idents) => {
                write_chunk!(self, loc.start(), "override")?;
                if !idents.is_empty() && self.config.override_spacing {
                    self.write_whitespace_separator(false)?;
                }
                self.visit_list("", idents, Some(loc.start()), Some(loc.end()), false)?;
                None
            }
        };
        if let Some(token) = token {
            let loc = attribute.loc();
            write_chunk!(self, loc.start(), loc.end(), "{}", token)?;
        }
        Ok(())
    }

    #[instrument(name = "base", skip_all)]
    fn visit_base(&mut self, base: &mut Base) -> Result<()> {
        return_source_if_disabled!(self, base.loc);

        let name_loc = &base.name.loc;
        let mut name = self.chunked(name_loc.start(), Some(name_loc.end()), |fmt| {
            fmt.visit_ident_path(&mut base.name)?;
            Ok(())
        })?;

        if base.args.is_none() || base.args.as_ref().unwrap().is_empty() {
            // This is ambiguous because the modifier can either by an inherited contract or a
            // modifier
            if self.context.function.is_some() {
                name.content.push_str("()");
            }
            self.write_chunk(&name)?;
            return Ok(())
        }

        let args = base.args.as_mut().unwrap();
        let args_start = CodeLocation::loc(args.first().unwrap()).start();

        name.content.push('(');
        let formatted_name = self.chunk_to_string(&name)?;

        let multiline = !self.will_it_fit(&formatted_name);

        self.surrounded(
            SurroundingChunk::new(&formatted_name, Some(args_start), None),
            SurroundingChunk::new(")", None, Some(base.loc.end())),
            |fmt, multiline_hint| {
                let args = fmt.items_to_chunks(
                    Some(base.loc.end()),
                    args.iter_mut().map(|arg| (arg.loc(), arg)),
                )?;
                let multiline = multiline ||
                    multiline_hint ||
                    fmt.are_chunks_separated_multiline("{}", &args, ",")?;
                fmt.write_chunks_separated(&args, ",", multiline)?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[instrument(name = "parameter", skip_all)]
    fn visit_parameter(&mut self, parameter: &mut Parameter) -> Result<()> {
        return_source_if_disabled!(self, parameter.loc);
        self.grouped(|fmt| {
            parameter.ty.visit(fmt)?;
            if let Some(storage) = &parameter.storage {
                write_chunk!(fmt, storage.loc().end(), "{storage}")?;
            }
            if let Some(name) = &parameter.name {
                write_chunk!(fmt, parameter.loc.end(), "{}", name.name)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "struct", skip_all)]
    fn visit_struct(&mut self, structure: &mut StructDefinition) -> Result<()> {
        return_source_if_disabled!(self, structure.loc);
        self.grouped(|fmt| {
            let struct_name = structure.name.safe_unwrap_mut();
            write_chunk!(fmt, struct_name.loc.start(), "struct")?;
            struct_name.visit(fmt)?;
            if structure.fields.is_empty() {
                return fmt.write_empty_brackets()
            }

            write!(fmt.buf(), " {{")?;
            fmt.surrounded(
                SurroundingChunk::new("", Some(struct_name.loc.end()), None),
                SurroundingChunk::new("}", None, Some(structure.loc.end())),
                |fmt, _multiline| {
                    let chunks = fmt.items_to_chunks(
                        Some(structure.loc.end()),
                        structure.fields.iter_mut().map(|ident| (ident.loc, ident)),
                    )?;
                    for mut chunk in chunks {
                        chunk.content.push(';');
                        fmt.write_chunk(&chunk)?;
                        fmt.write_whitespace_separator(true)?;
                    }
                    Ok(())
                },
            )
        })?;

        Ok(())
    }

    #[instrument(name = "event", skip_all)]
    fn visit_event(&mut self, event: &mut EventDefinition) -> Result<()> {
        return_source_if_disabled!(self, event.loc, ';');

        let event_name = event.name.safe_unwrap_mut();
        let mut name =
            self.visit_to_chunk(event_name.loc.start(), Some(event.loc.end()), event_name)?;
        name.content = format!("event {}(", name.content);

        let last_chunk = if event.anonymous { ") anonymous;" } else { ");" };
        if event.fields.is_empty() {
            name.content.push_str(last_chunk);
            self.write_chunk(&name)?;
        } else {
            let byte_offset = event.fields.first().unwrap().loc.start();
            let first_chunk = self.chunk_to_string(&name)?;
            self.surrounded(
                SurroundingChunk::new(first_chunk, Some(byte_offset), None),
                SurroundingChunk::new(last_chunk, None, Some(event.loc.end())),
                |fmt, multiline| {
                    let params = fmt
                        .items_to_chunks(None, event.fields.iter_mut().map(|arg| (arg.loc, arg)))?;

                    let multiline =
                        multiline && fmt.are_chunks_separated_multiline("{}", &params, ",")?;
                    fmt.write_chunks_separated(&params, ",", multiline)
                },
            )?;
        }

        Ok(())
    }

    #[instrument(name = "event_parameter", skip_all)]
    fn visit_event_parameter(&mut self, param: &mut EventParameter) -> Result<()> {
        return_source_if_disabled!(self, param.loc);

        self.grouped(|fmt| {
            param.ty.visit(fmt)?;
            if param.indexed {
                write_chunk!(fmt, param.loc.start(), "indexed")?;
            }
            if let Some(name) = &param.name {
                write_chunk!(fmt, name.loc.end(), "{}", name.name)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "error", skip_all)]
    fn visit_error(&mut self, error: &mut ErrorDefinition) -> Result<()> {
        return_source_if_disabled!(self, error.loc, ';');

        let error_name = error.name.safe_unwrap_mut();
        let mut name = self.visit_to_chunk(error_name.loc.start(), None, error_name)?;
        name.content = format!("error {}", name.content);

        let formatted_name = self.chunk_to_string(&name)?;
        write!(self.buf(), "{formatted_name}")?;
        let start_offset = error.fields.first().map(|f| f.loc.start());
        self.visit_list("", &mut error.fields, start_offset, Some(error.loc.end()), true)?;
        self.write_semicolon()?;

        Ok(())
    }

    #[instrument(name = "error_parameter", skip_all)]
    fn visit_error_parameter(&mut self, param: &mut ErrorParameter) -> Result<()> {
        return_source_if_disabled!(self, param.loc);
        self.grouped(|fmt| {
            param.ty.visit(fmt)?;
            if let Some(name) = &param.name {
                write_chunk!(fmt, name.loc.end(), "{}", name.name)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "type_definition", skip_all)]
    fn visit_type_definition(&mut self, def: &mut TypeDefinition) -> Result<()> {
        return_source_if_disabled!(self, def.loc, ';');
        self.grouped(|fmt| {
            write_chunk!(fmt, def.loc.start(), def.name.loc.start(), "type")?;
            def.name.visit(fmt)?;
            write_chunk!(fmt, def.name.loc.end(), CodeLocation::loc(&def.ty).start(), "is")?;
            def.ty.visit(fmt)?;
            fmt.write_semicolon()?;
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "stray_semicolon", skip_all)]
    fn visit_stray_semicolon(&mut self) -> Result<()> {
        self.write_semicolon()
    }

    #[instrument(name = "opening_paren", skip_all)]
    fn visit_opening_paren(&mut self) -> Result<()> {
        write_chunk!(self, "(")?;
        Ok(())
    }

    #[instrument(name = "closing_paren", skip_all)]
    fn visit_closing_paren(&mut self) -> Result<()> {
        write_chunk!(self, ")")?;
        Ok(())
    }

    #[instrument(name = "newline", skip_all)]
    fn visit_newline(&mut self) -> Result<()> {
        writeln_chunk!(self)?;
        Ok(())
    }

    #[instrument(name = "using", skip_all)]
    fn visit_using(&mut self, using: &mut Using) -> Result<()> {
        return_source_if_disabled!(self, using.loc, ';');

        write_chunk!(self, using.loc.start(), "using")?;

        let ty_start = using.ty.as_mut().map(|ty| CodeLocation::loc(&ty).start());
        let global_start = using.global.as_mut().map(|global| global.loc.start());
        let loc_end = using.loc.end();

        let (is_library, mut list_chunks) = match &mut using.list {
            UsingList::Library(library) => {
                (true, vec![self.visit_to_chunk(library.loc.start(), None, library)?])
            }
            UsingList::Functions(funcs) => {
                let mut funcs = funcs.iter_mut().peekable();
                let mut chunks = Vec::new();
                while let Some(func) = funcs.next() {
                    let next_byte_end = funcs.peek().map(|func| func.loc.start());
                    chunks.push(self.chunked(func.loc.start(), next_byte_end, |fmt| {
                        fmt.visit_ident_path(&mut func.path)?;
                        if let Some(op) = func.oper {
                            write!(fmt.buf(), " as {op}")?;
                        }
                        Ok(())
                    })?);
                }
                (false, chunks)
            }
            UsingList::Error => return self.visit_parser_error(using.loc),
        };

        let for_chunk = self.chunk_at(
            using.loc.start(),
            Some(ty_start.or(global_start).unwrap_or(loc_end)),
            None,
            "for",
        );
        let ty_chunk = if let Some(ty) = &mut using.ty {
            self.visit_to_chunk(ty.loc().start(), Some(global_start.unwrap_or(loc_end)), ty)?
        } else {
            self.chunk_at(using.loc.start(), Some(global_start.unwrap_or(loc_end)), None, "*")
        };
        let global_chunk = using
            .global
            .as_mut()
            .map(|global| self.visit_to_chunk(global.loc.start(), Some(using.loc.end()), global))
            .transpose()?;

        let write_for_def = |fmt: &mut Self| {
            fmt.grouped(|fmt| {
                fmt.write_chunk(&for_chunk)?;
                fmt.write_chunk(&ty_chunk)?;
                if let Some(global_chunk) = global_chunk.as_ref() {
                    fmt.write_chunk(global_chunk)?;
                }
                Ok(())
            })?;
            Ok(())
        };

        let simulated_for_def = self.simulate_to_string(write_for_def)?;

        if is_library {
            let chunk = list_chunks.pop().unwrap();
            if self.will_chunk_fit(&format!("{{}} {simulated_for_def};"), &chunk)? {
                self.write_chunk(&chunk)?;
                write_for_def(self)?;
            } else {
                self.write_whitespace_separator(true)?;
                self.grouped(|fmt| {
                    fmt.write_chunk(&chunk)?;
                    Ok(())
                })?;
                self.write_whitespace_separator(true)?;
                write_for_def(self)?;
            }
        } else {
            self.surrounded(
                SurroundingChunk::new("{", Some(using.loc.start()), None),
                SurroundingChunk::new(
                    "}",
                    None,
                    Some(ty_start.or(global_start).unwrap_or(loc_end)),
                ),
                |fmt, _multiline| {
                    let multiline = fmt.are_chunks_separated_multiline(
                        &format!("{{ {{}} }} {simulated_for_def};"),
                        &list_chunks,
                        ",",
                    )?;
                    fmt.write_chunks_separated(&list_chunks, ",", multiline)?;
                    Ok(())
                },
            )?;
            write_for_def(self)?;
        }

        self.write_semicolon()?;

        Ok(())
    }

    #[instrument(name = "yul_block", skip_all)]
    fn visit_yul_block(
        &mut self,
        loc: Loc,
        statements: &mut Vec<YulStatement>,
        attempt_single_line: bool,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);
        self.visit_block(loc, statements, attempt_single_line, false)?;
        Ok(())
    }

    #[instrument(name = "yul_expr", skip_all)]
    fn visit_yul_expr(&mut self, expr: &mut YulExpression) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, expr.loc());

        match expr {
            YulExpression::BoolLiteral(loc, val, ident) => {
                let val = if *val { "true" } else { "false" };
                self.visit_yul_string_with_ident(*loc, val, ident)
            }
            YulExpression::FunctionCall(expr) => self.visit_yul_function_call(expr),
            YulExpression::HexNumberLiteral(loc, val, ident) => {
                self.visit_yul_string_with_ident(*loc, val, ident)
            }
            YulExpression::HexStringLiteral(val, ident) => self.visit_yul_string_with_ident(
                val.loc,
                &self.quote_str(val.loc, Some("hex"), &val.hex),
                ident,
            ),
            YulExpression::NumberLiteral(loc, val, expr, ident) => {
                let val = if expr.is_empty() { val.to_owned() } else { format!("{val}e{expr}") };
                self.visit_yul_string_with_ident(*loc, &val, ident)
            }
            YulExpression::StringLiteral(val, ident) => self.visit_yul_string_with_ident(
                val.loc,
                &self.quote_str(val.loc, None, &val.string),
                ident,
            ),
            YulExpression::SuffixAccess(_, expr, ident) => {
                self.visit_member_access(expr, ident, |fmt, expr| match expr.as_mut() {
                    YulExpression::SuffixAccess(_, inner_expr, inner_ident) => {
                        Ok(Some((inner_expr, inner_ident)))
                    }
                    expr => {
                        expr.visit(fmt)?;
                        Ok(None)
                    }
                })
            }
            YulExpression::Variable(ident) => {
                write_chunk!(self, ident.loc.start(), ident.loc.end(), "{}", ident.name)
            }
        }
    }

    #[instrument(name = "yul_assignment", skip_all)]
    fn visit_yul_assignment<T>(
        &mut self,
        loc: Loc,
        exprs: &mut Vec<T>,
        expr: &mut Option<&mut YulExpression>,
    ) -> Result<(), Self::Error>
    where
        T: Visitable + CodeLocation,
    {
        return_source_if_disabled!(self, loc);

        self.grouped(|fmt| {
            let chunks =
                fmt.items_to_chunks(None, exprs.iter_mut().map(|expr| (expr.loc(), expr)))?;

            let multiline = fmt.are_chunks_separated_multiline("{} := ", &chunks, ",")?;
            fmt.write_chunks_separated(&chunks, ",", multiline)?;

            if let Some(expr) = expr {
                write_chunk!(fmt, expr.loc().start(), ":=")?;
                let chunk = fmt.visit_to_chunk(expr.loc().start(), Some(loc.end()), expr)?;
                if !fmt.will_chunk_fit("{}", &chunk)? {
                    fmt.write_whitespace_separator(true)?;
                }
                fmt.write_chunk(&chunk)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    #[instrument(name = "yul_for", skip_all)]
    fn visit_yul_for(&mut self, stmt: &mut YulFor) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, stmt.loc);
        write_chunk!(self, stmt.loc.start(), "for")?;
        self.visit_yul_block(stmt.init_block.loc, &mut stmt.init_block.statements, true)?;
        stmt.condition.visit(self)?;
        self.visit_yul_block(stmt.post_block.loc, &mut stmt.post_block.statements, true)?;
        self.visit_yul_block(stmt.execution_block.loc, &mut stmt.execution_block.statements, true)?;
        Ok(())
    }

    #[instrument(name = "yul_function_call", skip_all)]
    fn visit_yul_function_call(&mut self, stmt: &mut YulFunctionCall) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, stmt.loc);
        write_chunk!(self, stmt.loc.start(), "{}", stmt.id.name)?;
        self.visit_list("", &mut stmt.arguments, None, Some(stmt.loc.end()), true)
    }

    #[instrument(name = "yul_fun_def", skip_all)]
    fn visit_yul_fun_def(&mut self, stmt: &mut YulFunctionDefinition) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, stmt.loc);

        write_chunk!(self, stmt.loc.start(), "function {}", stmt.id.name)?;

        self.visit_list("", &mut stmt.params, None, None, true)?;

        if !stmt.returns.is_empty() {
            self.grouped(|fmt| {
                write_chunk!(fmt, "->")?;

                let chunks = fmt.items_to_chunks(
                    Some(stmt.body.loc.start()),
                    stmt.returns.iter_mut().map(|param| (param.loc, param)),
                )?;
                let multiline = fmt.are_chunks_separated_multiline("{}", &chunks, ",")?;
                fmt.write_chunks_separated(&chunks, ",", multiline)?;
                if multiline {
                    fmt.write_whitespace_separator(true)?;
                }
                Ok(())
            })?;
        }

        stmt.body.visit(self)?;

        Ok(())
    }

    #[instrument(name = "yul_if", skip_all)]
    fn visit_yul_if(
        &mut self,
        loc: Loc,
        expr: &mut YulExpression,
        block: &mut YulBlock,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);
        write_chunk!(self, loc.start(), "if")?;
        expr.visit(self)?;
        self.visit_yul_block(block.loc, &mut block.statements, true)
    }

    #[instrument(name = "yul_leave", skip_all)]
    fn visit_yul_leave(&mut self, loc: Loc) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);
        write_chunk!(self, loc.start(), loc.end(), "leave")
    }

    #[instrument(name = "yul_switch", skip_all)]
    fn visit_yul_switch(&mut self, stmt: &mut YulSwitch) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, stmt.loc);

        write_chunk!(self, stmt.loc.start(), "switch")?;
        stmt.condition.visit(self)?;
        writeln_chunk!(self)?;
        let mut cases = stmt.cases.iter_mut().peekable();
        while let Some(YulSwitchOptions::Case(loc, expr, block)) = cases.next() {
            write_chunk!(self, loc.start(), "case")?;
            expr.visit(self)?;
            self.visit_yul_block(block.loc, &mut block.statements, true)?;
            let is_last = cases.peek().is_none();
            if !is_last || stmt.default.is_some() {
                writeln_chunk!(self)?;
            }
        }
        if let Some(YulSwitchOptions::Default(loc, ref mut block)) = stmt.default {
            write_chunk!(self, loc.start(), "default")?;
            self.visit_yul_block(block.loc, &mut block.statements, true)?;
        }
        Ok(())
    }

    #[instrument(name = "yul_var_declaration", skip_all)]
    fn visit_yul_var_declaration(
        &mut self,
        loc: Loc,
        idents: &mut Vec<YulTypedIdentifier>,
        expr: &mut Option<YulExpression>,
    ) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, loc);
        self.grouped(|fmt| {
            write_chunk!(fmt, loc.start(), "let")?;
            fmt.visit_yul_assignment(loc, idents, &mut expr.as_mut())
        })?;
        Ok(())
    }

    #[instrument(name = "yul_typed_ident", skip_all)]
    fn visit_yul_typed_ident(&mut self, ident: &mut YulTypedIdentifier) -> Result<(), Self::Error> {
        return_source_if_disabled!(self, ident.loc);
        self.visit_yul_string_with_ident(ident.loc, &ident.id.name, &mut ident.ty)
    }

    #[instrument(name = "parser_error", skip_all)]
    fn visit_parser_error(&mut self, loc: Loc) -> Result<()> {
        Err(FormatterError::InvalidParsedItem(loc))
    }
}

/// An action which may be committed to a Formatter
struct Transaction<'f, 'a, W> {
    fmt: &'f mut Formatter<'a, W>,
    buffer: String,
    comments: Comments,
}

impl<'f, 'a, W> std::ops::Deref for Transaction<'f, 'a, W> {
    type Target = Formatter<'a, W>;
    fn deref(&self) -> &Self::Target {
        self.fmt
    }
}

impl<'f, 'a, W> std::ops::DerefMut for Transaction<'f, 'a, W> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.fmt
    }
}

impl<'f, 'a, W: Write> Transaction<'f, 'a, W> {
    /// Create a new transaction from a callback
    fn new(
        fmt: &'f mut Formatter<'a, W>,
        fun: impl FnMut(&mut Formatter<'a, W>) -> Result<()>,
    ) -> Result<Self> {
        let mut comments = fmt.comments.clone();
        let buffer = fmt.with_temp_buf(fun)?.w;
        comments = std::mem::replace(&mut fmt.comments, comments);
        Ok(Self { fmt, buffer, comments })
    }

    /// Commit the transaction to the Formatter
    fn commit(self) -> Result<String> {
        self.fmt.comments = self.comments;
        write_chunk!(self.fmt, "{}", self.buffer)?;
        Ok(self.buffer)
    }
}
