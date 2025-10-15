use crate::iter::IterDelimited;
use solar::parse::{
    ast::{CommentKind, Span},
    interface::{BytePos, CharPos, SourceMap, source_map::SourceFile},
    lexer::token::RawTokenKind as TokenKind,
};
use std::fmt;

mod comment;
pub use comment::{Comment, CommentStyle};

pub mod inline_config;

pub const DISABLE_START: &str = "forgefmt: disable-start";
pub const DISABLE_END: &str = "forgefmt: disable-end";

pub struct Comments {
    comments: std::collections::VecDeque<Comment>,
}

impl fmt::Debug for Comments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Comments")?;
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Comments {
    pub fn new(
        sf: &SourceFile,
        sm: &SourceMap,
        normalize_cmnts: bool,
        group_cmnts: bool,
        tab_width: Option<usize>,
    ) -> Self {
        let gatherer = CommentGatherer::new(sf, sm, normalize_cmnts, tab_width).gather();

        Self {
            comments: if group_cmnts { gatherer.group().into() } else { gatherer.comments.into() },
        }
    }

    pub fn peek(&self) -> Option<&Comment> {
        self.comments.front()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Comment> {
        self.comments.pop_front()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Comment> {
        self.comments.iter()
    }

    /// Adds a new comment at the beginning of the list.
    ///
    /// Should only be used when comments are gathered scattered, and must be manually sorted.
    ///
    /// **WARNING:** This struct works under the assumption that comments are always sorted by
    /// ascending span position. It is the caller's responsibility to ensure that this premise
    /// always holds true.
    pub fn push_front(&mut self, cmnt: Comment) {
        self.comments.push_front(cmnt)
    }

    /// Finds the first trailing comment on the same line as `span_pos`, allowing for `Mixed`
    /// style comments to appear before it.
    ///
    /// Returns the comment and its index in the buffer.
    pub fn peek_trailing(
        &self,
        sm: &SourceMap,
        span_pos: BytePos,
        next_pos: Option<BytePos>,
    ) -> Option<(&Comment, usize)> {
        let span_line = sm.lookup_char_pos(span_pos).line;
        for (i, cmnt) in self.iter().enumerate() {
            // If we have moved to the next line, we can stop.
            let comment_line = sm.lookup_char_pos(cmnt.pos()).line;
            if comment_line != span_line {
                break;
            }

            // The comment must start after the given span position.
            if cmnt.pos() < span_pos {
                continue;
            }

            // The comment must be before the next element.
            if cmnt.pos() >= next_pos.unwrap_or_else(|| cmnt.pos() + BytePos(1)) {
                break;
            }

            // Stop when we find a trailing or a non-mixed comment
            match cmnt.style {
                CommentStyle::Mixed => continue,
                CommentStyle::Trailing => return Some((cmnt, i)),
                _ => break,
            }
        }
        None
    }
}

struct CommentGatherer<'ast> {
    sf: &'ast SourceFile,
    sm: &'ast SourceMap,
    text: &'ast str,
    start_bpos: BytePos,
    pos: usize,
    comments: Vec<Comment>,
    code_to_the_left: bool,
    disabled_block_depth: usize,
    tab_width: Option<usize>,
}

impl<'ast> CommentGatherer<'ast> {
    fn new(
        sf: &'ast SourceFile,
        sm: &'ast SourceMap,
        normalize_cmnts: bool,
        tab_width: Option<usize>,
    ) -> Self {
        Self {
            sf,
            sm,
            text: sf.src.as_str(),
            start_bpos: sf.start_pos,
            pos: 0,
            comments: Vec::new(),
            code_to_the_left: false,
            disabled_block_depth: if normalize_cmnts { 0 } else { 1 },
            tab_width,
        }
    }

    /// Consumes the gatherer and returns the collected comments.
    fn gather(mut self) -> Self {
        for token in solar::parse::Cursor::new(&self.text[self.pos..]) {
            self.process_token(token);
        }
        self
    }

    /// Post-processes a list of comments to group consecutive comments.
    ///
    /// Necessary for properly indenting multi-line trailing comments, which would
    /// otherwise be parsed as a `Trailing` followed by several `Isolated`.
    fn group(self) -> Vec<Comment> {
        let mut processed = Vec::new();
        let mut cursor = self.comments.into_iter().peekable();

        while let Some(mut current) = cursor.next() {
            if current.kind == CommentKind::Line
                && (current.style.is_trailing() || current.style.is_isolated())
            {
                let mut ref_line = self.sm.lookup_char_pos(current.span.hi()).line;
                while let Some(next_comment) = cursor.peek() {
                    if !next_comment.style.is_isolated()
                        || next_comment.kind != CommentKind::Line
                        || ref_line + 1 != self.sm.lookup_char_pos(next_comment.span.lo()).line
                    {
                        break;
                    }

                    let next_to_merge = cursor.next().unwrap();
                    current.lines.extend(next_to_merge.lines);
                    current.span = current.span.to(next_to_merge.span);
                    ref_line += 1;
                }
            }

            processed.push(current);
        }

        processed
    }

    /// Creates a `Span` relative to the source file's start position.
    fn make_span(&self, range: std::ops::Range<usize>) -> Span {
        Span::new(self.start_bpos + range.start as u32, self.start_bpos + range.end as u32)
    }

    /// Processes a single token from the source.
    fn process_token(&mut self, token: solar::parse::lexer::token::RawToken) {
        let token_range = self.pos..self.pos + token.len as usize;
        let span = self.make_span(token_range.clone());
        let token_text = &self.text[token_range];

        // Keep track of disabled blocks
        if token_text.trim_start().contains(DISABLE_START) {
            self.disabled_block_depth += 1;
        } else if token_text.trim_start().contains(DISABLE_END) {
            self.disabled_block_depth -= 1;
        }

        match token.kind {
            TokenKind::Whitespace => {
                if let Some(mut idx) = token_text.find('\n') {
                    self.code_to_the_left = false;

                    while let Some(next_newline) = token_text[idx + 1..].find('\n') {
                        idx += 1 + next_newline;
                        let pos = self.pos + idx;
                        self.comments.push(Comment {
                            is_doc: false,
                            kind: CommentKind::Line,
                            style: CommentStyle::BlankLine,
                            lines: vec![],
                            span: self.make_span(pos..pos),
                        });
                        // If not disabled, early-exit as we want only a single blank line.
                        if self.disabled_block_depth == 0 {
                            break;
                        }
                    }
                }
            }
            TokenKind::BlockComment { is_doc, .. } => {
                let code_to_the_right = !matches!(
                    self.text[self.pos + token.len as usize..].chars().next(),
                    Some('\r' | '\n')
                );
                let style = match (self.code_to_the_left, code_to_the_right) {
                    (_, true) => CommentStyle::Mixed,
                    (false, false) => CommentStyle::Isolated,
                    (true, false) => CommentStyle::Trailing,
                };
                let kind = CommentKind::Block;

                // Count the number of chars since the start of the line by rescanning.
                let pos_in_file = self.start_bpos + BytePos(self.pos as u32);
                let line_begin_in_file = line_begin_pos(self.sf, pos_in_file);
                let line_begin_pos = (line_begin_in_file - self.start_bpos).to_usize();
                let mut col = CharPos(self.text[line_begin_pos..self.pos].chars().count());

                // To preserve alignment in multi-line non-doc comments, normalize the block based
                // on its least-indented line.
                if !is_doc && token_text.contains('\n') {
                    col = token_text.lines().skip(1).fold(col, |min, line| {
                        if line.is_empty() {
                            return min;
                        }
                        std::cmp::min(
                            CharPos(line.chars().count() - line.trim_start().chars().count()),
                            min,
                        )
                    })
                };

                let lines = self.split_block_comment_into_lines(token_text, is_doc, col);
                self.comments.push(Comment { is_doc, kind, style, lines, span })
            }
            TokenKind::LineComment { is_doc } => {
                let line =
                    if self.disabled_block_depth != 0 { token_text } else { token_text.trim_end() };
                self.comments.push(Comment {
                    is_doc,
                    kind: CommentKind::Line,
                    style: if self.code_to_the_left {
                        CommentStyle::Trailing
                    } else {
                        CommentStyle::Isolated
                    },
                    lines: vec![line.into()],
                    span,
                });
            }
            _ => {
                self.code_to_the_left = true;
            }
        }
        self.pos += token.len as usize;
    }

    /// Splits a block comment into lines, ensuring that each line is properly formatted.
    fn split_block_comment_into_lines(
        &self,
        text: &str,
        is_doc: bool,
        col: CharPos,
    ) -> Vec<String> {
        // if formatting is disabled, return as is
        if self.disabled_block_depth != 0 {
            return vec![text.into()];
        }

        let mut res: Vec<String> = vec![];
        let mut lines = text.lines();
        if let Some(line) = lines.next() {
            let line = line.trim_end();
            // Ensure first line of a doc comment only has the `/**` decorator
            if is_doc && let Some((_, second)) = line.split_once("/**") {
                res.push("/**".to_string());
                if !second.trim().is_empty() {
                    let line = normalize_block_comment_ws(second, col).trim_end();
                    // Ensure last line of a doc comment only has the `*/` decorator
                    if let Some((first, _)) = line.split_once("*/") {
                        if !first.trim().is_empty() {
                            res.push(format_doc_block_comment(first.trim_end(), self.tab_width));
                        }
                        res.push(" */".to_string());
                    } else {
                        res.push(format_doc_block_comment(line.trim_end(), self.tab_width));
                    }
                }
            } else {
                res.push(line.to_string());
            }
        }

        for (pos, line) in lines.delimited() {
            let line = normalize_block_comment_ws(line, col).trim_end().to_string();
            if !is_doc {
                res.push(line);
                continue;
            }
            if !pos.is_last {
                res.push(format_doc_block_comment(&line, self.tab_width));
            } else {
                // Ensure last line of a doc comment only has the `*/` decorator
                if let Some((first, _)) = line.split_once("*/")
                    && !first.trim().is_empty()
                {
                    res.push(format_doc_block_comment(first.trim_end(), self.tab_width));
                }
                res.push(" */".to_string());
            }
        }
        res
    }
}

/// Returns `None` if the first `col` chars of `s` contain a non-whitespace char.
/// Otherwise returns `Some(k)` where `k` is first char offset after that leading
/// whitespace. Note that `k` may be outside bounds of `s`.
fn all_whitespace(s: &str, col: CharPos) -> Option<usize> {
    let mut idx = 0;
    for (i, ch) in s.char_indices().take(col.to_usize()) {
        if !ch.is_whitespace() {
            return None;
        }
        idx = i + ch.len_utf8();
    }
    Some(idx)
}

/// Returns `Some(k)` where `k` is the byte offset of the first non-whitespace char. Returns `k = 0`
/// if `s` starts with a non-whitespace char. If `s` only contains whitespaces, returns `None`.
fn first_non_whitespace(s: &str) -> Option<usize> {
    let mut len = 0;
    for (i, ch) in s.char_indices() {
        if ch.is_whitespace() {
            len = ch.len_utf8()
        } else {
            return if i == 0 { Some(0) } else { Some(i + 1 - len) };
        }
    }
    None
}

/// Returns a slice of `s` with a whitespace prefix removed based on `col`. If the first `col` chars
/// of `s` are all whitespace, returns a slice starting after that prefix.
fn normalize_block_comment_ws(s: &str, col: CharPos) -> &str {
    let len = s.len();
    if let Some(col) = all_whitespace(s, col) {
        return if col < len { &s[col..] } else { "" };
    }
    if let Some(col) = first_non_whitespace(s) {
        return &s[col..];
    }
    s
}

/// Formats a doc block comment line so that they have the ` *` decorator.
fn format_doc_block_comment(line: &str, tab_width: Option<usize>) -> String {
    if line.is_empty() {
        return (" *").to_string();
    }

    if let Some((_, rest_of_line)) = line.split_once("*") {
        if rest_of_line.is_empty() {
            (" *").to_string()
        } else if let Some(tab_width) = tab_width {
            let mut normalized = String::from(" *");
            line_with_tabs(
                &mut normalized,
                rest_of_line,
                tab_width,
                Some(Consolidation::MinOneTab),
            );
            normalized
        } else {
            format!(" *{rest_of_line}",)
        }
    } else if let Some(tab_width) = tab_width {
        let mut normalized = String::from(" *\t");
        line_with_tabs(&mut normalized, line, tab_width, Some(Consolidation::WithoutSpaces));
        normalized
    } else {
        format!(" * {line}")
    }
}

pub enum Consolidation {
    MinOneTab,
    WithoutSpaces,
}

/// Normalizes the leading whitespace of a string slice according to a given tab width.
///
/// It aggregates and converts leading whitespace (spaces and tabs) into a representation that
/// maximizes the amount of tabs.
pub fn line_with_tabs(
    output: &mut String,
    line: &str,
    tab_width: usize,
    strategy: Option<Consolidation>,
) {
    // Find the end of the leading whitespace (any sequence of spaces and tabs)
    let first_non_ws = line.find(|c| c != ' ' && c != '\t').unwrap_or(line.len());
    let (leading_ws, rest_of_line) = line.split_at(first_non_ws);

    // Compute its equivalent length and derive the required amount of tabs and spaces
    let total_width =
        leading_ws.chars().fold(0, |width, c| width + if c == ' ' { 1 } else { tab_width });
    let (mut num_tabs, mut num_spaces) = (total_width / tab_width, total_width % tab_width);

    // Adjust based on the desired config
    match strategy {
        Some(Consolidation::MinOneTab) => {
            if num_tabs == 0 && num_spaces != 0 {
                (num_tabs, num_spaces) = (1, 0);
            } else if num_spaces != 0 {
                (num_tabs, num_spaces) = (num_tabs + 1, 0);
            }
        }
        Some(Consolidation::WithoutSpaces) => {
            if num_spaces != 0 {
                (num_tabs, num_spaces) = (num_tabs + 1, 0);
            }
        }
        None => (),
    };

    // Append the normalized indentation and the rest of the line to the output
    output.extend(std::iter::repeat_n('\t', num_tabs));
    output.extend(std::iter::repeat_n(' ', num_spaces));
    output.push_str(rest_of_line);
}

/// Estimates the display width of a string, accounting for tabs.
pub fn estimate_line_width(line: &str, tab_width: usize) -> usize {
    line.chars().fold(0, |width, c| width + if c == '\t' { tab_width } else { 1 })
}

/// Returns the `BytePos` of the beginning of the current line.
fn line_begin_pos(sf: &SourceFile, pos: BytePos) -> BytePos {
    let pos = sf.relative_position(pos);
    let line_index = sf.lookup_line(pos).unwrap();
    let line_start_pos = sf.lines()[line_index];
    sf.absolute_position(line_start_pos)
}
