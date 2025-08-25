//! Modified from [`rustc_ast::util::comments`](https://github.com/rust-lang/rust/blob/07d3fd1d9b9c1f07475b96a9d168564bf528db68/compiler/rustc_ast/src/util/comments.rs).

use solar_parse::{
    ast::{CommentKind, Span},
    interface::BytePos,
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
