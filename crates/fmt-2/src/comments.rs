use crate::iter::IterDelimited;

use super::comment::{Comment, CommentStyle};
use solar_parse::{
    ast::{CommentKind, Span},
    interface::{source_map::SourceFile, BytePos, CharPos, SourceMap},
    lexer::token::RawTokenKind as TokenKind,
};
use std::fmt;

pub struct Comments {
    comments: std::vec::IntoIter<Comment>,
}

impl fmt::Debug for Comments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Comments")?;
        f.debug_list().entries(self.iter()).finish()
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
fn format_doc_block_comment(line: &str) -> String {
    if line.is_empty() {
        return (" *").to_string();
    }

    if let Some((_, second)) = line.split_once("*") {
        if second.is_empty() {
            (" *").to_string()
        } else {
            format!(" *{second}")
        }
    } else {
        format!(" * {}", line)
    }
}

/// Splits a block comment into lines, ensuring that each line is properly formatted.
fn split_block_comment_into_lines(text: &str, is_doc: bool, col: CharPos) -> Vec<String> {
    let mut res: Vec<String> = vec![];
    let mut lines = text.lines();
    if let Some(line) = lines.next() {
        let line = line.trim_end();
        // Ensure first line of a doc comment only has the `/**` decorator
        if let Some((_, second)) = line.split_once("/**") {
            res.push("/**".to_string());
            if !second.trim().is_empty() {
                let line = normalize_block_comment_ws(second, col).trim_end();
                // Ensure last line of a doc comment only has the `*/` decorator
                if let Some((first, _)) = line.split_once("*/") {
                    if !first.trim().is_empty() {
                        res.push(format_doc_block_comment(first.trim_end()));
                    }
                    res.push(" */".to_string());
                } else {
                    res.push(format_doc_block_comment(line.trim_end()));
                }
            }
        } else {
            res.push(line.to_string());
        }
    }

    for (pos, line) in lines.into_iter().delimited() {
        let line = normalize_block_comment_ws(line, col).trim_end().to_string();
        if !is_doc {
            res.push(line);
            continue;
        }
        if !pos.is_last {
            res.push(format_doc_block_comment(&line));
        } else {
            if let Some((first, _)) = line.split_once("*/") {
                if !first.trim().is_empty() {
                    res.push(format_doc_block_comment(first));
                }
            }
            res.push(" */".to_string());
        }
    }
    res
}

/// Returns the `BytePos` of the beginning of the current line.
fn line_begin_pos(sf: &SourceFile, pos: BytePos) -> BytePos {
    let pos = sf.relative_position(pos);
    let line_index = sf.lookup_line(pos).unwrap();
    let line_start_pos = sf.lines()[line_index];
    sf.absolute_position(line_start_pos)
}

fn gather_comments(sf: &SourceFile) -> Vec<Comment> {
    let text = sf.src.as_str();
    let start_bpos = sf.start_pos;
    let mut pos = 0;
    let mut comments: Vec<Comment> = Vec::new();
    let mut code_to_the_left = false;

    let make_span = |range: std::ops::Range<usize>| {
        Span::new(start_bpos + range.start as u32, start_bpos + range.end as u32)
    };

    /*
    if let Some(shebang_len) = strip_shebang(text) {
        comments.push(Comment {
            style: CommentStyle::Isolated,
            lines: vec![text[..shebang_len].to_string()],
            pos: start_bpos,
        });
        pos += shebang_len;
    }
    */

    for token in solar_parse::Cursor::new(&text[pos..]) {
        let token_range = pos..pos + token.len as usize;
        let span = make_span(token_range.clone());
        let token_text = &text[token_range];
        match token.kind {
            TokenKind::Whitespace => {
                if let Some(mut idx) = token_text.find('\n') {
                    code_to_the_left = false;

                    // NOTE(dani): this used to be `while`, but we want only a single blank line.
                    if let Some(next_newline) = token_text[idx + 1..].find('\n') {
                        idx += 1 + next_newline;
                        let pos = pos + idx;
                        comments.push(Comment {
                            is_doc: false,
                            kind: CommentKind::Line,
                            style: CommentStyle::BlankLine,
                            lines: vec![],
                            span: make_span(pos..pos),
                        });
                    }
                }
            }
            TokenKind::BlockComment { is_doc, .. } => {
                let code_to_the_right =
                    !matches!(text[pos + token.len as usize..].chars().next(), Some('\r' | '\n'));
                let style = match (code_to_the_left, code_to_the_right) {
                    (false, false) => CommentStyle::Isolated,
                    // NOTE(rusowsky): unlike with `Trailing` comments, which are always printed
                    // with a hardbreak, `Mixed` comments should be followed by a space and defer
                    // breaks to the printer. Because of that, non-isolated code blocks are labeled
                    // as mixed.
                    _ => CommentStyle::Mixed,
                };
                let kind = CommentKind::Block;

                // Count the number of chars since the start of the line by rescanning.
                let pos_in_file = start_bpos + BytePos(pos as u32);
                let line_begin_in_file = line_begin_pos(sf, pos_in_file);
                let line_begin_pos = (line_begin_in_file - start_bpos).to_usize();
                let col = CharPos(text[line_begin_pos..pos].chars().count());

                let lines = split_block_comment_into_lines(token_text, is_doc, col);
                comments.push(Comment { is_doc, kind, style, lines, span })
            }
            TokenKind::LineComment { is_doc } => {
                comments.push(Comment {
                    is_doc,
                    kind: CommentKind::Line,
                    style: if code_to_the_left {
                        CommentStyle::Trailing
                    } else {
                        CommentStyle::Isolated
                    },
                    lines: vec![token_text.trim_end().to_string()],
                    span,
                });
            }
            _ => {
                code_to_the_left = true;
            }
        }
        pos += token.len as usize;
    }

    comments
}

impl Comments {
    pub fn new(sf: &SourceFile) -> Self {
        Self { comments: gather_comments(sf).into_iter() }
    }

    pub fn peek(&self) -> Option<&Comment> {
        self.comments.as_slice().first()
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Comment> {
        self.comments.next()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Comment> {
        self.comments.as_slice().iter()
    }

    pub fn peek_trailing_comment(
        &self,
        sm: &SourceMap,
        span_pos: BytePos,
        next_pos: Option<BytePos>,
    ) -> Option<&Comment> {
        if let Some(cmnt) = self.peek() {
            if !(cmnt.style.is_trailing() || cmnt.style.is_mixed()) {
                return None;
            }
            let span_line = sm.lookup_char_pos(span_pos);
            let comment_line = sm.lookup_char_pos(cmnt.pos());
            let next = next_pos.unwrap_or_else(|| cmnt.pos() + BytePos(1));
            if span_pos <= cmnt.pos() && cmnt.pos() < next && span_line.line == comment_line.line {
                return Some(cmnt);
            }
        }

        None
    }

    pub fn trailing_comment(
        &mut self,
        sm: &SourceMap,
        span_pos: BytePos,
        next_pos: Option<BytePos>,
    ) -> Option<Comment> {
        match self.peek_trailing_comment(sm, span_pos, next_pos) {
            Some(_) => self.next(),
            None => None,
        }
    }
}
