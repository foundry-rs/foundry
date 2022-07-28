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
    pub indent_len: usize,
    pub comment: String,
    pub position: CommentPosition,
}

impl CommentWithMetadata {
    fn new(
        comment: Comment,
        position: CommentPosition,
        has_newline_before: bool,
        indent_len: usize,
    ) -> Self {
        let (ty, loc, comment) = match comment {
            Comment::Line(loc, comment) => (CommentType::Line, loc, comment),
            Comment::Block(loc, comment) => (CommentType::Block, loc, comment),
        };
        Self {
            comment: comment.trim_end().to_string(),
            ty,
            loc,
            position,
            has_newline_before,
            indent_len,
        }
    }

    /// Construct a comment with metadata by analyzing its surrounding source code
    fn from_comment_and_src(
        comment: Comment,
        src: &str,
        last_comment: Option<&CommentWithMetadata>,
    ) -> Self {
        let src_before = &src[..comment.loc().start()];
        let mut lines_before = src_before.lines().rev();
        let this_line =
            if src_before.ends_with('\n') { "" } else { lines_before.next().unwrap_or("") };
        let indent_len = this_line.chars().take_while(|c| c.is_whitespace()).count();

        let (position, has_newline_before) = {
            if src_before.is_empty() {
                // beginning of code
                (CommentPosition::Prefix, false)
            } else if this_line.trim_start().is_empty() {
                // comment sits on a new line
                if let Some(last_line) = lines_before.next() {
                    if last_line.trim_start().is_empty() {
                        // line before is empty
                        (CommentPosition::Prefix, true)
                    } else {
                        // line has something
                        let code_end = src_before
                            .comment_state_char_indices()
                            .filter_map(|(state, idx, ch)| {
                                if matches!(state, CommentState::None) && !ch.is_whitespace() {
                                    Some(idx)
                                } else {
                                    None
                                }
                            })
                            .last()
                            .unwrap_or(0);
                        // check if the last comment after code was a postfix comment
                        if last_comment
                            .filter(|last_comment| {
                                last_comment.loc.end() > code_end && !last_comment.is_prefix()
                            })
                            .is_some()
                        {
                            // get the indent size of the next item of code
                            let next_indent_len = src[comment.loc().end()..]
                                .non_comment_chars()
                                .take_while(|ch| ch.is_whitespace())
                                .fold(
                                    indent_len,
                                    |indent, ch| if ch == '\n' { 0 } else { indent + 1 },
                                );
                            if indent_len > next_indent_len {
                                // the comment indent is bigger than the next code indent
                                (CommentPosition::Postfix, false)
                            } else {
                                // the comment indent is equal to or less than the next code
                                // indent
                                (CommentPosition::Prefix, false)
                            }
                        } else {
                            // if there is no postfix comment after the piece of code
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
        };

        Self::new(comment, position, has_newline_before, indent_len)
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
    pub fn new(mut comments: Vec<Comment>, src: &str) -> Self {
        let mut prefixes = Vec::new();
        let mut postfixes = Vec::new();
        let mut last_comment = None;

        comments.sort_by_key(|comment| comment.loc());
        for comment in comments {
            let comment =
                CommentWithMetadata::from_comment_and_src(comment, src, last_comment.as_ref());
            last_comment = Some(comment.clone());
            if comment.is_prefix() {
                prefixes.push(comment)
            } else {
                postfixes.push(comment)
            }
        }

        prefixes.reverse();
        postfixes.reverse();

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

/// The state of a character in a string with possible comments
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CommentState {
    /// character not in a comment
    None,
    /// First `/` in line comment start `"//"`
    LineStart1,
    /// Second `/` in  line comment start `"//"`
    LineStart2,
    /// Character in a line comment
    Line,
    /// `/` in block comment start `"/*"`
    BlockStart1,
    /// `*` in block comment start `"/*"`
    BlockStart2,
    /// Character in a block comment
    Block,
    /// `*` in block comment end `"*/"`
    BlockEnd1,
    /// `/` in block comment end `"*/"`
    BlockEnd2,
}

impl Default for CommentState {
    fn default() -> Self {
        CommentState::None
    }
}

/// An Iterator over characters and indices in a string slice with information about the state of
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

/// Helpers for iterating over comment containing strings
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
