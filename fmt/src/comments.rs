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
                                .find(|line| !line.trim_comments().trim().is_empty());
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

enum CommentState {
    None,
    Line,
    Block,
}

pub struct NonCommentChars<'a> {
    iter: std::iter::Peekable<std::str::Chars<'a>>,
    state: CommentState,
}

impl<'a> Iterator for NonCommentChars<'a> {
    type Item = char;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ch) = self.iter.next() {
            match self.state {
                CommentState::None => match ch {
                    '/' => match self.iter.peek() {
                        Some('/') => {
                            self.iter.next();
                            self.state = CommentState::Line;
                        }
                        Some('*') => {
                            self.iter.next();
                            self.state = CommentState::Block;
                        }
                        _ => return Some(ch),
                    },
                    _ => return Some(ch),
                },
                CommentState::Line => {
                    if ch == '\n' {
                        self.state = CommentState::None;
                        return Some('\n')
                    }
                }
                CommentState::Block => {
                    if ch == '*' {
                        if let Some('/') = self.iter.next() {
                            self.state = CommentState::None
                        }
                    }
                }
            }
        }
        None
    }
}

pub trait CommentStringExt {
    fn non_comment_chars(&self) -> NonCommentChars;
    fn trim_comments(&self) -> String {
        self.non_comment_chars().collect()
    }
}

impl<T> CommentStringExt for T
where
    T: AsRef<str>,
{
    fn non_comment_chars(&self) -> NonCommentChars {
        NonCommentChars { iter: self.as_ref().chars().peekable(), state: CommentState::None }
    }
}

impl CommentStringExt for str {
    fn non_comment_chars(&self) -> NonCommentChars {
        NonCommentChars { iter: self.chars().peekable(), state: CommentState::None }
    }
}
