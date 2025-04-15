use super::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    pp::{self, Breaks, Token},
};
use crate::{iter::IterDelimited, FormatterConfig, InlineConfig};
use foundry_config::fmt as config;
use itertools::Itertools;
use solar_parse::{
    ast::{self, token, Span},
    interface::{BytePos, SourceMap},
};
use std::borrow::Cow;

// TODO(dani): config
const INDENT: isize = 4;

pub(super) struct State<'a> {
    pub(crate) s: pp::Printer,
    sm: &'a SourceMap,
    comments: Comments,
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

/// Generic methods.
impl<'a> State<'a> {
    pub(super) fn new(
        sm: &'a SourceMap,
        config: FormatterConfig,
        inline_config: InlineConfig,
        comments: Comments,
    ) -> Self {
        Self { s: pp::Printer::new(), sm, comments, inline_config, config }
    }

    fn comments(&self) -> &Comments {
        &self.comments
    }

    fn comments_mut(&mut self) -> &mut Comments {
        &mut self.comments
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

    fn peek_comment<'b>(&'b self) -> Option<&'b Comment>
    where
        'a: 'b,
    {
        self.comments().peek()
    }

    fn next_comment(&mut self) -> Option<Comment> {
        self.comments_mut().next()
    }

    fn maybe_print_trailing_comment(&mut self, span: Span, next_pos: Option<BytePos>) {
        if let Some(cmnt) = self.comments.trailing_comment(self.sm, span, next_pos) {
            self.print_comment(cmnt);
        }
    }

    fn print_remaining_comments(&mut self) {
        // If there aren't any remaining comments, then we need to manually
        // make sure there is a line break at the end.
        if self.peek_comment().is_none() {
            self.hardbreak();
        }
        while let Some(cmnt) = self.next_comment() {
            self.print_comment(cmnt);
        }
    }

    fn bopen(&mut self) {
        self.word("{");
        self.end(); // Close the head-box.
    }

    fn bclose_maybe_open(&mut self, span: Span, empty: bool, close_box: bool) {
        let has_comment = self.maybe_print_comment(span.hi());
        if !empty || has_comment {
            self.break_offset_if_not_bol(1, -INDENT);
        }
        self.word("}");
        if close_box {
            self.end(); // Close the outer-box.
        }
    }

    fn bclose(&mut self, span: Span, empty: bool) {
        let close_box = true;
        self.bclose_maybe_open(span, empty, close_box)
    }

    fn break_offset_if_not_bol(&mut self, n: usize, off: isize) {
        if !self.is_beginning_of_line() {
            self.break_offset(n, off)
        } else if off != 0 {
            if let Some(last_token) = self.last_token_still_buffered() {
                if last_token.is_hardbreak_tok() {
                    // We do something pretty sketchy here: tuck the nonzero
                    // offset-adjustment we were going to deposit along with the
                    // break into the previous hardbreak.
                    self.replace_last_token_still_buffered(pp::Printer::hardbreak_tok_offset(off));
                }
            }
        }
    }
}

/// Span to source.
impl State<'_> {
    fn char_at(&self, pos: BytePos) -> char {
        let res = self.sm.lookup_byte_offset(pos);
        res.sf.src[res.pos.to_usize()..].chars().next().unwrap()
    }
}

/// Language-specific pretty printing.
impl State<'_> {
    pub fn print_source_unit(&mut self, source_unit: &ast::SourceUnit<'_>) {
        for item in source_unit.items.iter() {
            self.print_item(item);
        }
        self.print_remaining_comments();
    }

    fn print_item(&mut self, item: &ast::Item<'_>) {
        let ast::Item { docs, span, kind } = item;
        self.hardbreak_if_not_bol();
        self.print_docs(docs);
        self.maybe_print_comment(span.lo());
        match kind {
            ast::ItemKind::Pragma(ast::PragmaDirective { tokens }) => {
                self.word("pragma ");
                match tokens {
                    ast::PragmaTokens::Version(ident, semver_req) => {
                        self.print_ident(*ident);
                        self.nbsp();
                        self.word(semver_req.to_string());
                    }
                    ast::PragmaTokens::Custom(a, b) => {
                        self.print_ident_or_strlit(a);
                        if let Some(b) = b {
                            self.nbsp();
                            self.print_ident_or_strlit(b);
                        }
                    }
                    ast::PragmaTokens::Verbatim(tokens) => {
                        self.print_tokens(tokens);
                    }
                }
                self.word(";");
                self.hardbreak();
            }
            ast::ItemKind::Import(ast::ImportDirective { path, items }) => {
                self.word("import ");
                match items {
                    ast::ImportItems::Plain(ident) => {
                        self.print_ast_str_lit(path);
                        if let Some(ident) = ident {
                            self.word(" as ");
                            self.print_ident(*ident);
                        }
                    }
                    ast::ImportItems::Aliases(aliases) => {
                        self.cbox(INDENT);
                        self.word("{");
                        self.zerobreak(); // TODO(dani): braces space
                        for (pos, (ident, alias)) in aliases.iter().delimited() {
                            self.print_ident(*ident);
                            if let Some(alias) = alias {
                                self.word(" as ");
                                self.print_ident(*alias);
                            }
                            if !pos.is_last {
                                self.word(",");
                                self.space();
                            }
                        }
                        self.zerobreak(); // TODO(dani): braces space
                        self.offset(-INDENT);
                        self.word("}");
                        self.end();
                        self.word(" from ");
                        self.print_ast_str_lit(path);
                    }
                    ast::ImportItems::Glob(ident) => {
                        self.word("* as ");
                        self.print_ident(*ident);
                        self.word(" from ");
                        self.print_ast_str_lit(path);
                    }
                }
                self.word(";");
                self.hardbreak();
            }
            ast::ItemKind::Using(_using) => todo!("Using"),
            ast::ItemKind::Contract(_contract) => todo!("Contract"),
            ast::ItemKind::Function(_func) => todo!("Function"),
            ast::ItemKind::Variable(_var) => todo!("Variable"),
            ast::ItemKind::Struct(_strukt) => todo!("Struct"),
            ast::ItemKind::Enum(_enumm) => todo!("Enum"),
            ast::ItemKind::Udvt(_udvt) => todo!("Udvt"),
            ast::ItemKind::Error(_error) => todo!("Error"),
            ast::ItemKind::Event(_event) => todo!("Event"),
        }
    }

    fn print_docs(&mut self, docs: &ast::DocComments<'_>) {
        for &ast::DocComment { kind, span, symbol } in docs.iter() {
            self.maybe_print_comment(span.lo());
            self.word(match kind {
                ast::CommentKind::Line => {
                    format!("///{symbol}")
                }
                ast::CommentKind::Block => {
                    format!("/**{symbol}*/")
                }
            });
            self.hardbreak();
        }
    }

    fn print_ident_or_strlit(&mut self, value: &ast::IdentOrStrLit) {
        match value {
            ast::IdentOrStrLit::Ident(ident) => self.print_ident(*ident),
            ast::IdentOrStrLit::StrLit(strlit) => self.print_ast_str_lit(strlit),
        }
    }

    fn print_tokens(&mut self, tokens: &[token::Token]) {
        let s = tokens.iter().map(|t| self.token_to_string(t)).join(" ");
        self.word(s);
    }

    #[allow(clippy::single_match)]
    fn token_to_string<'a>(&self, token: &'a token::Token) -> Cow<'a, str> {
        match token.kind {
            token::TokenKind::Literal(kind, sym) => match kind {
                token::TokenLitKind::Str |
                token::TokenLitKind::UnicodeStr |
                token::TokenLitKind::HexStr => {
                    let kind = match kind {
                        token::TokenLitKind::Str => ast::StrKind::Str,
                        token::TokenLitKind::UnicodeStr => ast::StrKind::Unicode,
                        token::TokenLitKind::HexStr => ast::StrKind::Hex,
                        _ => unreachable!(),
                    };
                    return Cow::Owned(self.str_lit_to_string(token.span, sym.as_str(), kind));
                }
                token::TokenLitKind::Integer |
                token::TokenLitKind::Rational |
                token::TokenLitKind::Err(_) => {}
            },
            _ => {}
        }
        Cow::Borrowed(token.as_str())
    }

    fn print_ident(&mut self, ident: ast::Ident) {
        // TODO(dani): is this right?
        self.maybe_print_comment(ident.span.lo());

        self.word(ident.to_string());
    }

    /// Prints a raw AST string literal, which is unescaped.
    fn print_ast_str_lit(&mut self, strlit: &ast::StrLit) {
        self.print_str_lit_unescaped(strlit.span, strlit.value.as_str(), ast::StrKind::Str);
    }

    fn print_str_lit(&mut self, span: Span, s: &str, kind: ast::StrKind) {
        let s = self.str_lit_to_string(span, s, kind);
        self.word(s);
    }
    fn print_str_lit_unescaped(&mut self, span: Span, s: &str, kind: ast::StrKind) {
        let s = self.str_lit_to_string_unescaped(span, s, kind);
        self.word(s);
    }

    fn str_lit_to_string(&self, span: Span, s: &str, kind: ast::StrKind) -> String {
        self.str_lit_to_string_inner(span, s.escape_debug(), kind)
    }
    fn str_lit_to_string_unescaped(&self, span: Span, s: &str, kind: ast::StrKind) -> String {
        self.str_lit_to_string_inner(span, s, kind)
    }
    fn str_lit_to_string_inner(
        &self,
        span: Span,
        s: impl std::fmt::Display,
        kind: ast::StrKind,
    ) -> String {
        let prefix = match kind {
            ast::StrKind::Str => "",
            ast::StrKind::Unicode => "unicode",
            ast::StrKind::Hex => "hex",
        };
        let quote = match self.config.quote_style {
            config::QuoteStyle::Double => '\"',
            config::QuoteStyle::Single => '\'',
            config::QuoteStyle::Preserve => self.char_at(span.lo() + prefix.len() as u32),
        };
        format!("{prefix}{quote}{s}{quote}")
    }
}
