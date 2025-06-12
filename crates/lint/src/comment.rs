//! Modified from [`rustc_ast::util::comments`](https://github.com/rust-lang/rust/blob/07d3fd1d9b9c1f07475b96a9d168564bf528db68/compiler/rustc_ast/src/util/comments.rs).

use solar_parse::{
    ast::{CommentKind, Span},
    interface::{BytePos, Symbol},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CommentStyle {
    /// No code on either side of each line of the comment
    Isolated,
    /// Code exists to the left of the comment
    Trailing,
    /// Code before /* foo */ and after the comment
    Mixed,
    /// Just a manual blank line "\n\n", for layout
    BlankLine,
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub lines: Vec<String>,
    pub span: Span,
    pub style: CommentStyle,
    pub is_doc: bool,
    pub kind: CommentKind,
}

impl Comment {
    pub fn pos(&self) -> BytePos {
        self.span.lo()
    }

    pub fn prefix(&self) -> Option<&'static str> {
        if self.lines.is_empty() {
            return None;
        }
        Some(match (self.kind, self.is_doc) {
            (CommentKind::Line, false) => "//",
            (CommentKind::Line, true) => "///",
            (CommentKind::Block, false) => "/*",
            (CommentKind::Block, true) => "/**",
        })
    }

    pub fn suffix(&self) -> Option<&'static str> {
        if self.lines.is_empty() {
            return None;
        }
        match self.kind {
            CommentKind::Line => None,
            CommentKind::Block => Some("*/"),
        }
    }
}

/// A fast conservative estimate on whether the string can contain documentation links.
/// A pair of square brackets `[]` must exist in the string, but we only search for the
/// opening bracket because brackets always go in pairs in practice.
#[inline]
pub fn may_have_doc_links(s: &str) -> bool {
    s.contains('[')
}

/// Makes a doc string more presentable to users.
/// Used by rustdoc and perhaps other tools, but not by rustc.
pub fn beautify_doc_string(data: Symbol, kind: CommentKind) -> Symbol {
    fn get_vertical_trim(lines: &[&str]) -> Option<(usize, usize)> {
        let mut i = 0;
        let mut j = lines.len();
        // first line of all-stars should be omitted
        if lines.first().is_some_and(|line| line.chars().all(|c| c == '*')) {
            i += 1;
        }

        // like the first, a last line of all stars should be omitted
        if j > i && !lines[j - 1].is_empty() && lines[j - 1].chars().all(|c| c == '*') {
            j -= 1;
        }

        if i != 0 || j != lines.len() {
            Some((i, j))
        } else {
            None
        }
    }

    fn get_horizontal_trim(lines: &[&str], kind: CommentKind) -> Option<String> {
        let mut i = usize::MAX;
        let mut first = true;

        // In case we have doc comments like `/**` or `/*!`, we want to remove stars if they are
        // present. However, we first need to strip the empty lines so they don't get in the middle
        // when we try to compute the "horizontal trim".
        let lines = match kind {
            CommentKind::Block => {
                // Whatever happens, we skip the first line.
                let mut i = lines
                    .first()
                    .map(|l| if l.trim_start().starts_with('*') { 0 } else { 1 })
                    .unwrap_or(0);
                let mut j = lines.len();

                while i < j && lines[i].trim().is_empty() {
                    i += 1;
                }
                while j > i && lines[j - 1].trim().is_empty() {
                    j -= 1;
                }
                &lines[i..j]
            }
            CommentKind::Line => lines,
        };

        for line in lines {
            for (j, c) in line.chars().enumerate() {
                if j > i || !"* \t".contains(c) {
                    return None;
                }
                if c == '*' {
                    if first {
                        i = j;
                        first = false;
                    } else if i != j {
                        return None;
                    }
                    break;
                }
            }
            if i >= line.len() {
                return None;
            }
        }
        Some(lines.first()?[..i].to_string())
    }

    let data_s = data.as_str();
    if data_s.contains('\n') {
        let mut lines = data_s.lines().collect::<Vec<&str>>();
        let mut changes = false;
        let lines = if let Some((i, j)) = get_vertical_trim(&lines) {
            changes = true;
            // remove whitespace-only lines from the start/end of lines
            &mut lines[i..j]
        } else {
            &mut lines
        };
        if let Some(horizontal) = get_horizontal_trim(lines, kind) {
            changes = true;
            // remove a "[ \t]*\*" block from each line, if possible
            for line in lines.iter_mut() {
                if let Some(tmp) = line.strip_prefix(&horizontal) {
                    *line = tmp;
                    if kind == CommentKind::Block &&
                        (*line == "*" || line.starts_with("* ") || line.starts_with("**"))
                    {
                        *line = &line[1..];
                    }
                }
            }
        }
        if changes {
            return Symbol::intern(&lines.join("\n"));
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar_parse::interface::enter;

    #[test]
    fn test_block_doc_comment_1() {
        enter(|| {
            let comment = "\n * Test \n **  Test\n *   Test\n";
            let stripped = beautify_doc_string(Symbol::intern(comment), CommentKind::Block);
            assert_eq!(stripped.as_str(), " Test \n*  Test\n   Test");
        })
    }

    #[test]
    fn test_block_doc_comment_2() {
        enter(|| {
            let comment = "\n * Test\n *  Test\n";
            let stripped = beautify_doc_string(Symbol::intern(comment), CommentKind::Block);
            assert_eq!(stripped.as_str(), " Test\n  Test");
        })
    }

    #[test]
    fn test_block_doc_comment_3() {
        enter(|| {
            let comment = "\n let a: *i32;\n *a = 5;\n";
            let stripped = beautify_doc_string(Symbol::intern(comment), CommentKind::Block);
            assert_eq!(stripped.as_str(), "let a: *i32;\n*a = 5;");
        })
    }

    #[test]
    fn test_line_doc_comment() {
        enter(|| {
            let stripped = beautify_doc_string(Symbol::intern(" test"), CommentKind::Line);
            assert_eq!(stripped.as_str(), " test");
            let stripped = beautify_doc_string(Symbol::intern("! test"), CommentKind::Line);
            assert_eq!(stripped.as_str(), "! test");
            let stripped = beautify_doc_string(Symbol::intern("test"), CommentKind::Line);
            assert_eq!(stripped.as_str(), "test");
            let stripped = beautify_doc_string(Symbol::intern("!test"), CommentKind::Line);
            assert_eq!(stripped.as_str(), "!test");
        })
    }

    #[test]
    fn test_doc_blocks() {
        enter(|| {
            let stripped = beautify_doc_string(
                Symbol::intern(" # Returns\n     *\n     "),
                CommentKind::Block,
            );
            assert_eq!(stripped.as_str(), " # Returns\n\n");

            let stripped = beautify_doc_string(
                Symbol::intern("\n     * # Returns\n     *\n     "),
                CommentKind::Block,
            );
            assert_eq!(stripped.as_str(), " # Returns\n\n");

            let stripped = beautify_doc_string(Symbol::intern("\n *     a\n "), CommentKind::Block);
            assert_eq!(stripped.as_str(), "     a\n");
        })
    }
}
