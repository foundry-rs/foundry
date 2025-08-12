mod comment;

use comment::{Comment, CommentStyle};
use solar_parse::{
    ast::{CommentKind, Span},
    interface::{BytePos, CharPos, SourceMap, source_map::SourceFile},
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

fn trim_whitespace_prefix(s: &str, col: CharPos) -> &str {
    let len = s.len();
    match all_whitespace(s, col) {
        Some(col) => {
            if col < len {
                &s[col..]
            } else {
                ""
            }
        }
        None => s,
    }
}

fn split_block_comment_into_lines(text: &str, col: CharPos) -> Vec<String> {
    let mut res: Vec<String> = vec![];
    let mut lines = text.lines();
    // just push the first line
    res.extend(lines.next().map(|it| it.to_string()));
    // for other lines, strip common whitespace prefix
    for line in lines {
        res.push(trim_whitespace_prefix(line, col).to_string())
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
                    (_, true) => CommentStyle::Mixed,
                    (false, false) => CommentStyle::Isolated,
                    (true, false) => CommentStyle::Trailing,
                };
                let kind = CommentKind::Block;

                // Count the number of chars since the start of the line by rescanning.
                let pos_in_file = start_bpos + BytePos(pos as u32);
                let line_begin_in_file = line_begin_pos(sf, pos_in_file);
                let line_begin_pos = (line_begin_in_file - start_bpos).to_usize();
                let col = CharPos(text[line_begin_pos..pos].chars().count());

                let lines = split_block_comment_into_lines(token_text, col);
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
                    lines: vec![token_text.to_string()],
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

    pub fn trailing_comment(
        &mut self,
        sm: &SourceMap,
        span: Span,
        next_pos: Option<BytePos>,
    ) -> Option<Comment> {
        if let Some(cmnt) = self.peek() {
            if cmnt.style != CommentStyle::Trailing {
                return None;
            }
            let span_line = sm.lookup_char_pos(span.hi());
            let comment_line = sm.lookup_char_pos(cmnt.pos());
            let next = next_pos.unwrap_or_else(|| cmnt.pos() + BytePos(1));
            if span.hi() < cmnt.pos() && cmnt.pos() < next && span_line.line == comment_line.line {
                return Some(self.next().unwrap());
            }
        }

        None
    }
}
