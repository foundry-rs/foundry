use crate::solang_ext::*;
use itertools::Itertools;
use solang_parser::pt::*;

fn trim_comments(s: &str) -> String {
    enum CommentState {
        None,
        Line,
        Block,
    }
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut state = CommentState::None;
    while let Some(ch) = chars.next() {
        match state {
            CommentState::None => match ch {
                '/' => match chars.peek() {
                    Some('/') => {
                        chars.next();
                        state = CommentState::Line;
                    }
                    Some('*') => {
                        chars.next();
                        state = CommentState::Block;
                    }
                    _ => out.push(ch),
                },
                _ => out.push(ch),
            },
            CommentState::Line => {
                if ch == '\n' {
                    state = CommentState::None;
                    out.push('\n')
                }
            }
            CommentState::Block => {
                if ch == '*' {
                    if let Some('/') = chars.next() {
                        state = CommentState::None
                    }
                }
            }
        }
    }
    out
}

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
pub struct CommentWithMetadata {
    pub ty: CommentType,
    pub loc: Loc,
    pub has_newline_before: bool,
    pub comment: String,
    pub position: CommentPosition,
}

impl CommentWithMetadata {
    fn new(comment: Comment, position: CommentPosition, has_newline_before: bool) -> Self {
        let (ty, loc, comment) = match comment {
            Comment::Line(loc, comment) => (CommentType::Line, loc, comment),
            Comment::Block(loc, comment) => (CommentType::Block, loc, comment),
        };
        Self { ty, loc, comment, position, has_newline_before }
    }
    fn from_comment_and_src(comment: Comment, src: &str) -> Self {
        let (position, has_newline_before) = {
            let src_before = &src[..comment.loc().start()];
            if src_before.is_empty() {
                // beginning of code
                (CommentPosition::Prefix, false)
            } else {
                let mut lines_before = src_before.lines().rev();
                let this_line =
                    if src_before.ends_with('\n') { "" } else { lines_before.next().unwrap() };
                if this_line.trim_start().is_empty() {
                    // comment sits on a new line
                    if let Some(last_line) = lines_before.next() {
                        if last_line.trim_start().is_empty() {
                            // line before is empty
                            (CommentPosition::Prefix, true)
                        } else {
                            // line has something
                            let next_code = src[comment.loc().end()..]
                                .lines()
                                .find(|line| !trim_comments(line).trim().is_empty());
                            if let Some(next_code) = next_code {
                                let next_indent =
                                    next_code.chars().position(|ch| !ch.is_whitespace()).unwrap();
                                if this_line.len() > next_indent {
                                    // next line has a smaller indent
                                    (CommentPosition::Postfix, false)
                                } else {
                                    // next line has same or equal indent
                                    (CommentPosition::Prefix, false)
                                }
                            } else {
                                // end of file
                                (CommentPosition::Postfix, false)
                            }
                        }
                    } else {
                        // beginning of file
                        (CommentPosition::Prefix, false)
                    }
                } else {
                    // comment is after some code
                    (CommentPosition::Postfix, false)
                }
            }
        };
        Self::new(comment, position, has_newline_before)
    }
    pub fn is_line(&self) -> bool {
        matches!(self.ty, CommentType::Line)
    }
    pub fn is_prefix(&self) -> bool {
        matches!(self.position, CommentPosition::Prefix)
    }
    pub fn is_before(&self, byte: usize) -> bool {
        self.loc.start() < byte
    }
}

/// Comments are stored in reverse order for easy removal
#[derive(Debug, Clone)]
pub struct Comments {
    prefixes: Vec<CommentWithMetadata>,
    postfixes: Vec<CommentWithMetadata>,
}

impl Comments {
    pub fn new(comments: Vec<Comment>, src: &str) -> Self {
        let mut prefixes = Vec::new();
        let mut postfixes = Vec::new();

        for comment in comments.into_iter().rev() {
            let comment = CommentWithMetadata::from_comment_and_src(comment, src);
            if comment.is_prefix() {
                prefixes.push(comment)
            } else {
                postfixes.push(comment)
            }
        }
        Self { prefixes, postfixes }
    }

    pub(crate) fn get_comments_before(&self, byte: usize) -> Vec<&CommentWithMetadata> {
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

    pub(crate) fn remove_prefixes_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        let mut prefixes = self.prefixes.split_off(
            self.prefixes
                .iter()
                .find_position(|comment| comment.is_before(byte))
                .map(|(idx, _)| idx)
                .unwrap_or_else(|| self.prefixes.len()),
        );
        prefixes.reverse();
        prefixes
    }

    pub(crate) fn remove_postfixes_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        let mut postfixes = self.postfixes.split_off(
            self.postfixes
                .iter()
                .find_position(|comment| comment.is_before(byte))
                .map(|(idx, _)| idx)
                .unwrap_or_else(|| self.postfixes.len()),
        );
        postfixes.reverse();
        postfixes
    }

    pub(crate) fn remove_comments_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        let mut out = self.remove_prefixes_before(byte);
        out.append(&mut self.remove_postfixes_before(byte));
        out.sort_by_key(|comment| comment.loc.start());
        out
    }
}
