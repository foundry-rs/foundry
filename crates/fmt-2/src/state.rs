use super::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    pp::{self, Token},
};
use crate::{iter::IterDelimited, FormatterConfig, InlineConfig};
use foundry_config::fmt as config;
use itertools::{Either, Itertools};
use solar_parse::{
    ast::{self, token, Span},
    interface::{BytePos, SourceMap},
    Cursor,
};
use std::borrow::Cow;

// TODO(dani): bunch docs into `Comments` since misplaced docs get ignore

pub(super) struct State<'sess, 'ast> {
    pub(crate) s: pp::Printer,
    ind: isize,

    sm: &'sess SourceMap,
    comments: Comments,
    config: FormatterConfig,
    inline_config: InlineConfig,

    contract: Option<&'ast ast::ItemContract<'ast>>,
}

impl std::ops::Deref for State<'_, '_> {
    type Target = pp::Printer;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.s
    }
}

impl std::ops::DerefMut for State<'_, '_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.s
    }
}

/// Generic methods.
impl<'sess> State<'sess, '_> {
    pub(super) fn new(
        sm: &'sess SourceMap,
        config: FormatterConfig,
        inline_config: InlineConfig,
        comments: Comments,
    ) -> Self {
        Self {
            s: pp::Printer::new(config.line_length),
            ind: config.tab_width as isize,
            sm,
            comments,
            inline_config,
            config,
            contract: None,
        }
    }

    fn comments(&self) -> &Comments {
        &self.comments
    }

    fn comments_mut(&mut self) -> &mut Comments {
        &mut self.comments
    }

    /// Prints comments that are before the given position.
    ///
    /// Returns `Some` with the style of the last comment printed, or `None` if no comment was
    /// printed.
    fn maybe_print_comments(&mut self, pos: BytePos) -> Option<CommentStyle> {
        let mut has_comment = None;
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= pos {
                break;
            }
            let cmnt = self.next_comment().unwrap();
            has_comment = Some(cmnt.style);
            self.print_comment(cmnt);
        }
        has_comment
    }

    fn print_comment(&mut self, mut cmnt: Comment) {
        match cmnt.style {
            CommentStyle::Mixed => {
                if !self.is_beginning_of_line() {
                    // TODO(dani): ?
                    // self.zerobreak();
                    self.space();
                }
                if let Some(last) = cmnt.lines.pop() {
                    self.ibox(0);

                    for line in cmnt.lines {
                        self.word(line);
                        self.hardbreak();
                    }

                    self.word(last);
                    self.space();

                    self.end();
                }
                self.zerobreak();
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
                    self.nbsp();
                }
                if cmnt.lines.len() == 1 {
                    self.word(cmnt.lines.pop().unwrap());
                    self.hardbreak();
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
        'sess: 'b,
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
        if self.peek_comment().is_none() && !self.is_beginning_of_line() {
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
        let comment = self.maybe_print_comments(span.hi());
        if !empty || comment.is_some() {
            self.break_offset_if_not_bol(1, -self.ind);
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
impl State<'_, '_> {
    fn char_at(&self, pos: BytePos) -> char {
        let res = self.sm.lookup_byte_offset(pos);
        res.sf.src[res.pos.to_usize()..].chars().next().unwrap()
    }

    /// Returns `true` if the span is disabled and has been printed as-is.
    #[must_use]
    fn handle_span(&mut self, span: Span) -> bool {
        self.maybe_print_comments(span.lo());
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
        // Drop comments that are included in the span.
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= span.hi() {
                break;
            }
            let _ = self.next_comment().unwrap();
        }
    }
}

#[derive(Default)]
struct FunctionLike<'a, 'b> {
    kind: &'static str,
    name: Option<ast::Ident>,
    parameters: &'a [ast::VariableDefinition<'b>],
    visibility: Option<ast::Visibility>,
    state_mutability: ast::StateMutability,
    virtual_: bool,
    override_: Option<&'a ast::Override<'b>>,
    modifiers: &'a [ast::Modifier<'b>],
    returns: &'a [ast::VariableDefinition<'b>],
    anonymous: bool,
    body: Option<&'a [ast::Stmt<'b>]>,
    body_span: Span,
}

/// Language-specific pretty printing.
impl<'ast> State<'_, 'ast> {
    pub fn print_source_unit(&mut self, source_unit: &'ast ast::SourceUnit<'ast>) {
        for item in source_unit.items.iter() {
            self.print_item(item);
        }
        self.print_remaining_comments();
    }

    fn print_item(&mut self, item: &'ast ast::Item<'ast>) {
        let ast::Item { docs, span, kind } = item;
        self.print_docs(docs);
        self.maybe_print_comments(span.lo());
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
                    ast::ImportItems::Plain(_) | ast::ImportItems::Glob(_) => {
                        self.print_ast_str_lit(path);
                        if let Some(ident) = items.source_alias() {
                            self.word(" as ");
                            self.print_ident(ident);
                        }
                    }
                    ast::ImportItems::Aliases(aliases) => {
                        self.s.cbox(self.ind);
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
                        self.s.offset(-self.ind);
                        self.word("}");
                        self.end();
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
                        self.s.cbox(self.ind);
                        self.word("{");
                        self.braces_break();
                        for (pos, (path, op)) in items.iter().delimited() {
                            self.print_path(path);
                            if let Some(op) = op {
                                self.word(" as ");
                                self.word(op.to_str());
                            }
                            if !pos.is_last {
                                self.word(",");
                                self.space();
                            }
                        }
                        self.braces_break();
                        self.s.offset(-self.ind);
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
            ast::ItemKind::Contract(c @ ast::ItemContract { kind, name, layout, bases, body }) => {
                self.contract = Some(c);

                self.s.cbox(self.ind);
                self.s.cbox(0);
                self.word_nbsp(kind.to_str());
                self.print_ident(*name);
                self.nbsp();
                if !bases.is_empty() {
                    self.word("is");
                    self.space();
                    for (pos, base) in bases.iter().delimited() {
                        self.print_modifier_call(base, false);
                        if !pos.is_last {
                            self.word(",");
                            self.space();
                        }
                    }
                    self.space();
                    self.s.offset(-self.ind);
                }
                self.end();
                if let Some(layout) = layout {
                    self.word("layout at ");
                    self.print_expr(layout.slot);
                    self.space();
                }

                self.word("{");
                if !body.is_empty() {
                    self.hardbreak();
                    let comment = self.maybe_print_comments(body[0].span.lo());
                    if self.config.contract_new_lines && comment != Some(CommentStyle::BlankLine) {
                        self.hardbreak();
                    }
                    for item in body.iter() {
                        self.print_item(item);
                    }
                    let comment = self.maybe_print_comments(span.hi());
                    if self.config.contract_new_lines && comment != Some(CommentStyle::BlankLine) {
                        self.hardbreak();
                    }
                } else {
                    self.maybe_print_comments(span.hi());
                    self.zerobreak();
                }
                self.s.offset(-self.ind);
                self.end();
                self.word("}");
                self.hardbreak();

                self.contract = None;
            }
            ast::ItemKind::Function(func) => self.print_function(func),
            ast::ItemKind::Variable(var) => self.print_var_def(var),
            ast::ItemKind::Struct(ast::ItemStruct { name, fields }) => {
                self.s.cbox(self.ind);
                self.word("struct ");
                self.print_ident(*name);
                self.word(" {");
                self.hardbreak_if_nonempty();
                for var in fields.iter() {
                    self.print_var_def(var);
                }
                self.s.offset(-self.ind);
                self.end();
                self.maybe_print_comments(span.hi());
                self.word("}");
                self.hardbreak();
            }
            ast::ItemKind::Enum(ast::ItemEnum { name, variants }) => {
                self.s.cbox(self.ind);
                self.word("enum ");
                self.print_ident(*name);
                self.word(" {");
                self.hardbreak_if_nonempty();
                for (pos, ident) in variants.iter().delimited() {
                    self.print_ident(*ident);
                    self.trailing_comma(pos.is_last);
                }
                self.s.offset(-self.ind);
                self.end();
                self.word("}");
                self.hardbreak();
            }
            ast::ItemKind::Udvt(ast::ItemUdvt { name, ty }) => {
                self.word("type ");
                self.print_ident(*name);
                self.word(" is ");
                self.print_ty(ty);
                self.word(";");
                self.hardbreak();
            }
            ast::ItemKind::Error(ast::ItemError { name, parameters }) => {
                self.print_function_like(FunctionLike {
                    kind: "error",
                    name: Some(*name),
                    parameters,
                    ..Default::default()
                });
            }
            ast::ItemKind::Event(ast::ItemEvent { name, parameters, anonymous }) => {
                self.print_function_like(FunctionLike {
                    kind: "event",
                    name: Some(*name),
                    parameters,
                    anonymous: *anonymous,
                    ..Default::default()
                });
            }
        }
    }

    fn print_function(&mut self, func: &'ast ast::ItemFunction<'ast>) {
        let ast::ItemFunction { kind, header, body, body_span } = func;
        let ast::FunctionHeader {
            name,
            ref parameters,
            visibility,
            state_mutability,
            ref modifiers,
            virtual_,
            ref override_,
            ref returns,
        } = *header;
        self.print_function_like(FunctionLike {
            kind: kind.to_str(),
            name,
            parameters,
            visibility,
            state_mutability,
            virtual_,
            override_: override_.as_ref(),
            modifiers,
            returns,
            anonymous: false,
            body: body.as_deref(),
            body_span: *body_span,
        });
    }

    fn print_function_like(&mut self, args: FunctionLike<'ast, 'ast>) {
        let FunctionLike {
            kind,
            name,
            parameters,
            visibility,
            state_mutability,
            virtual_,
            override_,
            modifiers,
            returns,
            anonymous,
            body,
            body_span,
        } = args;
        self.word(kind);
        if let Some(name) = name {
            self.nbsp();
            self.print_ident(name);
        }
        self.print_parameter_list(parameters);
        if let Some(visibility) = visibility {
            self.nbsp();
            self.word(visibility.to_str());
        }
        if state_mutability != ast::StateMutability::NonPayable {
            self.nbsp();
            self.word(state_mutability.to_str());
        }
        if virtual_ {
            self.nbsp();
            self.word("virtual");
        }
        if let Some(override_) = override_ {
            self.nbsp();
            self.print_override(override_);
        }
        for modifier in modifiers {
            self.nbsp();

            // Add `()` in functions when the modifier is a base contract.
            // HACK: heuristics:
            // 1. exactly matches the name of a base contract as declared in the `contract is`;
            // this does not account for inheritance;
            let is_contract_base = self.contract.is_some_and(|contract| {
                contract.bases.iter().any(|contract_base| contract_base.name == modifier.name)
            });
            // 2. assume that title case names in constructors are bases.
            // LEGACY: constructors used to also be `function NameOfContract...`; not checked.
            let is_constructor = args.kind == "constructor";
            // LEGACY: we are checking the beginning of the path, not the last segment.
            let is_base_contract = is_contract_base ||
                (is_constructor &&
                    modifier.name.first().name.as_str().starts_with(char::is_uppercase));

            self.print_modifier_call(modifier, is_base_contract);
        }
        if !returns.is_empty() {
            self.nbsp();
            self.word("returns");
            self.print_parameter_list(returns);
        }
        if anonymous {
            self.nbsp();
            self.word("anonymous");
        }
        if let Some(body) = body {
            self.nbsp();
            self.print_block(body, body_span);
        } else {
            self.word(";");
            self.hardbreak();
        }
    }

    fn print_var_def(&mut self, var: &'ast ast::VariableDefinition<'ast>) {
        self.print_var(var);
        self.word(";");
        self.maybe_print_trailing_comment(var.span, None);
        self.hardbreak();
    }

    fn print_var(&mut self, var: &'ast ast::VariableDefinition<'ast>) {
        let ast::VariableDefinition {
            span,
            ty,
            visibility,
            mutability,
            data_location,
            override_,
            indexed,
            name,
            initializer,
        } = var;

        if self.handle_span(*span) {
            return;
        }

        self.print_ty(ty);
        if let Some(visibility) = visibility {
            self.nbsp();
            self.word(visibility.to_str());
        }
        if let Some(mutability) = mutability {
            self.nbsp();
            self.word(mutability.to_str());
        }
        if let Some(data_location) = data_location {
            self.nbsp();
            self.word(data_location.to_str());
        }
        if let Some(override_) = override_ {
            self.nbsp();
            self.print_override(override_);
        }
        if *indexed {
            self.nbsp();
            self.word("indexed");
        }
        if let Some(ident) = name {
            self.nbsp();
            self.print_ident(*ident);
        }
        if let Some(initializer) = initializer {
            self.word(" = ");
            self.neverbreak();
            self.print_expr(initializer);
        }
    }

    fn print_parameter_list(&mut self, parameters: &'ast [ast::VariableDefinition<'ast>]) {
        if parameters.is_empty() {
            self.word("()");
            return;
        }
        self.s.cbox(self.ind);
        self.word("(");
        self.zerobreak();
        for (pos, var) in parameters.iter().delimited() {
            self.print_var(var);
            if !pos.is_last {
                self.word(",");
                self.space();
            }
        }
        self.zerobreak();
        self.s.offset(-self.ind);
        self.word(")");
        self.end();
    }

    fn print_docs(&mut self, docs: &'ast ast::DocComments<'ast>) {
        for &ast::DocComment { kind, span, symbol } in docs.iter() {
            self.maybe_print_comments(span.lo());
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

    fn print_ident_or_strlit(&mut self, value: &'ast ast::IdentOrStrLit) {
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
        self.maybe_print_comments(ident.span.lo());
        self.word(ident.to_string());
    }

    fn print_path(&mut self, path: &'ast ast::PathSlice) {
        for (pos, ident) in path.segments().iter().delimited() {
            self.print_ident(*ident);
            if !pos.is_last {
                self.word(".");
            }
        }
    }

    // TODO: Yul literals are slightly different than normal solidity ones
    fn print_lit(&mut self, lit: &'ast ast::Lit) {
        let &ast::Lit { span, symbol, ref kind } = lit;
        if self.handle_span(span) {
            return;
        }

        match *kind {
            ast::LitKind::Str(kind, ..) => {
                self.s.cbox(0);
                for (pos, (span, symbol)) in lit.literals().delimited() {
                    if !self.handle_span(span) {
                        let quote_pos = span.lo() + kind.prefix().len() as u32;
                        self.print_str_lit(kind, quote_pos, symbol.as_str());
                    }
                    if !pos.is_last {
                        self.space_if_not_bol();
                        self.maybe_print_trailing_comment(span, None);
                    } else {
                        self.neverbreak();
                    }
                }
                self.end();
            }
            ast::LitKind::Number(_) | ast::LitKind::Rational(_) => {
                self.print_num_literal(symbol.as_str());
            }
            ast::LitKind::Address(value) => self.word(value.to_string()),
            ast::LitKind::Bool(value) => self.word(if value { "true" } else { "false" }),
            ast::LitKind::Err(_) => self.word(symbol.to_string()),
        }
    }

    fn print_num_literal(&mut self, source: &str) {
        fn strip_underscores_if(b: bool, s: &str) -> Cow<'_, str> {
            if b && s.contains('_') {
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

        let (val, exp) = source.split_once(['e', 'E']).unwrap_or((source, ""));
        let (val, fract) = val.split_once('.').unwrap_or((val, ""));

        let strip_undescores = !config.is_preserve();
        let mut val = &strip_underscores_if(strip_undescores, val)[..];
        let mut exp = &strip_underscores_if(strip_undescores, exp)[..];
        let mut fract = &strip_underscores_if(strip_undescores, fract)[..];

        // strip any padded 0's
        let mut exp_sign = "";
        if !["0x", "0b", "0o"].iter().any(|prefix| source.starts_with(prefix)) {
            val = val.trim_start_matches('0');
            fract = fract.trim_end_matches('0');
            (exp_sign, exp) =
                if let Some(exp) = exp.strip_prefix('-') { ("-", exp) } else { ("", exp) };
            exp = exp.trim_start_matches('0');
        }

        let mut out = String::with_capacity(source.len() * 2);
        if val.is_empty() {
            out.push('0');
        } else {
            add_underscores(&mut out, config, val, false);
        }
        if source.contains('.') {
            out.push('.');
            if !fract.is_empty() {
                add_underscores(&mut out, config, fract, true);
            } else {
                out.push('0');
            }
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
    fn print_ast_str_lit(&mut self, strlit: &'ast ast::StrLit) {
        self.print_str_lit(ast::StrKind::Str, strlit.span.lo(), strlit.value.as_str());
    }

    /// `s` should be the *unescaped contents of the string literal*.
    fn print_str_lit(&mut self, kind: ast::StrKind, quote_pos: BytePos, s: &str) {
        self.maybe_print_comments(quote_pos);
        let s = self.str_lit_to_string(kind, quote_pos, s);
        self.word(s);
    }

    /// `s` should be the *unescaped contents of the string literal*.
    fn str_lit_to_string(&self, kind: ast::StrKind, quote_pos: BytePos, s: &str) -> String {
        let prefix = kind.prefix();
        let quote = match self.config.quote_style {
            config::QuoteStyle::Double => '\"',
            config::QuoteStyle::Single => '\'',
            config::QuoteStyle::Preserve => self.char_at(quote_pos),
        };
        debug_assert!(matches!(quote, '\"' | '\''), "{quote:?}");
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
        let mut s = format!("{prefix}{quote}{s}{quote}");

        // If the output is not a single token then revert to the original quote.
        if Cursor::new(&s).exactly_one().is_err() {
            let other_quote = if quote == '\"' { '\'' } else { '\"' };
            {
                let s = unsafe { s.as_bytes_mut() };
                s[prefix.len()] = other_quote as u8;
                s[s.len() - 1] = other_quote as u8;
            }
            debug_assert!(Cursor::new(&s).exactly_one().map(|_| true).unwrap());
        }

        s
    }

    fn print_ty(&mut self, ty: &'ast ast::Type<'ast>) {
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
                    self.nbsp();
                    self.print_ident(*ident);
                }
                self.word(" => ");
                self.print_ty(value);
                if let Some(ident) = value_name {
                    self.nbsp();
                    self.print_ident(*ident);
                }
                self.word(")");
            }
            ast::TypeKind::Custom(path) => self.print_path(path),
        }
    }

    fn print_override(&mut self, override_: &'ast ast::Override<'ast>) {
        let ast::Override { span, paths } = override_;
        if self.handle_span(*span) {
            return;
        }
        self.word("override");
        if !paths.is_empty() {
            if self.config.override_spacing {
                self.nbsp();
            }
            self.s.cbox(self.ind);
            self.word("(");
            self.zerobreak();
            for (pos, path) in paths.iter().delimited() {
                self.print_path(path);
                if !pos.is_last {
                    self.word(",");
                    self.space();
                }
            }
            self.zerobreak();
            self.s.offset(-self.ind);
            self.word(")");
            self.end();
        }
    }

    /* --- Expressions --- */

    #[expect(unused_variables)]
    fn print_expr(&mut self, expr: &'ast ast::Expr<'ast>) {
        let ast::Expr { span, ref kind } = *expr;
        if self.handle_span(span) {
            return;
        }

        match kind {
            ast::ExprKind::Array(exprs) => {
                self.word("[");
                self.s.cbox(self.ind);
                self.zerobreak();
                for (pos, elem) in exprs.iter().delimited() {
                    self.print_expr(elem);
                    self.trailing_comma(pos.is_last);
                }
                self.s.offset(-self.ind);
                self.end();
                self.word("]");
            }
            ast::ExprKind::Assign(expr, bin_op, expr1) => todo!(),
            ast::ExprKind::Binary(expr, bin_op, expr1) => todo!(),
            ast::ExprKind::Call(expr, call_args) => todo!(),
            ast::ExprKind::CallOptions(expr, named_args) => todo!(),
            ast::ExprKind::Delete(expr) => todo!(),
            ast::ExprKind::Ident(ident) => self.print_ident(*ident),
            ast::ExprKind::Index(expr, index_kind) => todo!(),
            ast::ExprKind::Lit(lit, unit) => {
                self.print_lit(lit);
                if let Some(unit) = unit {
                    self.nbsp();
                    self.word(unit.to_str());
                }
            }
            ast::ExprKind::Member(expr, ident) => todo!(),
            ast::ExprKind::New(_) => todo!(),
            ast::ExprKind::Payable(call_args) => todo!(),
            ast::ExprKind::Ternary(expr, expr1, expr2) => todo!(),
            ast::ExprKind::Tuple(exprs) => todo!(),
            ast::ExprKind::TypeCall(_) => todo!(),
            ast::ExprKind::Type(_) => todo!(),
            ast::ExprKind::Unary(un_op, expr) => {
                let prefix = un_op.kind.is_prefix();
                let op = un_op.kind.to_str();
                if prefix {
                    self.word(op);
                }
                self.print_expr(expr);
                if !prefix {
                    debug_assert!(un_op.kind.is_postfix());
                    self.word(op);
                }
            }
        }
    }

    // If `add_parens_if_empty` is true, then add parentheses `()` even if there are no arguments.
    fn print_modifier_call(
        &mut self,
        modifier: &'ast ast::Modifier<'ast>,
        add_parens_if_empty: bool,
    ) {
        let ast::Modifier { name, arguments } = modifier;
        self.print_path(name);
        if !arguments.is_empty() || add_parens_if_empty {
            self.print_call_args(arguments);
        }
    }

    fn print_call_args(&mut self, args: &'ast ast::CallArgs<'ast>) {
        let ast::CallArgs { span, kind } = args;
        if self.handle_span(*span) {
            return;
        }

        self.s.cbox(self.ind);
        self.word("(");
        match kind {
            ast::CallArgsKind::Unnamed(exprs) => {
                self.zerobreak();
                for (pos, expr) in exprs.iter().delimited() {
                    self.print_expr(expr);
                    if !pos.is_last {
                        self.word(",");
                        self.space();
                    }
                }
                self.zerobreak();
            }
            ast::CallArgsKind::Named(named_args) => {
                self.word("{");
                self.braces_break();
                for (pos, ast::NamedArg { name, value }) in named_args.iter().delimited() {
                    self.print_ident(*name);
                    self.word(": ");
                    self.print_expr(value);
                    if !pos.is_last {
                        self.word(",");
                        self.space();
                    }
                }
                self.braces_break();
                self.word("}");
            }
        }
        self.s.offset(-self.ind);
        self.end();
        self.word(")");
    }

    /* --- Statements --- */

    #[expect(unused_variables)]
    fn print_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) {
        // TODO(dani)
        let ast::Stmt { docs, span, kind } = stmt;
        self.print_docs(docs);
        if self.handle_span(*span) {
            return;
        }
        match kind {
            ast::StmtKind::Assembly(stmt_assembly) => todo!(),
            ast::StmtKind::DeclSingle(variable_definition) => todo!(),
            ast::StmtKind::DeclMulti(variable_definitions, expr) => todo!(),
            ast::StmtKind::Block(stmts) => todo!(),
            ast::StmtKind::Break => self.word("break"),
            ast::StmtKind::Continue => self.word("continue"),
            ast::StmtKind::DoWhile(stmt, expr) => todo!(),
            ast::StmtKind::Emit(path_slice, call_args) => todo!(),
            ast::StmtKind::Expr(expr) => self.print_expr(expr),
            ast::StmtKind::For { init, cond, next, body } => todo!(),
            ast::StmtKind::If(expr, stmt, stmt1) => todo!(),
            ast::StmtKind::Return(expr) => {
                self.word("return");
                if let Some(expr) = expr {
                    self.nbsp();
                    self.print_expr(expr);
                }
            }
            ast::StmtKind::Revert(path_slice, call_args) => todo!(),
            ast::StmtKind::Try(stmt_try) => todo!(),
            ast::StmtKind::UncheckedBlock(block) => {
                self.word("unchecked ");
                self.print_block(block, stmt.span);
            }
            ast::StmtKind::While(expr, stmt) => todo!(),
            ast::StmtKind::Placeholder => self.word("_"),
        }
        if stmt_needs_semi(kind) {
            self.word(";");
        }
        self.maybe_print_comments(stmt.span.hi());
        self.maybe_print_trailing_comment(stmt.span, None);
    }

    fn print_block(&mut self, block: &'ast [ast::Stmt<'ast>], span: Span) {
        // TODO: attempt_single_line, attempt_omit_braces
        self.word("{");
        if block.is_empty() {
            self.maybe_print_comments(span.hi());
        } else if self.single_line_block(block, span) {
            self.space();
            self.print_stmt(&block[0]);
            self.maybe_print_comments(span.hi());
            self.space();
        } else {
            self.s.cbox(self.ind);
            self.hardbreak();
            for stmt in block {
                self.print_stmt(stmt);
                self.hardbreak_if_not_bol();
            }
            self.s.offset(-self.ind);
            self.end();
            self.maybe_print_comments(span.hi());
        }
        self.word("}");
        self.hardbreak();
    }

    fn single_line_block(&self, block: &'ast [ast::Stmt<'_>], span: Span) -> bool {
        if block.len() != 1 {
            return false;
        }
        match self.config.single_line_statement_blocks {
            config::SingleLineBlockStyle::Preserve => self.sm.is_multiline(span),
            config::SingleLineBlockStyle::Single => true,
            config::SingleLineBlockStyle::Multi => false,
        }
    }
}

fn stmt_needs_semi<'ast>(stmt: &'ast ast::StmtKind<'ast>) -> bool {
    match stmt {
        ast::StmtKind::Assembly { .. } |
        ast::StmtKind::Block { .. } |
        ast::StmtKind::For { .. } |
        ast::StmtKind::If { .. } |
        ast::StmtKind::Try { .. } |
        ast::StmtKind::UncheckedBlock { .. } |
        ast::StmtKind::While { .. } => false,

        ast::StmtKind::DeclSingle { .. } |
        ast::StmtKind::DeclMulti { .. } |
        ast::StmtKind::Break { .. } |
        ast::StmtKind::Continue { .. } |
        ast::StmtKind::DoWhile { .. } |
        ast::StmtKind::Emit { .. } |
        ast::StmtKind::Expr { .. } |
        ast::StmtKind::Return { .. } |
        ast::StmtKind::Revert { .. } |
        ast::StmtKind::Placeholder { .. } => true,
    }
}
