use super::{Printer, Token};
use std::borrow::Cow;

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
