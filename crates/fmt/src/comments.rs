use crate::inline_config::{InlineConfigItem, InvalidInlineConfigItem};
use itertools::Itertools;
use solang_parser::pt::*;
use std::collections::VecDeque;

/// The type of a Comment
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentType {
    /// A Line comment (e.g. `// ...`)
    Line,
    /// A Block comment (e.g. `/* ... */`)
    Block,
    /// A Doc Line comment (e.g. `/// ...`)
    DocLine,
    /// A Doc Block comment (e.g. `/** ... */`)
    DocBlock,
}

/// The comment position
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentPosition {
    /// Comes before the code it describes
    Prefix,
    /// Comes after the code it describes
    Postfix,
}

/// Comment with additional metadata
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentWithMetadata {
    pub ty: CommentType,
    pub loc: Loc,
    pub has_newline_before: bool,
    pub indent_len: usize,
    pub comment: String,
    pub position: CommentPosition,
}

impl PartialOrd for CommentWithMetadata {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CommentWithMetadata {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.loc.cmp(&other.loc)
    }
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
            Comment::DocLine(loc, comment) => (CommentType::DocLine, loc, comment),
            Comment::DocBlock(loc, comment) => (CommentType::DocBlock, loc, comment),
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
    fn from_comment_and_src(comment: Comment, src: &str, last_comment: Option<&Self>) -> Self {
        let src_before = &src[..comment.loc().start()];
        if src_before.is_empty() {
            return Self::new(comment, CommentPosition::Prefix, false, 0)
        }

        let mut lines_before = src_before.lines().rev();
        let this_line =
            if src_before.ends_with('\n') { "" } else { lines_before.next().unwrap_or_default() };
        let indent_len = this_line.chars().take_while(|c| c.is_whitespace()).count();
        let last_line = lines_before.next().map(str::trim_start);

        if matches!(comment, Comment::DocLine(..) | Comment::DocBlock(..)) {
            return Self::new(
                comment,
                CommentPosition::Prefix,
                last_line.map_or(true, str::is_empty),
                indent_len,
            )
        }

        // TODO: this loop takes almost the entirety of the time spent in parsing, which is up to
        // 80% of `crate::fmt`
        let mut code_end = 0;
        for (state, idx, ch) in src_before.comment_state_char_indices() {
            if matches!(state, CommentState::None) && !ch.is_whitespace() {
                code_end = idx;
            }
        }

        let (position, has_newline_before) = if src_before[code_end..].contains('\n') {
            // comment sits on a line without code
            if let Some(last_line) = last_line {
                if last_line.is_empty() {
                    // line before is empty
                    (CommentPosition::Prefix, true)
                } else {
                    // line has something
                    // check if the last comment after code was a postfix comment
                    if last_comment
                        .map_or(false, |last| last.loc.end() > code_end && !last.is_prefix())
                    {
                        // get the indent size of the next item of code
                        let next_indent_len = src[comment.loc().end()..]
                            .non_comment_chars()
                            .take_while(|ch| ch.is_whitespace())
                            .fold(indent_len, |indent, ch| if ch == '\n' { 0 } else { indent + 1 });
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
        };

        Self::new(comment, position, has_newline_before, indent_len)
    }

    pub fn is_line(&self) -> bool {
        matches!(self.ty, CommentType::Line | CommentType::DocLine)
    }

    pub fn is_doc_block(&self) -> bool {
        matches!(self.ty, CommentType::DocBlock)
    }

    pub fn is_prefix(&self) -> bool {
        matches!(self.position, CommentPosition::Prefix)
    }

    pub fn is_before(&self, byte: usize) -> bool {
        self.loc.start() < byte
    }

    /// Returns the contents of the comment without the start and end tokens
    pub fn contents(&self) -> &str {
        let mut s = self.comment.as_str();
        if let Some(stripped) = s.strip_prefix(self.start_token()) {
            s = stripped;
        }
        if let Some(end_token) = self.end_token() {
            if let Some(stripped) = s.strip_suffix(end_token) {
                s = stripped;
            }
        }
        s
    }

    /// The start token of the comment
    #[inline]
    pub const fn start_token(&self) -> &'static str {
        match self.ty {
            CommentType::Line => "//",
            CommentType::Block => "/*",
            CommentType::DocLine => "///",
            CommentType::DocBlock => "/**",
        }
    }

    /// The token that gets written on the newline when the
    /// comment is wrapped
    #[inline]
    pub const fn wrap_token(&self) -> &'static str {
        match self.ty {
            CommentType::Line => "// ",
            CommentType::DocLine => "/// ",
            CommentType::Block => "",
            CommentType::DocBlock => " * ",
        }
    }

    /// The end token of the comment
    #[inline]
    pub const fn end_token(&self) -> Option<&'static str> {
        match self.ty {
            CommentType::Line | CommentType::DocLine => None,
            CommentType::Block | CommentType::DocBlock => Some("*/"),
        }
    }
}

/// A list of comments
#[derive(Clone, Debug, Default)]
pub struct Comments {
    prefixes: VecDeque<CommentWithMetadata>,
    postfixes: VecDeque<CommentWithMetadata>,
}

impl Comments {
    pub fn new(mut comments: Vec<Comment>, src: &str) -> Self {
        let mut prefixes = VecDeque::with_capacity(comments.len());
        let mut postfixes = VecDeque::with_capacity(comments.len());
        let mut last_comment = None;

        comments.sort_by_key(|comment| comment.loc());
        for comment in comments {
            let comment = CommentWithMetadata::from_comment_and_src(comment, src, last_comment);
            let vec = if comment.is_prefix() { &mut prefixes } else { &mut postfixes };
            vec.push_back(comment);
            last_comment = Some(vec.back().unwrap());
        }
        Self { prefixes, postfixes }
    }

    /// Helper for removing comments before a byte offset
    fn remove_comments_before(
        comments: &mut VecDeque<CommentWithMetadata>,
        byte: usize,
    ) -> Vec<CommentWithMetadata> {
        let pos = comments
            .iter()
            .find_position(|comment| !comment.is_before(byte))
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| comments.len());
        if pos == 0 {
            return Vec::new()
        }
        comments.rotate_left(pos);
        comments.split_off(comments.len() - pos).into()
    }

    /// Remove any prefix comments that occur before the byte offset in the src
    pub(crate) fn remove_prefixes_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        Self::remove_comments_before(&mut self.prefixes, byte)
    }

    /// Remove any postfix comments that occur before the byte offset in the src
    pub(crate) fn remove_postfixes_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        Self::remove_comments_before(&mut self.postfixes, byte)
    }

    /// Remove any comments that occur before the byte offset in the src
    pub(crate) fn remove_all_comments_before(&mut self, byte: usize) -> Vec<CommentWithMetadata> {
        self.remove_prefixes_before(byte)
            .into_iter()
            .merge(self.remove_postfixes_before(byte))
            .collect()
    }

    pub(crate) fn pop(&mut self) -> Option<CommentWithMetadata> {
        if self.iter().next()?.is_prefix() {
            self.prefixes.pop_front()
        } else {
            self.postfixes.pop_front()
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CommentWithMetadata> {
        self.prefixes.iter().merge(self.postfixes.iter())
    }

    /// Parse all comments to return a list of inline config items. This will return an iterator of
    /// results of parsing comments which start with `forgefmt:`
    pub fn parse_inline_config_items(
        &self,
    ) -> impl Iterator<Item = Result<(Loc, InlineConfigItem), (Loc, InvalidInlineConfigItem)>> + '_
    {
        self.iter()
            .filter_map(|comment| {
                Some((comment, comment.contents().trim_start().strip_prefix("forgefmt:")?.trim()))
            })
            .map(|(comment, item)| {
                let loc = comment.loc;
                item.parse().map(|out| (loc, out)).map_err(|out| (loc, out))
            })
    }
}

/// The state of a character in a string with possible comments
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentState {
    /// character not in a comment
    #[default]
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

/// An Iterator over characters and indices in a string slice with information about the state of
/// comments
pub struct CommentStateCharIndices<'a> {
    iter: std::str::CharIndices<'a>,
    state: CommentState,
}

impl<'a> CommentStateCharIndices<'a> {
    #[inline]
    fn new(string: &'a str) -> Self {
        Self { iter: string.char_indices(), state: CommentState::None }
    }

    #[inline]
    pub fn with_state(mut self, state: CommentState) -> Self {
        self.state = state;
        self
    }

    #[inline]
    pub fn peek(&mut self) -> Option<(usize, char)> {
        self.iter.clone().next()
    }
}

impl Iterator for CommentStateCharIndices<'_> {
    type Item = (CommentState, usize, char);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (idx, ch) = self.iter.next()?;
        match self.state {
            CommentState::None => {
                if ch == '/' {
                    self.state = match self.peek() {
                        Some((_, '/')) => CommentState::LineStart1,
                        Some((_, '*')) => CommentState::BlockStart1,
                        _ => CommentState::None,
                    };
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
                    if let Some((_, '/')) = self.peek() {
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

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }

    #[inline]
    fn count(self) -> usize {
        self.iter.count()
    }
}

impl std::iter::FusedIterator for CommentStateCharIndices<'_> {}

/// An Iterator over characters in a string slice which are not a apart of comments
pub struct NonCommentChars<'a>(CommentStateCharIndices<'a>);

impl<'a> Iterator for NonCommentChars<'a> {
    type Item = char;

    #[inline]
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
    fn comment_state_char_indices(&self) -> CommentStateCharIndices<'_>;

    #[inline]
    fn non_comment_chars(&self) -> NonCommentChars<'_> {
        NonCommentChars(self.comment_state_char_indices())
    }

    #[inline]
    fn trim_comments(&self) -> String {
        self.non_comment_chars().collect()
    }
}

impl<T> CommentStringExt for T
where
    T: AsRef<str>,
{
    #[inline]
    fn comment_state_char_indices(&self) -> CommentStateCharIndices<'_> {
        CommentStateCharIndices::new(self.as_ref())
    }
}

impl CommentStringExt for str {
    #[inline]
    fn comment_state_char_indices(&self) -> CommentStateCharIndices<'_> {
        CommentStateCharIndices::new(self)
    }
}
