use crate::solang_ext::*;
use itertools::Itertools;
use solang_parser::pt::*;

/// The type of a Comment
#[derive(Debug, Clone, Copy)]
pub enum CommentType {
    /// A Line comment (e.g. `// ...`)
    Line,
    /// A Block comment (e.g. `/* ... */`)
    Block,
}

/// The comment position
#[derive(Debug, Clone, Copy)]
pub enum CommentPosition {
    /// Comes before the code it describes
    Prefix,
    /// Comes after the code it describes
    Postfix,
}

/// Comment with additional metadata
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
        Self { comment: comment.trim_end().to_string(), ty, loc, position, has_newline_before }
    }

    /// Construct a comment with metadata by analyzing its surrounding source code
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
                            let this_indent = this_line.len();
                            let mut this_indent_larger = this_indent > 0;
                            let mut next_indent = this_indent;
                            for ch in src[comment.loc().end()..].non_comment_chars() {
                                if ch == '\n' {
                                    next_indent = 0;
                                } else if ch.is_whitespace() {
                                    next_indent += 1;
                                } else {
                                    this_indent_larger = this_indent > next_indent;
                                    break
                                }
                            }
                            if this_indent_larger {
                                // next line has a smaller indent
                                (CommentPosition::Postfix, false)
                            } else {
                                // next line has same or equal indent
                                (CommentPosition::Prefix, false)
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

/// A list of comments
/// NOTE: comments are stored in reverse order for easy removal
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

    /// Remove any prefix comments that occur before the byte offset in the src
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

    /// Remove any postfix comments that occur before the byte offset in the src
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

    /// Remove any comments that occur before the byte offset in the src
    pub(crate) fn remove_comments_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        let mut out = self.remove_prefixes_before(byte);
        out.append(&mut self.remove_postfixes_before(byte));
        out.sort_by_key(|comment| comment.loc.start());
        out
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CommentState {
    None,
    LineStart1,
    LineStart2,
    Line,
    BlockStart1,
    BlockStart2,
    Block,
    BlockEnd1,
    BlockEnd2,
}

impl Default for CommentState {
    fn default() -> Self {
        CommentState::None
    }
}

/// An Iterator over characters and indexes in a string slice with information about the state of
/// comments
pub struct CommentStateCharIndices<'a> {
    iter: std::iter::Peekable<std::str::CharIndices<'a>>,
    state: CommentState,
}

impl<'a> CommentStateCharIndices<'a> {
    fn new(string: &'a str) -> Self {
        Self { iter: string.char_indices().peekable(), state: CommentState::None }
    }
    pub fn with_state(mut self, state: CommentState) -> Self {
        self.state = state;
        self
    }
}

impl<'a> Iterator for CommentStateCharIndices<'a> {
    type Item = (CommentState, usize, char);
    fn next(&mut self) -> Option<Self::Item> {
        let (idx, ch) = self.iter.next()?;
        match self.state {
            CommentState::None => {
                if ch == '/' {
                    match self.iter.peek() {
                        Some((_, '/')) => {
                            self.state = CommentState::LineStart1;
                        }
                        Some((_, '*')) => {
                            self.state = CommentState::BlockStart1;
                        }
                        _ => {}
                    }
                }
            }
            CommentState::LineStart1 => {
                self.state = CommentState::LineStart2;
            }
            CommentState::LineStart2 => {
                self.state = CommentState::Line;
            }
            CommentState::Line => {
                if ch == '\n' {
                    self.state = CommentState::None;
                }
            }
            CommentState::BlockStart1 => {
                self.state = CommentState::BlockStart2;
            }
            CommentState::BlockStart2 => {
                self.state = CommentState::Block;
            }
            CommentState::Block => {
                if ch == '*' {
                    if let Some((_, '/')) = self.iter.peek() {
                        self.state = CommentState::BlockEnd1;
                    }
                }
            }
            CommentState::BlockEnd1 => {
                self.state = CommentState::BlockEnd2;
            }
            CommentState::BlockEnd2 => {
                self.state = CommentState::None;
            }
        }
        Some((self.state, idx, ch))
    }
}

/// An Iterator over characters in a string slice which are not a apart of comments
pub struct NonCommentChars<'a>(CommentStateCharIndices<'a>);

impl<'a> Iterator for NonCommentChars<'a> {
    type Item = char;
    fn next(&mut self) -> Option<Self::Item> {
        for (state, _, ch) in self.0.by_ref() {
            if state == CommentState::None {
                return Some(ch)
            }
        }
        None
    }
}

/// Helpers for iterating over non-comment characters
pub trait CommentStringExt {
    fn comment_state_char_indices(&self) -> CommentStateCharIndices;
    fn non_comment_chars(&self) -> NonCommentChars {
        NonCommentChars(self.comment_state_char_indices())
    }
    fn trim_comments(&self) -> String {
        self.non_comment_chars().collect()
    }
}

impl<T> CommentStringExt for T
where
    T: AsRef<str>,
{
    fn comment_state_char_indices(&self) -> CommentStateCharIndices {
        CommentStateCharIndices::new(self.as_ref())
    }
}

impl CommentStringExt for str {
    fn comment_state_char_indices(&self) -> CommentStateCharIndices {
        CommentStateCharIndices::new(self)
    }
}
