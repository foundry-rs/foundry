use super::{BeginToken, BreakToken, Breaks, IndentStyle, Printer, Token, SIZE_INFINITY};
use std::borrow::Cow;

impl Printer {
    /// "raw box"
    pub fn rbox(&mut self, indent: isize, breaks: Breaks) {
        self.scan_begin(BeginToken { indent: IndentStyle::Block { offset: indent }, breaks });
    }

    /// Inconsistent breaking box
    pub fn ibox(&mut self, indent: isize) {
        self.rbox(indent, Breaks::Inconsistent);
    }

    /// Consistent breaking box
    pub fn cbox(&mut self, indent: isize) {
        self.rbox(indent, Breaks::Consistent);
    }

    pub fn visual_align(&mut self) {
        self.scan_begin(BeginToken { indent: IndentStyle::Visual, breaks: Breaks::Consistent });
    }

    pub fn break_offset(&mut self, n: usize, off: isize) {
        self.scan_break(BreakToken { offset: off, blank_space: n, ..BreakToken::default() });
    }

    pub fn end(&mut self) {
        self.scan_end();
    }

    pub fn eof(mut self) -> String {
        self.scan_eof();
        self.out
    }

    pub fn word(&mut self, w: impl Into<Cow<'static, str>>) {
        self.scan_string(w.into());
    }

    fn spaces(&mut self, n: usize) {
        self.break_offset(n, 0);
    }

    pub fn zerobreak(&mut self) {
        self.spaces(0);
    }

    pub fn space(&mut self) {
        self.spaces(1);
    }

    pub fn hardbreak(&mut self) {
        self.spaces(SIZE_INFINITY as usize);
    }

    pub fn last_token_is_hardbreak(&self) -> bool {
        if let Some(token) = self.last_token() {
            return token.is_hardbreak();
        }

        false
    }

    pub fn is_beginning_of_line(&self) -> bool {
        match self.last_token() {
            Some(last_token) => last_token.is_hardbreak(),
            None => self.out.is_empty() || self.out.ends_with('\n'),
        }
    }

    pub fn is_bol_or_only_ind(&self) -> bool {
        for i in self.buf.index_range().rev() {
            let token = &self.buf[i].token;
            if token.is_hardbreak() || matches!(token, Token::Begin(_)) {
                return true;
            }
            if Self::token_has_non_whitespace_content(token) {
                return false;
            }
        }

        let last_line =
            if let Some(pos) = self.out.rfind('\n') { &self.out[pos + 1..] } else { &self.out[..] };

        last_line.trim().is_empty()
    }

    fn token_has_non_whitespace_content(token: &Token) -> bool {
        match token {
            Token::String(s) => !s.trim().is_empty(),
            Token::Break(bt) => {
                if let Some(char) = bt.pre_break {
                    !char.is_whitespace()
                } else {
                    false
                }
            }
            Token::Begin(_) => false,
            Token::End => false,
        }
    }

    pub(crate) fn hardbreak_tok_offset(offset: isize) -> Token {
        Token::Break(BreakToken {
            offset,
            blank_space: SIZE_INFINITY as usize,
            ..BreakToken::default()
        })
    }

    pub fn space_if_nonempty(&mut self) {
        self.scan_break(BreakToken { blank_space: 1, if_nonempty: true, ..BreakToken::default() });
    }

    pub fn hardbreak_if_nonempty(&mut self) {
        self.scan_break(BreakToken {
            blank_space: SIZE_INFINITY as usize,
            if_nonempty: true,
            ..BreakToken::default()
        });
    }

    // Doesn't actually print trailing comma since it's not allowed in Solidity.
    pub fn trailing_comma(&mut self, is_last: bool) {
        if is_last {
            // self.scan_break(BreakToken { pre_break: Some(','), ..BreakToken::default() });
            self.zerobreak();
        } else {
            self.word(",");
            self.space();
        }
    }

    pub fn trailing_comma_or_space(&mut self, is_last: bool) {
        if is_last {
            self.scan_break(BreakToken {
                blank_space: 1,
                pre_break: Some(','),
                ..BreakToken::default()
            });
        } else {
            self.word(",");
            self.space();
        }
    }

    pub fn neverbreak(&mut self) {
        self.scan_break(BreakToken { never_break: true, ..BreakToken::default() });
    }
}

impl Token {
    pub(crate) fn is_hardbreak(&self) -> bool {
        if let Self::Break(BreakToken {
            offset,
            blank_space,
            pre_break: _,
            post_break: _,
            if_nonempty: _,
            never_break,
        }) = *self
        {
            offset == 0 && blank_space == SIZE_INFINITY as usize && !never_break
        } else {
            false
        }
    }
}
