//! Format buffer.

use crate::{
    comments::{CommentState, CommentStringExt},
    string::{QuoteState, QuotedStringExt},
};
use std::fmt::Write;

/// An indent group. The group may optionally skip the first line
#[derive(Clone, Debug, Default)]
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
            Self::LineStart(state) => *state,
            Self::WriteTokens(state) => *state,
            Self::WriteString(_) => CommentState::None,
        }
    }
}

impl Default for WriteState {
    fn default() -> Self {
        Self::LineStart(CommentState::default())
    }
}

/// A wrapper around a `std::fmt::Write` interface. The wrapper keeps track of indentation as well
/// as information about the last `write_str` command if available. The formatter may also be
/// restricted to a single line, in which case it will throw an error on a newline
#[derive(Clone, Debug)]
pub struct FormatBuffer<W> {
    pub w: W,
    indents: Vec<IndentGroup>,
    base_indent_len: usize,
    tab_width: usize,
    last_char: Option<char>,
    current_line_len: usize,
    restrict_to_single_line: bool,
    state: WriteState,
}

impl<W> FormatBuffer<W> {
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

    /// Check if the buffer is at the beginning of a new line
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
        self._write_raw(s.as_ref())
    }

    fn _write_raw(&mut self, s: &str) -> std::fmt::Result {
        let mut lines = s.lines().peekable();
        let mut comment_state = self.state.comment_state();
        while let Some(line) = lines.next() {
            // remove the whitespace that covered by the base indent length (this is normally the
            // case with temporary buffers as this will be re-added by the underlying IndentWriter
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
            if lines.peek().is_some() || s.ends_with('\n') {
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
                            self.current_line_len = 0;
                            self.last_char = Some(' ');
                            // a newline has been inserted
                            if len > 0 {
                                if self.last_indent_group_skipped() {
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
    fn test_buffer_indents() -> std::fmt::Result {
        let delta = 1;

        let mut buf = FormatBuffer::new(String::new(), TAB_WIDTH);
        assert_eq!(buf.indents.len(), 0);
        assert_eq!(buf.level(), 0);
        assert_eq!(buf.current_indent_len(), 0);

        buf.indent(delta);
        assert_eq!(buf.indents.len(), delta);
        assert_eq!(buf.level(), delta);
        assert_eq!(buf.current_indent_len(), delta * TAB_WIDTH);

        buf.indent(delta);
        buf.set_last_indent_group_skipped(true);
        assert!(buf.last_indent_group_skipped());
        assert_eq!(buf.indents.len(), delta * 2);
        assert_eq!(buf.level(), delta);
        assert_eq!(buf.current_indent_len(), delta * TAB_WIDTH);
        buf.dedent(delta);

        buf.dedent(delta);
        assert_eq!(buf.indents.len(), 0);
        assert_eq!(buf.level(), 0);
        assert_eq!(buf.current_indent_len(), 0);

        // panics on extra dedent
        let res = std::panic::catch_unwind(|| buf.clone().dedent(delta));
        assert!(res.is_err());

        Ok(())
    }

    #[test]
    fn test_identical_temp_buf() -> std::fmt::Result {
        let content = "test string";
        let multiline_content = "test\nmultiline\nmultiple";
        let mut buf = FormatBuffer::new(String::new(), TAB_WIDTH);

        // create identical temp buf
        let mut temp = buf.create_temp_buf();
        writeln!(buf, "{content}")?;
        writeln!(temp, "{content}")?;
        assert_eq!(buf.w, format!("{content}\n"));
        assert_eq!(temp.w, buf.w);
        assert_eq!(temp.current_line_len, buf.current_line_len);
        assert_eq!(temp.base_indent_len, buf.total_indent_len());

        let delta = 1;
        buf.indent(delta);

        let mut temp_indented = buf.create_temp_buf();
        assert!(temp_indented.w.is_empty());
        assert_eq!(temp_indented.base_indent_len, buf.total_indent_len());
        assert_eq!(temp_indented.level() + delta, buf.level());

        let indent = " ".repeat(delta * TAB_WIDTH);

        let mut original_buf = buf.clone();
        write!(buf, "{multiline_content}")?;
        let expected_content = format!(
            "{}\n{}{}",
            content,
            indent,
            multiline_content.lines().collect::<Vec<_>>().join(&format!("\n{indent}"))
        );
        assert_eq!(buf.w, expected_content);

        write!(temp_indented, "{multiline_content}")?;

        // write temp buf to original and assert the result
        write!(original_buf, "{}", temp_indented.w)?;
        assert_eq!(buf.w, original_buf.w);

        Ok(())
    }

    #[test]
    fn test_preserves_original_content_with_default_settings() -> std::fmt::Result {
        let contents = [
            "simple line",
            r"
            some 
                    multiline
    content",
            "// comment",
            "/* comment */",
            r"mutliline
            content
            // comment1
            with comments
            /* comment2 */ ",
        ];

        for content in contents.iter() {
            let mut buf = FormatBuffer::new(String::new(), TAB_WIDTH);
            write!(buf, "{content}")?;
            assert_eq!(&buf.w, content);
        }

        Ok(())
    }
}
