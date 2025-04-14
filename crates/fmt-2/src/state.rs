use super::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    pp::{self, Breaks, Token},
};
use crate::{inline_config::InlineConfigItem, FormatterConfig, FormatterError, InlineConfig};
use solar_parse::interface::{BytePos, Session};
use std::path::Path;

pub(super) struct State<'a> {
    s: pp::Printer,
    comments: Option<Comments<'a>>,
    config: FormatterConfig,
    inline_config: InlineConfig,
}

impl std::ops::Deref for State<'_> {
    type Target = pp::Printer;

    fn deref(&self) -> &Self::Target {
        &self.s
    }
}

impl std::ops::DerefMut for State<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.s
    }
}

impl<'a> State<'a> {
    pub(super) fn new(
        config: FormatterConfig,
        inline_config: InlineConfig,
        comments: Option<Comments<'a>>,
    ) -> Self {
        Self { s: pp::Printer::new(), comments, inline_config, config }
    }

    fn once(config: FormatterConfig) {
        Self::new(config, InlineConfig::default(), None);
    }

    fn comments(&self) -> Option<&Comments<'a>> {
        self.comments.as_ref()
    }

    fn comments_mut(&mut self) -> Option<&mut Comments<'a>> {
        self.comments.as_mut()
    }

    fn peek_comment<'b>(&'b self) -> Option<&'b Comment>
    where
        'a: 'b,
    {
        self.comments().and_then(|c| c.peek())
    }

    fn next_comment(&mut self) -> Option<Comment> {
        self.comments_mut().and_then(|c| c.next())
    }

    fn strsep<'x, T: 'x, F, I>(
        &mut self,
        sep: &'static str,
        space_before: bool,
        b: Breaks,
        elts: I,
        mut op: F,
    ) where
        F: FnMut(&mut Self, &T),
        I: IntoIterator<Item = &'x T>,
    {
        let mut it = elts.into_iter();

        self.rbox(0, b);
        if let Some(first) = it.next() {
            op(self, first);
            for elt in it {
                if space_before {
                    self.space();
                }
                self.word_space(sep);
                op(self, elt);
            }
        }
        self.end();
    }

    fn commasep<'x, T: 'x, F, I>(&mut self, b: Breaks, elts: I, op: F)
    where
        F: FnMut(&mut Self, &T),
        I: IntoIterator<Item = &'x T>,
    {
        self.strsep(",", false, b, elts, op)
    }

    fn maybe_print_comment(&mut self, pos: BytePos) -> bool {
        let mut has_comment = false;
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= pos {
                break;
            }
            has_comment = true;
            let cmnt = self.next_comment().unwrap();
            self.print_comment(cmnt);
        }
        has_comment
    }

    fn print_comment(&mut self, cmnt: Comment) {
        match cmnt.style {
            CommentStyle::Mixed => {
                if !self.is_beginning_of_line() {
                    self.zerobreak();
                }
                if let Some((last, lines)) = cmnt.lines.split_last() {
                    self.ibox(0);

                    for line in lines {
                        self.word(line.clone());
                        self.hardbreak()
                    }

                    self.word(last.clone());
                    self.space();

                    self.end();
                }
                self.zerobreak()
            }
            CommentStyle::Isolated => {
                self.hardbreak_if_not_bol();
                for line in &cmnt.lines {
                    // Don't print empty lines because they will end up as trailing
                    // whitespace.
                    if !line.is_empty() {
                        self.word(line.clone());
                    }
                    self.hardbreak();
                }
            }
            CommentStyle::Trailing => {
                if !self.is_beginning_of_line() {
                    self.word(" ");
                }
                if let [line] = cmnt.lines.as_slice() {
                    self.word(line.clone());
                    self.hardbreak()
                } else {
                    self.visual_align();
                    for line in &cmnt.lines {
                        if !line.is_empty() {
                            self.word(line.clone());
                        }
                        self.hardbreak();
                    }
                    self.end();
                }
            }
            CommentStyle::BlankLine => {
                // We need to do at least one, possibly two hardbreaks.
                let twice = match self.last_token() {
                    Some(Token::String(s)) => ";" == s,
                    Some(Token::Begin(_)) => true,
                    Some(Token::End) => true,
                    _ => false,
                };
                if twice {
                    self.hardbreak();
                }
                self.hardbreak();
            }
        }
    }
}
