use crate::solang_ext::*;
use itertools::Itertools;
use solang_parser::pt::*;

#[derive(Debug, Clone, Copy)]
pub enum CommentType {
    Line,
    Block,
}

#[derive(Debug, Clone, Copy)]
pub enum CommentPosition {
    Prefix,
    Postfix,
}

#[derive(Debug, Clone)]
pub struct DestructuredComment {
    pub ty: CommentType,
    pub loc: Loc,
    pub comment: String,
    pub position: CommentPosition,
}

impl DestructuredComment {
    fn new(comment: Comment, position: CommentPosition) -> Self {
        let (ty, loc, comment) = match comment {
            Comment::Line(loc, comment) => (CommentType::Line, loc, comment),
            Comment::Block(loc, comment) => (CommentType::Block, loc, comment),
        };
        Self { ty, loc, comment, position }
    }
    pub fn is_line(&self) -> bool {
        matches!(self.ty, CommentType::Line)
    }
    pub fn is_prefix(&self) -> bool {
        matches!(self.position, CommentPosition::Prefix)
    }
    pub fn needs_newline(&self) -> bool {
        self.is_line() || self.is_prefix()
    }
    pub fn is_before(&self, byte: usize) -> bool {
        self.loc.start() < byte
    }
}

/// Comments are stored in reverse order for easy removal
#[derive(Debug, Clone)]
pub struct Comments {
    prefixes: Vec<DestructuredComment>,
    postfixes: Vec<DestructuredComment>,
}

impl Comments {
    pub fn new(comments: Vec<Comment>, src: &str) -> Self {
        let mut prefixes = Vec::new();
        let mut postfixes = Vec::new();

        for comment in comments.into_iter().rev() {
            if Self::is_newline_comment(&comment, src) {
                prefixes.push(DestructuredComment::new(comment, CommentPosition::Prefix))
            } else {
                postfixes.push(DestructuredComment::new(comment, CommentPosition::Postfix))
            }
        }
        Self { prefixes, postfixes }
    }

    fn is_newline_comment(comment: &Comment, src: &str) -> bool {
        for ch in src[..comment.loc().start()].chars().rev() {
            if ch == '\n' {
                return true
            } else if !ch.is_whitespace() {
                return false
            }
        }
        true
    }

    pub(crate) fn pop_prefix(&mut self, byte: usize) -> Option<DestructuredComment> {
        if self.prefixes.last()?.is_before(byte) {
            Some(self.prefixes.pop().unwrap())
        } else {
            None
        }
    }

    pub(crate) fn peek_prefix(&mut self, byte: usize) -> Option<&DestructuredComment> {
        self.prefixes.last().and_then(
            |comment| {
                if comment.is_before(byte) {
                    Some(comment)
                } else {
                    None
                }
            },
        )
    }

    pub(crate) fn pop_postfix(&mut self, byte: usize) -> Option<DestructuredComment> {
        if self.postfixes.last()?.is_before(byte) {
            Some(self.postfixes.pop().unwrap())
        } else {
            None
        }
    }

    pub(crate) fn get_comments_before(&self, byte: usize) -> Vec<&DestructuredComment> {
        let mut out = self
            .prefixes
            .iter()
            .rev()
            .take_while(|comment| comment.is_before(byte))
            .chain(self.prefixes.iter().rev().take_while(|comment| comment.is_before(byte)))
            .collect::<Vec<_>>();
        out.sort_by_key(|comment| comment.loc.start());
        out
    }

    pub(crate) fn remove_comments_before(&mut self, byte: usize) -> Vec<DestructuredComment> {
        let mut out = self.prefixes.split_off(
            self.prefixes
                .iter()
                .find_position(|comment| comment.is_before(byte))
                .map(|(idx, _)| idx)
                .unwrap_or_else(|| self.prefixes.len()),
        );
        out.append(
            &mut self.postfixes.split_off(
                self.postfixes
                    .iter()
                    .find_position(|comment| comment.is_before(byte))
                    .map(|(idx, _)| idx)
                    .unwrap_or_else(|| self.postfixes.len()),
            ),
        );
        out.sort_by_key(|comment| comment.loc.start());
        out
    }

    pub(crate) fn drain(&mut self) -> Vec<DestructuredComment> {
        let mut out = std::mem::take(&mut self.prefixes);
        out.append(&mut self.postfixes);
        out.sort_by_key(|comment| comment.loc.start());
        out
    }
}
