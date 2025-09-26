use super::{Printer, Token};
use std::borrow::Cow;

/// Provides a method `fn push_str_crlf(..)` for appending a string slice while normalizing its line
/// endings to CRLF (`\r\n`).
///
/// Used to ensure that output is compatible with platforms that expect Windows-style line endings.
pub(super) trait StringCrlf {
    fn push_str_crlf(&mut self, string: &str);
}

impl StringCrlf for String {
    fn push_str_crlf(&mut self, string: &str) {
        // Reserve memory in to minimize re-allocations.
        self.reserve(string.len());

        let mut last_was_cr = self.ends_with('\r');
        for ch in string.chars() {
            if ch == '\n' && !last_was_cr {
                self.push('\r');
            }
            self.push(ch);
            last_was_cr = ch == '\r';
        }
    }
}

impl Printer {
    pub fn word_space(&mut self, w: impl Into<Cow<'static, str>>) {
        self.word(w);
        self.space();
    }

    /// Adds a new hardbreak if not at the beginning of the line.
    /// If there was a buffered break token, replaces it (ensures hardbreak) keeping the offset.
    pub fn hardbreak_if_not_bol(&mut self) {
        if !self.is_bol_or_only_ind() {
            if let Some(Token::Break(last)) = self.last_token_still_buffered()
                && last.offset != 0
            {
                self.replace_last_token_still_buffered(Self::hardbreak_tok_offset(last.offset));
                return;
            }
            self.hardbreak();
        }
    }

    pub fn space_if_not_bol(&mut self) {
        if !self.is_bol_or_only_ind() {
            self.space();
        }
    }

    pub fn nbsp(&mut self) {
        self.word(" ");
    }

    pub fn space_or_nbsp(&mut self, breaks: bool) {
        if breaks {
            self.space();
        } else {
            self.nbsp();
        }
    }

    pub fn word_nbsp(&mut self, w: impl Into<Cow<'static, str>>) {
        self.word(w);
        self.nbsp();
    }

    /// Synthesizes a comment that was not textually present in the original
    /// source file.
    pub fn synth_comment(&mut self, text: impl Into<Cow<'static, str>>) {
        self.word("/*");
        self.space();
        self.word(text);
        self.space();
        self.word("*/");
    }
}
