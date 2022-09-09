//! Format buffer

use std::fmt::Write;

use crate::{
    comments::{CommentState, CommentStringExt},
    string::{QuoteState, QuotedStringExt},
};

/// An indent group. The group may optionally skip the first line
#[derive(Default, Clone, Debug)]
struct IndentGroup {
    skip_line: bool,
}

#[derive(Clone, Copy, Debug)]
enum WriteState {
    LineStart(CommentState),
    WriteTokens(CommentState),
    WriteString(char),
}

impl WriteState {
    fn comment_state(&self) -> CommentState {
        match self {
            WriteState::LineStart(state) => *state,
            WriteState::WriteTokens(state) => *state,
            WriteState::WriteString(_) => CommentState::None,
        }
    }
}

impl Default for WriteState {
    fn default() -> Self {
        WriteState::LineStart(CommentState::default())
    }
}

/// A wrapper around a `std::fmt::Write` interface. The wrapper keeps track of indentation as well
/// as information about the last `write_str` command if available. The formatter may also be
/// restricted to a single line, in which case it will throw an error on a newline
#[derive(Clone, Debug)]
pub struct FormatBuffer<W: Sized> {
    pub w: W,
    indents: Vec<IndentGroup>,
    base_indent_len: usize,
    tab_width: usize,
    last_char: Option<char>,
    current_line_len: usize,
    restrict_to_single_line: bool,
    state: WriteState,
}

impl<W: Sized> FormatBuffer<W> {
    pub fn new(w: W, tab_width: usize) -> Self {
        Self {
            w,
            tab_width,
            base_indent_len: 0,
            indents: vec![],
            current_line_len: 0,
            last_char: None,
            restrict_to_single_line: false,
            state: WriteState::default(),
        }
    }

    /// Create a new temporary buffer based on an existing buffer which retains information about
    /// the buffer state, but has a blank String as its underlying `Write` interface
    pub fn create_temp_buf(&self) -> FormatBuffer<String> {
        let mut new = FormatBuffer::new(String::new(), self.tab_width);
        new.base_indent_len = self.total_indent_len();
        new.current_line_len = self.current_line_len();
        new.last_char = self.last_char;
        new.restrict_to_single_line = self.restrict_to_single_line;
        new.state = match self.state {
            WriteState::WriteTokens(state) | WriteState::LineStart(state) => {
                WriteState::LineStart(state)
            }
            WriteState::WriteString(ch) => WriteState::WriteString(ch),
        };
        new
    }

    /// Restrict the buffer to a single line
    pub fn restrict_to_single_line(&mut self, restricted: bool) {
        self.restrict_to_single_line = restricted;
    }

    /// Indent the buffer by delta
    pub fn indent(&mut self, delta: usize) {
        self.indents.extend(std::iter::repeat(IndentGroup::default()).take(delta));
    }

    /// Dedent the buffer by delta
    pub fn dedent(&mut self, delta: usize) {
        self.indents.truncate(self.indents.len() - delta);
    }

    /// Get the current level of the indent. This is multiplied by the tab width to get the
    /// resulting indent
    fn level(&self) -> usize {
        self.indents.iter().filter(|i| !i.skip_line).count()
    }

    /// Check if the last indent group is being skipped
    pub fn last_indent_group_skipped(&self) -> bool {
        self.indents.last().map(|i| i.skip_line).unwrap_or(false)
    }

    /// Set whether the last indent group should be skipped
    pub fn set_last_indent_group_skipped(&mut self, skip_line: bool) {
        if let Some(i) = self.indents.last_mut() {
            i.skip_line = skip_line
        }
    }

    /// Get the current indent size (level * tab_width)
    pub fn current_indent_len(&self) -> usize {
        self.level() * self.tab_width
    }

    /// Get the total indent size
    pub fn total_indent_len(&self) -> usize {
        self.current_indent_len() + self.base_indent_len
    }

    /// Get the current written position (this does not include the indent size)
    pub fn current_line_len(&self) -> usize {
        self.current_line_len
    }

    /// Set the current position
    pub fn set_current_line_len(&mut self, len: usize) {
        self.current_line_len = len
    }

    /// Check if the buffer is at the beggining of a new line
    pub fn is_beginning_of_line(&self) -> bool {
        matches!(self.state, WriteState::LineStart(_))
    }

    /// Start a new indent group (skips first indent)
    pub fn start_group(&mut self) {
        self.indents.push(IndentGroup { skip_line: true });
    }

    /// End the last indent group
    pub fn end_group(&mut self) {
        self.indents.pop();
    }

    /// Get the last char written to the buffer
    pub fn last_char(&self) -> Option<char> {
        self.last_char
    }

    /// When writing a newline apply state changes
    fn handle_newline(&mut self, mut comment_state: CommentState) {
        if comment_state == CommentState::Line {
            comment_state = CommentState::None;
        }
        self.current_line_len = 0;
        self.set_last_indent_group_skipped(false);
        self.last_char = Some('\n');
        self.state = WriteState::LineStart(comment_state);
    }
}

impl<W: Write> FormatBuffer<W> {
    /// Write a raw string to the buffer. This will ignore indents and remove the indents of the
    /// written string to match the current base indent of this buffer if it is a temp buffer
    pub fn write_raw(&mut self, s: impl AsRef<str>) -> std::fmt::Result {
        let mut lines = s.as_ref().lines().peekable();
        let mut comment_state = self.state.comment_state();
        while let Some(line) = lines.next() {
            // remove the whitespace that covered by the base indent length (this is normally the
            // case with temporary buffers as this will be readded by the underlying IndentWriter
            // later on
            let (new_comment_state, line_start) = line
                .comment_state_char_indices()
                .with_state(comment_state)
                .take(self.base_indent_len)
                .take_while(|(_, _, ch)| ch.is_whitespace())
                .last()
                .map(|(state, idx, _)| (state, idx + 1))
                .unwrap_or((comment_state, 0));
            comment_state = new_comment_state;
            let trimmed_line = &line[line_start..];
            if !trimmed_line.is_empty() {
                self.w.write_str(trimmed_line)?;
                self.current_line_len += trimmed_line.len();
                self.last_char = trimmed_line.chars().next_back();
                self.state = WriteState::WriteTokens(comment_state);
            }
            if lines.peek().is_some() || s.as_ref().ends_with('\n') {
                if self.restrict_to_single_line {
                    return Err(std::fmt::Error)
                }
                self.w.write_char('\n')?;
                self.handle_newline(comment_state);
            }
        }
        Ok(())
    }
}

impl<W: Write> Write for FormatBuffer<W> {
    fn write_str(&mut self, mut s: &str) -> std::fmt::Result {
        if s.is_empty() {
            return Ok(())
        }

        let mut indent = " ".repeat(self.current_indent_len());

        loop {
            match self.state {
                WriteState::LineStart(mut comment_state) => {
                    match s.find(|b| b != '\n') {
                        // No non-empty lines in input, write the entire string (only newlines)
                        None => {
                            if !s.is_empty() {
                                self.w.write_str(s)?;
                                self.handle_newline(comment_state);
                            }
                            break
                        }

                        // We can see the next non-empty line. Write up to the
                        // beginning of that line, then insert an indent, then
                        // continue.
                        Some(len) => {
                            let (head, tail) = s.split_at(len);
                            self.w.write_str(head)?;
                            self.w.write_str(&indent)?;
                            // self.last_indent = indent.clone();
                            // self.base_indent_len = indent.len();
                            self.current_line_len = 0;
                            self.last_char = Some(' ');
                            // a newline has been inserted
                            if len > 0 {
                                if self.last_indent_group_skipped() &&
                                    comment_state != CommentState::Block &&
                                    comment_state != CommentState::BlockStart1 &&
                                    comment_state != CommentState::BlockStart2
                                {
                                    indent = " ".repeat(self.current_indent_len() + self.tab_width);
                                    self.set_last_indent_group_skipped(false);
                                }
                                if comment_state == CommentState::Line {
                                    comment_state = CommentState::None;
                                }
                            }
                            s = tail;
                            self.state = WriteState::WriteTokens(comment_state);
                        }
                    }
                }
                WriteState::WriteTokens(comment_state) => {
                    if s.is_empty() {
                        break
                    }

                    // find the next newline or non-comment string separator (e.g. ' or ")
                    let mut len = 0;
                    let mut new_state = WriteState::WriteTokens(comment_state);
                    for (state, idx, ch) in s.comment_state_char_indices().with_state(comment_state)
                    {
                        len = idx;
                        if ch == '\n' {
                            if self.restrict_to_single_line {
                                return Err(std::fmt::Error)
                            }
                            new_state = WriteState::LineStart(state);
                            break
                        } else if state == CommentState::None && (ch == '\'' || ch == '"') {
                            new_state = WriteState::WriteString(ch);
                            break
                        } else {
                            new_state = WriteState::WriteTokens(state);
                        }
                    }

                    if matches!(new_state, WriteState::WriteTokens(_)) {
                        // No newlines or strings found, write the entire string
                        self.w.write_str(s)?;
                        self.current_line_len += s.len();
                        self.last_char = s.chars().next_back();
                        self.state = new_state;
                        break
                    } else {
                        // A newline or string has been found. Write up to that character and
                        // continue on the tail
                        let (head, tail) = s.split_at(len + 1);
                        self.w.write_str(head)?;
                        s = tail;
                        match new_state {
                            WriteState::LineStart(comment_state) => {
                                self.handle_newline(comment_state)
                            }
                            new_state => {
                                self.current_line_len += head.len();
                                self.last_char = head.chars().next_back();
                                self.state = new_state;
                            }
                        }
                    }
                }
                WriteState::WriteString(quote) => {
                    match s.quoted_ranges().with_state(QuoteState::String(quote)).next() {
                        // No end found, write the rest of the string
                        None => {
                            self.w.write_str(s)?;
                            self.current_line_len += s.len();
                            self.last_char = s.chars().next_back();
                            break
                        }
                        // String end found, write the string and continue to add tokens after
                        Some((_, _, len)) => {
                            let (head, tail) = s.split_at(len + 1);
                            self.w.write_str(head)?;
                            if let Some((_, last)) = head.rsplit_once('\n') {
                                self.set_last_indent_group_skipped(false);
                                // TODO: self.last_indent = String::new();
                                self.current_line_len = last.len();
                            } else {
                                self.current_line_len += head.len();
                            }
                            self.last_char = Some(quote);
                            s = tail;
                            self.state = WriteState::WriteTokens(CommentState::None);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB_WIDTH: usize = 4;

    #[test]
    fn test_identical_temp_buf() -> std::fmt::Result {
        let w = String::new();
        let mut buf = FormatBuffer::new(w, TAB_WIDTH);
        let mut temp = buf.create_temp_buf();

        let content = "test string";
        write!(buf, "{content}")?;
        write!(temp, "{content}")?;
        assert_eq!(temp.base_indent_len, buf.total_indent_len());
        assert_eq!(temp.w, buf.w);

        let delta = 1;
        buf.indent(delta);
        let mut temp_indented = buf.create_temp_buf();
        write!(temp_indented, "{content}")?;
        assert_eq!(temp_indented.base_indent_len, buf.total_indent_len());
        assert_eq!(temp_indented.level() + delta, buf.level());
        assert_eq!(temp_indented.w, buf.w);

        assert_eq!(1, 2);
        Ok(())
    }
}
