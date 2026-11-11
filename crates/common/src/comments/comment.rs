//! Modified from [`rustc_ast::util::comments`](https://github.com/rust-lang/rust/blob/07d3fd1d9b9c1f07475b96a9d168564bf528db68/compiler/rustc_ast/src/util/comments.rs).

use super::inline_config::{InlineConfigItem, InvalidInlineConfigItem};
use solar::parse::{
    ast::{CommentKind, Span},
    interface::BytePos,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl CommentStyle {
    pub const fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }
    pub const fn is_trailing(&self) -> bool {
        matches!(self, Self::Trailing)
    }
    pub const fn is_isolated(&self) -> bool {
        matches!(self, Self::Isolated)
    }
    pub const fn is_blank(&self) -> bool {
        matches!(self, Self::BlankLine)
    }
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

    pub const fn prefix(&self) -> Option<&'static str> {
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

    pub const fn suffix(&self) -> Option<&'static str> {
        if self.lines.is_empty() {
            return None;
        }
        match self.kind {
            CommentKind::Line => None,
            CommentKind::Block => Some("*/"),
        }
    }

    /// Parses formatter directives on the first and last lines of the comment.
    pub fn format_directives(
        &self,
    ) -> impl Iterator<Item = Result<InlineConfigItem<()>, InvalidInlineConfigItem>> + '_ {
        let mut lines = self.lines.iter().flat_map(|line| line.lines());
        let first = lines.next();
        let last = lines.last();
        [first, last].into_iter().flatten().filter_map(|line| self.parse_format_directive(line))
    }

    fn parse_format_directive(
        &self,
        mut text: &str,
    ) -> Option<Result<InlineConfigItem<()>, InvalidInlineConfigItem>> {
        if let Some(prefix) = self.prefix() {
            text = text.strip_prefix(prefix).unwrap_or(text);
        }
        if let Some(suffix) = self.suffix() {
            text = text.strip_suffix(suffix).unwrap_or(text);
        }
        Some(text.trim_start().strip_prefix("forgefmt:")?.trim().parse())
    }
}
