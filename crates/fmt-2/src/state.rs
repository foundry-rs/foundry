use super::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    pp::{self, Breaks, Token},
};
use crate::{iter::IterDelimited, FormatterConfig, InlineConfig};
use foundry_config::fmt as config;
use itertools::{Either, Itertools};
use solar_parse::{
    ast::{self, token, Span},
    interface::{BytePos, SourceMap},
};
use std::borrow::Cow;

// TODO(dani): config
const INDENT: isize = 4;

/*
- [ ]
/// Maximum line length where formatter will try to wrap the line
pub line_length: usize,

- [ ]
/// Number of spaces per indentation level
pub tab_width: usize,

- [x]
/// Print spaces between brackets
pub bracket_spacing: bool,

- [x]
/// Style of uint/int256 types
pub int_types: IntTypes,

- [ ]
/// Style of multiline function header in case it doesn't fit
pub multiline_func_header: MultilineFuncHeaderStyle,

- [x]
/// Style of quotation marks
pub quote_style: QuoteStyle,

- [x]
/// Style of underscores in number literals
pub number_underscore: NumberUnderscore,

- [x]
/// Style of underscores in hex literals
pub hex_underscore: HexUnderscore,

- [ ]
/// Style of single line blocks in statements
pub single_line_statement_blocks: SingleLineBlockStyle,

- [ ]
/// Print space in state variable, function and modifier `override` attribute
pub override_spacing: bool,

- [ ]
/// Wrap comments on `line_length` reached
pub wrap_comments: bool,

- [N/A]
/// Globs to ignore
pub ignore: Vec<String>,

- [ ]
/// Add new line at start and end of contract declarations
pub contract_new_lines: bool,

- [ ]
/// Sort import statements alphabetically in groups (a group is separated by a newline).
pub sort_imports: bool,
*/

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

    // TODO(dani): remove bool?
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

    fn print_comment(&mut self, mut cmnt: Comment) {
        match cmnt.style {
            CommentStyle::Mixed => {
                if !self.is_beginning_of_line() {
                    self.zerobreak();
                }
                if let Some(last) = cmnt.lines.pop() {
                    self.ibox(0);

                    for line in cmnt.lines {
                        self.word(line);
                        self.hardbreak()
                    }

                    self.word(last);
                    self.space();

                    self.end();
                }
                self.zerobreak()
            }
            CommentStyle::Isolated => {
                self.hardbreak_if_not_bol();
                for line in cmnt.lines {
                    // Don't print empty lines because they will end up as trailing
                    // whitespace.
                    if !line.is_empty() {
                        self.word(line);
                    }
                    self.hardbreak();
                }
            }
            CommentStyle::Trailing => {
                if !self.is_beginning_of_line() {
                    self.word(" ");
                }
                if cmnt.lines.len() == 1 {
                    self.word(cmnt.lines.pop().unwrap());
                    self.hardbreak()
                } else {
                    self.visual_align();
                    for line in cmnt.lines {
                        if !line.is_empty() {
                            self.word(line);
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

    fn braces_break(&mut self) {
        if self.config.bracket_spacing {
            self.space();
        } else {
            self.zerobreak();
        }
    }
}

/// Span to source.
impl State<'_> {
    fn char_at(&self, pos: BytePos) -> char {
        let res = self.sm.lookup_byte_offset(pos);
        res.sf.src[res.pos.to_usize()..].chars().next().unwrap()
    }

    /// Returns `true` if the span is disabled and has been printed as-is.
    #[must_use]
    fn handle_span(&mut self, span: Span) -> bool {
        self.maybe_print_comment(span.lo());
        self.print_span_if_disabled(span)
    }

    /// Returns `true` if the span is disabled and has been printed as-is.
    #[inline]
    #[must_use]
    fn print_span_if_disabled(&mut self, span: Span) -> bool {
        let disabled = self.inline_config.is_disabled(span);
        if disabled {
            self.print_span_cold(span);
        }
        disabled
    }

    #[cold]
    fn print_span_cold(&mut self, span: Span) {
        self.print_span(span);
    }

    fn print_span(&mut self, span: Span) {
        match self.sm.span_to_snippet(span) {
            Ok(s) => self.word(s),
            Err(e) => panic!("failed to print {span:?}: {e:#?}"),
        }
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
                        self.braces_break();
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
                        self.braces_break();
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
            ast::ItemKind::Using(ast::UsingDirective { list, ty, global }) => {
                self.word("using ");
                match list {
                    ast::UsingList::Single(path) => self.print_path(path),
                    ast::UsingList::Multiple(items) => {
                        self.cbox(INDENT);
                        self.word("{");
                        self.braces_break();
                        for (pos, (path, op)) in items.iter().delimited() {
                            self.print_path(path);
                            if let Some(op) = op {
                                self.word(" as ");
                                // TODO(dani): op.to_str()
                                let _ = op;
                                self.word("?");
                            }
                            if !pos.is_last {
                                self.word(",");
                                self.space();
                            }
                        }
                        self.braces_break();
                        self.offset(-INDENT);
                        self.word("}");
                        self.end();
                    }
                }
                self.word(" for ");
                if let Some(ty) = ty {
                    self.print_ty(ty);
                } else {
                    self.word("*");
                }
                if *global {
                    self.word(" global");
                }
                self.word(";");
                self.hardbreak();
            }
            ast::ItemKind::Contract(_contract) => todo!(),
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
        // Leave unchanged.
        let span = Span::join_first_last(tokens.iter().map(|t| t.span));
        self.print_span(span);
    }

    fn print_ident(&mut self, ident: ast::Ident) {
        self.maybe_print_comment(ident.span.lo());
        self.word(ident.to_string());
    }

    fn print_path(&mut self, path: &ast::PathSlice) {
        for (pos, ident) in path.segments().iter().delimited() {
            self.print_ident(*ident);
            if !pos.is_last {
                self.word(".");
            }
        }
    }

    fn print_lit(&mut self, lit: &ast::Lit) {
        let &ast::Lit { span, symbol, ref kind } = lit;
        if self.handle_span(span) {
            return;
        }
        let s = symbol.as_str();

        match *kind {
            ast::LitKind::Str(kind, _) => {
                let prefix_len = match kind {
                    ast::StrKind::Str => 0,
                    ast::StrKind::Unicode => 7,
                    ast::StrKind::Hex => 3,
                };
                let quote_pos = span.lo() + prefix_len as u32;
                let s = &s[prefix_len + 1..s.len() - 1];
                self.print_str_lit(kind, quote_pos, s);
                return;
            }
            ast::LitKind::Number(_) | ast::LitKind::Rational(_)
                if !self.config.number_underscore.is_preserve() =>
            {
                self.print_num_literal(s);
                return;
            }
            _ => {}
        };
        self.word(s.to_string());
    }

    fn print_num_literal(&mut self, source: &str) {
        fn strip_underscores(s: &str) -> Cow<'_, str> {
            if s.contains('_') {
                Cow::Owned(s.replace('_', ""))
            } else {
                Cow::Borrowed(s)
            }
        }

        fn add_underscores(
            out: &mut String,
            config: config::NumberUnderscore,
            string: &str,
            reversed: bool,
        ) {
            if !config.is_thousands() || string.len() < 5 {
                out.push_str(string);
                return;
            }

            let chunks = if reversed {
                Either::Left(string.as_bytes().chunks(3))
            } else {
                Either::Right(string.as_bytes().rchunks(3).rev())
            }
            .map(|chunk| std::str::from_utf8(chunk).unwrap());
            for chunk in Itertools::intersperse(chunks, "_") {
                out.push_str(chunk);
            }
        }

        debug_assert!(source.is_ascii(), "{source:?}");

        let config = self.config.number_underscore;
        debug_assert!(!config.is_preserve());

        let (val, exp) = source.split_once(['e', 'E']).unwrap_or((source, ""));
        let (val, fract) = val.split_once('.').unwrap_or((val, ""));

        let val = strip_underscores(val);
        let exp = strip_underscores(exp);
        let fract = strip_underscores(fract);

        // strip any padded 0's
        let val = val.trim_start_matches('0');
        let fract = fract.trim_end_matches('0');
        let (exp_sign, exp) =
            if let Some(exp) = exp.strip_prefix('-') { ("-", exp) } else { ("", &exp[..]) };
        let exp = exp.trim_start_matches('0');

        let mut out = String::with_capacity(source.len() * 2);
        if val.is_empty() {
            out.push('0');
        } else {
            add_underscores(&mut out, config, val, false);
        }
        if !fract.is_empty() {
            out.push('.');
            add_underscores(&mut out, config, fract, true);
        }
        if !exp.is_empty() {
            // TODO: preserve the `E`?
            /*
            out.push(if source.contains('e') {
                'e'
            } else {
                debug_assert!(source.contains('E'));
                'E'
            });
            */
            out.push('e');
            out.push_str(exp_sign);
            add_underscores(&mut out, config, exp, false);
        }

        self.word(out);
    }

    /// Prints a raw AST string literal, which is unescaped.
    fn print_ast_str_lit(&mut self, strlit: &ast::StrLit) {
        self.print_str_lit(ast::StrKind::Str, strlit.span.lo(), strlit.value.as_str());
    }

    /// `s` should be the *unescaped contents of the string literal*.
    fn print_str_lit(&mut self, kind: ast::StrKind, quote_pos: BytePos, s: &str) {
        let s = self.str_lit_to_string(kind, quote_pos, s);
        self.word(s);
    }

    /// `s` should be the *unescaped contents of the string literal*.
    fn str_lit_to_string(&self, kind: ast::StrKind, quote_pos: BytePos, s: &str) -> String {
        let prefix = match kind {
            ast::StrKind::Str => "",
            ast::StrKind::Unicode => "unicode",
            ast::StrKind::Hex => "hex",
        };
        let quote = match self.config.quote_style {
            config::QuoteStyle::Double => '\"',
            config::QuoteStyle::Single => '\'',
            config::QuoteStyle::Preserve => self.char_at(quote_pos),
        };
        let s = solar_parse::interface::data_structures::fmt::from_fn(move |f| {
            if matches!(kind, ast::StrKind::Hex) {
                match self.config.hex_underscore {
                    config::HexUnderscore::Preserve => {}
                    config::HexUnderscore::Remove | config::HexUnderscore::Bytes => {
                        let mut clean = s.to_string().replace('_', "");
                        if matches!(self.config.hex_underscore, config::HexUnderscore::Bytes) {
                            clean =
                                clean.chars().chunks(2).into_iter().map(|c| c.format("")).join("_");
                        }
                        return f.write_str(&clean);
                    }
                };
            }
            f.write_str(s)
        });
        format!("{prefix}{quote}{s}{quote}")
    }

    fn print_ty(&mut self, ty: &ast::Type<'_>) {
        if self.handle_span(ty.span) {
            return;
        }

        match &ty.kind {
            &ast::TypeKind::Elementary(ty) => 'b: {
                match ty {
                    // `address payable` is normalized to `address`.
                    ast::ElementaryType::Address(true) => self.word("address payable"),
                    // Integers are normalized to long form.
                    ast::ElementaryType::Int(size) | ast::ElementaryType::UInt(size) => {
                        match (self.config.int_types, size.bits_raw()) {
                            (config::IntTypes::Short, 0 | 256) |
                            (config::IntTypes::Preserve, 0) => {
                                let short = match ty {
                                    ast::ElementaryType::Int(_) => "int",
                                    ast::ElementaryType::UInt(_) => "uint",
                                    _ => unreachable!(),
                                };
                                self.word(short);
                                break 'b;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                self.word(ty.to_abi_str());
            }
            ast::TypeKind::Array(ast::TypeArray { element, size }) => {
                self.print_ty(element);
                if let Some(size) = size {
                    self.word("[");
                    self.print_expr(size);
                    self.word("]");
                } else {
                    self.word("[]");
                }
            }
            ast::TypeKind::Function(_func) => todo!(),
            ast::TypeKind::Mapping(ast::TypeMapping { key, key_name, value, value_name }) => {
                self.word("mapping(");
                self.print_ty(key);
                if let Some(ident) = key_name {
                    self.word(" ");
                    self.print_ident(*ident);
                }
                self.word(" => ");
                self.print_ty(value);
                if let Some(ident) = value_name {
                    self.word(" ");
                    self.print_ident(*ident);
                }
            }
            ast::TypeKind::Custom(path) => self.print_path(path),
        }
    }

    fn print_expr(&mut self, expr: &ast::Expr<'_>) {
        // TODO
        let _ = expr;
        self.word("<expr>");
    }
}
