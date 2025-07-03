use crate::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    iter::{IterDelimited, IteratorPosition},
    pp::{self, BreakToken, Token},
    FormatterConfig, InlineConfig,
};
use foundry_config::fmt as config;
use itertools::{Either, Itertools};
use solar_parse::{
    ast::{self, token, yul, Span},
    interface::{BytePos, SourceMap},
    Cursor,
};
use std::{borrow::Cow, collections::HashMap};

// TODO(dani): trailing comments should always be passed Some

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

    /// Prints comments that are before the given position.
    ///
    /// Returns `Some` with the style of the last comment printed, or `None` if no comment was
    /// printed.
    fn print_comments(&mut self, pos: BytePos) -> Option<CommentStyle> {
        self.print_comments_inner(pos, false, false)
    }

    fn print_comments_skip_ws(&mut self, pos: BytePos) -> Option<CommentStyle> {
        self.print_comments_inner(pos, true, false)
    }

    fn print_cmnts_with_space_skip_ws(&mut self, pos: BytePos) -> Option<CommentStyle> {
        self.print_comments_inner(pos, true, true)
    }

    fn print_comments_inner(
        &mut self,
        pos: BytePos,
        skip_ws: bool,
        mixed_with_space: bool,
    ) -> Option<CommentStyle> {
        let mut last_style = None;
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= pos {
                break;
            }
            let cmnt = self.next_comment().unwrap();
            if skip_ws && cmnt.style.is_blank() {
                continue;
            }
            last_style = Some(cmnt.style);
            self.print_comment(cmnt, mixed_with_space);
        }
        last_style
    }

    fn print_comment(&mut self, mut cmnt: Comment, with_space: bool) {
        match cmnt.style {
            CommentStyle::Mixed => {
                let never_break = self.last_token_is_neverbreak();
                if !never_break && !self.is_bol_or_only_ind() {
                    if with_space {
                        self.space()
                    } else {
                        self.zerobreak();
                    }
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
                if never_break {
                    self.neverbreak();
                }
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
                // if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                if !self.is_bol_or_only_ind() {
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
        self.comments.peek()
    }

    fn peek_comment_before<'b>(&'b self, pos: BytePos) -> Option<&'b Comment>
    where
        'sess: 'b,
    {
        self.comments.peek().filter(|c| c.pos() < pos)
    }

    fn next_comment(&mut self) -> Option<Comment> {
        self.comments.next()
    }

    fn peek_trailing_comment<'b>(
        &'b self,
        span_pos: BytePos,
        next_pos: Option<BytePos>,
    ) -> Option<&'b Comment>
    where
        'sess: 'b,
    {
        self.comments.peek_trailing_comment(self.sm, span_pos, next_pos)
    }

    fn print_trailing_comment(&mut self, span_pos: BytePos, next_pos: Option<BytePos>) -> bool {
        if let Some(cmnt) = self.comments.trailing_comment(self.sm, span_pos, next_pos) {
            self.print_comment(cmnt, false);
            return true;
        }

        false
    }

    fn print_remaining_comments(&mut self) {
        // If there aren't any remaining comments, then we need to manually
        // make sure there is a line break at the end.
        if self.peek_comment().is_none() && !self.is_bol_or_only_ind() {
            self.hardbreak();
        }
        while let Some(cmnt) = self.next_comment() {
            self.print_comment(cmnt, false);
        }
    }

    fn break_offset_if_not_bol(&mut self, n: usize, off: isize) {
        if !self.is_beginning_of_line() {
            self.break_offset(n, off)
        } else if off != 0 {
            if let Some(last_token) = self.last_token_still_buffered() {
                if last_token.is_hardbreak() {
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

    fn print_tuple<'a, T, P, S>(
        &mut self,
        values: &'a [T],
        pos_lo: BytePos,
        pos_hi: BytePos,
        mut print: P,
        mut get_span: S,
        format: ListFormat,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        // Format single-item inline lists directly without boxes
        if values.len() == 1 && matches!(format, ListFormat::Inline) {
            if let Some(span) = get_span(&values[0]) {
                self.print_comments(span.lo());
            }
            self.word("(");
            print(self, &values[0]);
            self.word(")");
            return;
        }

        // Otherwise, use commasep
        self.word("(");
        self.commasep(values, pos_lo, pos_hi, print, get_span, format);
        self.word(")");
    }

    fn print_array<'a, T, P, S>(&mut self, values: &'a [T], span: Span, print: P, get_span: S)
    where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        self.word("[");
        self.commasep(values, span.lo(), span.hi(), print, get_span, ListFormat::Consistent(false));
        self.word("]");
    }

    fn commasep<'a, T, P, S>(
        &mut self,
        values: &'a [T],
        pos_lo: BytePos,
        pos_hi: BytePos,
        mut print: P,
        mut get_span: S,
        format: ListFormat,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        if values.is_empty() {
            return;
        }

        self.s.cbox(self.ind);
        let mut skip_first_break = false;
        if let Some(first_pos) = get_span(&values[0]).map(Span::lo) {
            if format.breaks_comments() && self.peek_comment_before(first_pos).is_some() {
                // If cmnts should break + comment before the 1st item, force hardbreak.
                self.hardbreak();
            }
            skip_first_break = self.print_trailing_comment(pos_lo, Some(first_pos));
            skip_first_break = self
                .peek_comment_before(first_pos)
                .map_or(skip_first_break, |_| format.breaks_comments());
        }
        if !skip_first_break {
            self.zerobreak();
        }
        if format.is_compact() {
            self.s.cbox(0);
        }

        let mut skip_last_break = false;
        for (i, value) in values.iter().enumerate() {
            let is_last = i == values.len() - 1;
            let span = get_span(value);

            // <cmnt> box(<value><cmnt>)
            if let Some(span) = span {
                if self
                    .print_cmnts_with_space_skip_ws(span.lo())
                    .map_or(false, |cmnt| cmnt.is_mixed()) &&
                    format.breaks_comments()
                {
                    self.hardbreak(); // trailing and isolated comments already hardbreak
                }
            }
            self.s.ibox(0);

            print(self, value);
            if !is_last {
                self.word(",");
            }
            let next_pos = if is_last { None } else { get_span(&values[i + 1]).map(Span::lo) }
                .unwrap_or(pos_hi);
            if !is_last &&
                format.breaks_comments() &&
                self.peek_comment_before(next_pos).map_or(false, |cmnt| cmnt.style.is_mixed())
            {
                self.hardbreak(); // trailing and isolated comments already hardbreak
            }
            self.print_cmnts_with_space_skip_ws(next_pos);
            if is_last && self.is_bol_or_only_ind() {
                // if a trailing comment is printed at the very end, we have to manually adjust
                // the offset to avoid having a double break.
                self.break_offset_if_not_bol(0, -self.ind);
                skip_last_break = true;
            }
            self.end();
            if !is_last && !self.is_bol_or_only_ind() {
                self.space();
            }
        }

        if format.is_compact() {
            self.end();
        }
        if !skip_last_break {
            self.zerobreak();
            self.s.offset(-self.ind);
        }
        self.end();
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
        self.print_comments(span.lo());
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

#[rustfmt::skip]
macro_rules! get_span {
    () => { |value| Some(value.span) };
    (()) => { |value| Some(value.span()) };
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
        let ast::Item { ref docs, span, ref kind } = *item;
        self.print_docs(docs);
        self.print_comments(span.lo());
        match kind {
            ast::ItemKind::Pragma(pragma) => self.print_pragma(pragma),
            ast::ItemKind::Import(import) => self.print_import(import),
            ast::ItemKind::Using(using) => self.print_using(using),
            ast::ItemKind::Contract(contract) => self.print_contract(contract, span),
            ast::ItemKind::Function(func) => self.print_function(func),
            ast::ItemKind::Variable(var) => self.print_var_def(var),
            ast::ItemKind::Struct(strukt) => self.print_struct(strukt, span),
            ast::ItemKind::Enum(enm) => self.print_enum(enm, span),
            ast::ItemKind::Udvt(udvt) => self.print_udvt(udvt),
            ast::ItemKind::Error(err) => self.print_error(err),
            ast::ItemKind::Event(event) => self.print_event(event),
        }
        self.print_comments(span.hi());
        // self.print_trailing_comment(span.hi(), None);
        self.hardbreak_if_not_bol();
    }

    fn print_pragma(&mut self, pragma: &'ast ast::PragmaDirective<'ast>) {
        self.word("pragma ");
        match &pragma.tokens {
            ast::PragmaTokens::Version(ident, semver_req) => {
                self.print_ident(ident);
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
    }

    fn print_import(&mut self, import: &'ast ast::ImportDirective<'ast>) {
        let ast::ImportDirective { path, items } = import;
        self.word("import ");
        match items {
            ast::ImportItems::Plain(_) | ast::ImportItems::Glob(_) => {
                self.print_ast_str_lit(path);
                if let Some(ident) = items.source_alias() {
                    self.word(" as ");
                    self.print_ident(&ident);
                }
            }
            ast::ImportItems::Aliases(aliases) => {
                self.s.cbox(self.ind);
                self.word("{");
                self.braces_break();
                for (pos, (ident, alias)) in aliases.iter().delimited() {
                    self.print_ident(ident);
                    if let Some(alias) = alias {
                        self.word(" as ");
                        self.print_ident(alias);
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
    }

    fn print_using(&mut self, using: &'ast ast::UsingDirective<'ast>) {
        let ast::UsingDirective { list, ty, global } = using;
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
    }

    fn print_contract(&mut self, c: &'ast ast::ItemContract<'ast>, span: Span) {
        let ast::ItemContract { kind, name, layout, bases, body } = c;
        self.contract = Some(c);

        self.s.cbox(self.ind);
        self.cbox(0);
        self.word_nbsp(kind.to_str());
        self.print_ident(name);
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
            if let Some(cmnt) = self.print_comments(body[0].span.lo()) {
                if self.config.contract_new_lines && !cmnt.is_blank() {
                    self.hardbreak();
                }
            }
            for item in body.iter() {
                self.print_item(item);
            }
            if let Some(cmnt) = self.print_comments(span.hi()) {
                if self.config.contract_new_lines && !cmnt.is_blank() {
                    self.hardbreak();
                }
            }
            self.s.offset(-self.ind);
        } else {
            if self.print_comments(span.hi()).is_some() {
                self.zerobreak();
            } else if self.config.bracket_spacing {
                self.nbsp();
            };
        }
        self.end();
        self.word("}");

        self.contract = None;
    }

    fn print_struct(&mut self, strukt: &'ast ast::ItemStruct<'ast>, span: Span) {
        let ast::ItemStruct { name, fields } = strukt;
        self.s.cbox(self.ind);
        self.word("struct ");
        self.print_ident(name);
        self.word(" {");
        self.hardbreak_if_nonempty();
        for var in fields.iter() {
            self.print_var_def(var);
            self.print_trailing_comment(var.span.hi(), None);
            self.hardbreak_if_not_bol();
        }
        self.print_comments_skip_ws(span.hi());
        self.s.offset(-self.ind);
        self.end();
        self.word("}");
    }

    fn print_enum(&mut self, enm: &'ast ast::ItemEnum<'ast>, span: Span) {
        let ast::ItemEnum { name, variants } = enm;
        self.s.cbox(self.ind);
        self.word("enum ");
        self.print_ident(name);
        self.word(" {");
        self.hardbreak_if_nonempty();
        for (pos, ident) in variants.iter().delimited() {
            self.print_ident(ident);
            if !pos.is_last {
                self.word(",");
            }
            self.print_trailing_comment(ident.span.hi(), None);
            self.hardbreak_if_not_bol();
            if !pos.is_last {
                self.hardbreak();
            }
        }
        self.print_comments_skip_ws(span.hi());
        self.s.offset(-self.ind);
        self.end();
        self.word("}");
    }

    fn print_udvt(&mut self, udvt: &'ast ast::ItemUdvt<'ast>) {
        let ast::ItemUdvt { name, ty } = udvt;
        self.word("type ");
        self.print_ident(name);
        self.word(" is ");
        self.print_ty(ty);
        self.word(";");
    }

    fn print_function(&mut self, func: &'ast ast::ItemFunction<'ast>) {
        let ast::ItemFunction { kind, ref header, ref body, body_span } = *func;
        let ast::FunctionHeader {
            name,
            ref parameters,
            visibility,
            state_mutability: sm,
            virtual_,
            ref override_,
            ref returns,
            ..
        } = *header;

        self.cbox(0);
        self.s.cbox(self.ind);
        let fn_start_indent = self.s.current_indent();

        // Print fn name and params
        self.word(kind.to_str());
        if let Some(name) = name {
            self.nbsp();
            self.print_ident(&name);
        }
        self.s.ibox(-self.ind);
        self.print_parameter_list(parameters, parameters.span, ListFormat::Compact(true));
        self.end();

        // Map attributes to their corresponding comments
        let (attributes, mut map) =
            AttributeCommentMapper::new(header.returns.span.lo()).build(self, header);

        if !(attributes.len() == 1 && attributes[0].is_non_payable()) {
            self.space();
        }

        self.s.cbox(0);
        self.s.cbox(0);
        // Print fn attributes in correct order
        let mut is_first = true;
        if let Some(v) = visibility {
            self.print_fn_attribute(v.span, &mut map, &mut |s| s.word(v.to_str()), is_first);
            is_first = false;
        }
        if *sm != ast::StateMutability::NonPayable {
            if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                self.space();
            }
            self.print_fn_attribute(sm.span, &mut map, &mut |s| s.word(sm.to_str()), is_first);
            is_first = false;
        }
        if let Some(v) = virtual_ {
            if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                self.space();
            }
            self.print_fn_attribute(v, &mut map, &mut |s| s.word("virtual"), is_first);
            is_first = false;
        }
        if let Some(o) = override_ {
            if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                self.space();
            }
            self.print_fn_attribute(o.span, &mut map, &mut |s| s.print_override(o), is_first);
            is_first = false;
        }
        for m in attributes.iter().filter(|a| matches!(a.kind, AttributeKind::Modifier(_))) {
            if let AttributeKind::Modifier(modifier) = m.kind {
                let is_base = self.is_modifier_a_base_contract(kind, modifier);
                self.print_fn_attribute(
                    m.span,
                    &mut map,
                    &mut |s| s.print_modifier_call(modifier, is_base),
                    is_first,
                );
                is_first = false;
            }
        }
        if !returns.is_empty() {
            if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                self.space();
            }
            self.word("returns ");
            self.print_parameter_list(returns, returns.span, ListFormat::Consistent(false));
        }
        self.print_comments_skip_ws(body_span.lo());
        self.end();

        // Print fn body
        if let Some(body) = body {
            let current_indent = self.s.current_indent();
            let new_line = current_indent > fn_start_indent;
            if !new_line {
                self.nbsp();
                // self.word("{");
                self.end();
                if !body.is_empty() {
                    self.hardbreak();
                }
            } else {
                self.space();
                self.s.offset(-self.ind);
                // self.word("{");
                self.end();
            }
            self.end();
            // self.print_block_without_braces(body, body_span, new_line);
            self.print_block(body, body_span);
            // self.word("}");
        } else {
            self.end();
            self.neverbreak();
            self.s.offset(-self.ind);
            self.end();
            self.word(";");
        }
        self.end();

        // trailing comments after the fn body are wapped
        if self.peek_trailing_comment(body_span.hi(), None).is_some() {
            self.hardbreak();
            self.hardbreak();
            self.print_trailing_comment(body_span.hi(), None);
        }
    }

    fn is_modifier_a_base_contract(
        &self,
        kind: ast::FunctionKind,
        modifier: &'ast ast::Modifier<'ast>,
    ) -> bool {
        // Add `()` in functions when the modifier is a base contract.
        // HACK: heuristics:
        // 1. exactly matches the name of a base contract as declared in the `contract is`;
        // this does not account for inheritance;
        let is_contract_base = self.contract.is_some_and(|contract| {
            contract.bases.iter().any(|contract_base| contract_base.name == modifier.name)
        });
        // 2. assume that title case names in constructors are bases.
        // LEGACY: constructors used to also be `function NameOfContract...`; not checked.
        let is_constructor = matches!(kind, ast::FunctionKind::Constructor);
        // LEGACY: we are checking the beginning of the path, not the last segment.
        is_contract_base ||
            (is_constructor &&
                modifier.name.first().name.as_str().starts_with(char::is_uppercase))
    }

    fn print_error(&mut self, err: &'ast ast::ItemError<'ast>) {
        let ast::ItemError { name, parameters } = err;
        self.word("error ");
        self.print_ident(name);
        self.print_parameter_list(parameters, parameters.span, ListFormat::Consistent(false));
        self.word(";");
    }

    fn print_event(&mut self, event: &'ast ast::ItemEvent<'ast>) {
        let ast::ItemEvent { name, parameters, anonymous } = event;
        self.word("event ");
        self.print_ident(name);
        self.print_parameter_list(parameters, parameters.span, ListFormat::Compact(false));
        if *anonymous {
            self.word(" anonymous");
        }
        self.word(";");
    }

    fn print_var_def(&mut self, var: &'ast ast::VariableDefinition<'ast>) {
        self.print_var(var);
        self.word(";");
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

        self.cbox(0);
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
            // TODO: make `Spanned` and print comments up to the span
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
            self.print_ident(ident);
        }
        if let Some(initializer) = initializer {
            self.word(" = ");
            self.neverbreak();
            self.print_expr(initializer);
        }
        self.end();
    }

    fn print_parameter_list(
        &mut self,
        parameters: &'ast [ast::VariableDefinition<'ast>],
        span: Span,
        format: ListFormat,
    ) {
        self.print_tuple(parameters, span.lo(), span.hi(), Self::print_var, get_span!(), format);
    }

    // NOTE(rusowsky): is this needed?
    fn print_docs(&mut self, docs: &'ast ast::DocComments<'ast>) {
        // Handled with `self.comments`.
        let _ = docs;
    }

    fn print_ident_or_strlit(&mut self, value: &'ast ast::IdentOrStrLit) {
        match value {
            ast::IdentOrStrLit::Ident(ident) => self.print_ident(ident),
            ast::IdentOrStrLit::StrLit(strlit) => self.print_ast_str_lit(strlit),
        }
    }

    fn print_tokens(&mut self, tokens: &[token::Token]) {
        // Leave unchanged.
        let span = Span::join_first_last(tokens.iter().map(|t| t.span));
        self.print_span(span);
    }

    fn print_ident(&mut self, ident: &ast::Ident) {
        self.print_comments_skip_ws(ident.span.lo());
        self.word(ident.to_string());
    }

    fn print_path(&mut self, path: &'ast ast::PathSlice) {
        self.s.cbox(self.ind);
        for (pos, ident) in path.segments().iter().delimited() {
            self.print_ident(ident);
            if !pos.is_last {
                self.zerobreak();
                self.word(".");
            }
        }
        self.end();
    }

    // TODO: Yul literals are slightly different than normal solidity ones
    fn print_lit(&mut self, lit: &'ast ast::Lit) {
        let ast::Lit { span, symbol, ref kind } = *lit;
        if self.handle_span(span) {
            return;
        }

        match *kind {
            ast::LitKind::Str(kind, ..) => {
                self.cbox(0);
                for (pos, (span, symbol)) in lit.literals().delimited() {
                    if !self.handle_span(span) {
                        let quote_pos = span.lo() + kind.prefix().len() as u32;
                        self.print_str_lit(kind, quote_pos, symbol.as_str());
                    }
                    if !pos.is_last {
                        self.space_if_not_bol();
                        self.print_trailing_comment(span.hi(), None);
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
        self.print_comments(quote_pos);
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
                    ast::ElementaryType::Address(true) => {
                        self.word("address payable");
                        break 'b;
                    }
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
            ast::TypeKind::Function(ast::TypeFunction {
                parameters,
                visibility,
                state_mutability,
                returns,
            }) => {
                self.cbox(0);
                self.word("function");
                self.print_parameter_list(parameters, parameters.span, ListFormat::Inline);
                self.space();

                if let Some(v) = visibility {
                    self.word(v.to_str());
                    self.nbsp();
                }
                if !matches!(**state_mutability, ast::StateMutability::NonPayable) {
                    self.word(state_mutability.to_str());
                    self.nbsp();
                }
                if !returns.is_empty() {
                    self.word("returns");
                    self.nbsp();
                    self.print_parameter_list(returns, returns.span, ListFormat::Consistent(false));
                }
                self.end();
            }
            ast::TypeKind::Mapping(ast::TypeMapping { key, key_name, value, value_name }) => {
                self.word("mapping(");
                self.print_ty(key);
                if let Some(ident) = key_name {
                    self.nbsp();
                    self.print_ident(ident);
                }
                self.word(" => ");
                self.print_ty(value);
                if let Some(ident) = value_name {
                    self.nbsp();
                    self.print_ident(ident);
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
            self.print_tuple(
                paths,
                span.lo(),
                span.hi(),
                |this, path| this.print_path(path),
                get_span!(()),
                ListFormat::Consistent(false),
            );
        }
    }

    /* --- Expressions --- */

    fn print_expr(&mut self, expr: &'ast ast::Expr<'ast>) {
        let ast::Expr { span, ref kind } = *expr;
        if self.handle_span(span) {
            return;
        }

        match kind {
            ast::ExprKind::Array(exprs) => {
                self.print_array(exprs, expr.span, |this, e| this.print_expr(e), get_span!())
            }
            ast::ExprKind::Assign(lhs, None, rhs) => {
                self.ibox(0);
                self.print_expr(lhs);
                self.word(" = ");
                self.neverbreak();
                self.print_expr(rhs);
                self.end();
            }
            ast::ExprKind::Assign(lhs, Some(bin_op), rhs) |
            ast::ExprKind::Binary(lhs, bin_op, rhs) => {
                self.s.ibox(self.ind);
                self.s.ibox(-self.ind);
                self.print_expr(lhs);
                self.end();
                self.space();
                self.word(bin_op.kind.to_str());
                if matches!(kind, ast::ExprKind::Assign(..)) {
                    self.word("=");
                }
                self.nbsp();
                self.print_expr(rhs);
                self.end();
            }
            ast::ExprKind::Call(expr, call_args) => {
                self.print_expr(expr);
                self.print_call_args(call_args);
            }
            ast::ExprKind::CallOptions(expr, named_args) => {
                self.print_expr(expr);
                self.print_named_args(named_args);
            }
            ast::ExprKind::Delete(expr) => {
                self.word("delete ");
                self.print_expr(expr);
            }
            ast::ExprKind::Ident(ident) => self.print_ident(ident),
            ast::ExprKind::Index(expr, kind) => {
                self.print_expr(expr);
                self.word("[");
                self.s.cbox(self.ind);

                let mut skip_break = false;
                match kind {
                    ast::IndexKind::Index(expr) => {
                        if let Some(expr) = expr {
                            self.zerobreak();
                            self.print_expr(expr);
                        }
                    }
                    ast::IndexKind::Range(expr0, expr1) => {
                        if let Some(expr0) = expr0 {
                            if self.print_comments(expr0.span.lo()).map_or(true, |s| s.is_mixed()) {
                                self.zerobreak();
                            }
                            self.print_expr(expr0);
                        } else {
                            self.zerobreak();
                        }
                        self.word(":");
                        if let Some(expr1) = expr1 {
                            self.s.ibox(self.ind);
                            if expr0.is_some() {
                                self.zerobreak();
                            }
                            self.print_cmnts_with_space_skip_ws(expr1.span.lo());
                            self.print_expr(expr1);
                        }

                        let mut is_trailing = false;
                        if let Some(style) = self.print_cmnts_with_space_skip_ws(span.hi()) {
                            skip_break = true;
                            is_trailing = style.is_trailing();
                        }

                        // Manually revert indentation if there is `expr1` and/or comments.
                        if skip_break && expr1.is_some() {
                            self.break_offset_if_not_bol(0, -2 * self.ind);
                            self.end();
                            // if a trailing comment is printed at the very end, we have to manually
                            // adjust the offset to avoid having a double break.
                            if !is_trailing {
                                self.break_offset_if_not_bol(0, -self.ind);
                            }
                        } else if skip_break {
                            self.break_offset_if_not_bol(0, -self.ind);
                        } else if expr1.is_some() {
                            self.end();
                        }
                    }
                }
                if !skip_break {
                    self.zerobreak();
                    self.s.offset(-self.ind);
                }
                self.end();
                self.word("]");
            }
            ast::ExprKind::Lit(lit, unit) => {
                self.print_lit(lit);
                if let Some(unit) = unit {
                    self.nbsp();
                    self.word(unit.to_str());
                }
            }
            ast::ExprKind::Member(expr, ident) => {
                self.print_expr(expr);
                if self.print_trailing_comment(expr.span.hi(), Some(ident.span.lo())) {
                    // if a trailing comment is printed at the very end, we have to manually adjust
                    // the offset to avoid having a double break.
                    self.break_offset_if_not_bol(0, self.ind);
                }
                self.word(".");
                self.print_ident(ident);
            }
            ast::ExprKind::New(ty) => {
                self.word("new ");
                self.print_ty(ty);
            }
            ast::ExprKind::Payable(args) => {
                self.word("payable");
                self.print_call_args(args);
            }
            ast::ExprKind::Ternary(cond, then, els) => {
                self.s.cbox(self.ind);
                // conditional expression
                self.s.ibox(0);
                self.print_comments(cond.span.lo());
                self.print_expr(cond);
                let cmnt = self.peek_comment_before(then.span.lo());
                if cmnt.is_some() {
                    self.space();
                }
                self.print_comments(then.span.lo());
                self.end();
                if !self.is_bol_or_only_ind() {
                    self.space();
                }
                // then expression
                self.s.ibox(0);
                self.word("? ");
                self.print_expr(then);
                let cmnt = self.peek_comment_before(els.span.lo());
                if cmnt.is_some() {
                    self.space();
                }
                self.print_comments(els.span.lo());
                self.end();
                if !self.is_bol_or_only_ind() {
                    self.space();
                }
                // then expression
                self.s.ibox(0);
                self.word(": ");
                self.print_expr(els);
                self.end();
                self.neverbreak();
                self.s.offset(-self.ind);
                self.end();
            }
            ast::ExprKind::Tuple(exprs) => self.print_tuple(
                exprs,
                span.lo(),
                span.hi(),
                |this, expr| {
                    if let Some(expr) = expr {
                        this.print_expr(expr);
                    }
                },
                |e| e.as_deref().map(|e| e.span),
                ListFormat::Consistent(false),
            ),
            ast::ExprKind::TypeCall(ty) => {
                self.word("type");
                self.print_tuple(
                    std::slice::from_ref(ty),
                    span.lo(),
                    span.hi(),
                    Self::print_ty,
                    get_span!(),
                    ListFormat::Consistent(false),
                );
            }
            ast::ExprKind::Type(ty) => self.print_ty(ty),
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
        let ast::CallArgs { span, ref kind } = *args;
        if self.handle_span(span) {
            return;
        }

        match kind {
            ast::CallArgsKind::Unnamed(exprs) => {
                self.print_tuple(
                    exprs,
                    span.lo(),
                    span.hi(),
                    |this, e| this.print_expr(e),
                    get_span!(),
                    ListFormat::Compact(true),
                );
            }
            ast::CallArgsKind::Named(named_args) => {
                self.word("(");
                self.print_named_args(named_args);
                self.word(")");
            }
        }
    }

    fn print_named_args(&mut self, args: &'ast [ast::NamedArg<'ast>]) {
        self.word("{");
        self.s.ibox(self.ind);
        let (should_print, pos) = match (args.first(), self.peek_comment()) {
            (Some(arg), Some(cmnt)) => (cmnt.pos() < arg.name.span.lo(), arg.name.span.lo()),
            _ => (false, BytePos::from_u32(0)),
        };
        if should_print {
            self.space();
            self.print_comments_skip_ws(pos);
        } else {
            self.braces_break();
        }
        self.cbox(0);

        for (i, ast::NamedArg { name, value }) in args.iter().enumerate() {
            self.print_ident(name);
            self.word(": ");
            self.print_expr(value);
            if i == args.len() - 1 {
                self.braces_break();
            } else {
                self.word(",");
                if self.print_comments_skip_ws(args[i + 1].name.span.lo()).is_none() {
                    self.space();
                }
            }
        }
        self.s.offset(-self.ind);
        self.end();
        self.end();
        self.word("}");
    }

    /* --- Statements --- */

    fn print_stmt(&mut self, stmt: &'ast ast::Stmt<'ast>) {
        let ast::Stmt { ref docs, span, ref kind } = *stmt;
        self.print_docs(docs);
        if self.handle_span(span) {
            return;
        }
        match kind {
            ast::StmtKind::Assembly(ast::StmtAssembly { dialect, flags, block }) => {
                self.word("assembly ");
                if let Some(dialect) = dialect {
                    self.print_ast_str_lit(dialect);
                    self.nbsp();
                }
                if !flags.is_empty() {
                    self.print_tuple(
                        flags,
                        span.lo(),
                        span.hi(),
                        Self::print_ast_str_lit,
                        get_span!(),
                        ListFormat::Consistent(false),
                    );
                }
                self.print_yul_block(block, span, false);
            }
            ast::StmtKind::DeclSingle(var) => self.print_var(var),
            ast::StmtKind::DeclMulti(vars, expr) => {
                self.print_tuple(
                    vars,
                    span.lo(),
                    span.hi(),
                    |this, var| {
                        if let Some(var) = var {
                            this.print_var(var);
                        }
                    },
                    |v| v.as_ref().map(|v| v.span),
                    ListFormat::Consistent(false),
                );
                self.word(" = ");
                self.neverbreak();
                self.print_expr(expr);
            }
            ast::StmtKind::Block(stmts) => self.print_block(stmts, span),
            ast::StmtKind::Break => self.word("break"),
            ast::StmtKind::Continue => self.word("continue"),
            ast::StmtKind::DoWhile(stmt, cond) => {
                self.word("do ");
                self.print_stmt_as_block(stmt, BlockFormat::Regular);
                self.nbsp();
                self.print_if_cond("while", cond, cond.span.hi());
            }
            ast::StmtKind::Emit(path, args) => self.print_emit_revert("emit", path, args),
            ast::StmtKind::Expr(expr) => self.print_expr(expr),
            ast::StmtKind::For { init, cond, next, body } => {
                self.cbox(0);
                self.s.ibox(self.ind);
                self.word("for (");
                self.zerobreak();
                self.s.cbox(0);
                if let Some(init) = init {
                    self.print_stmt(init);
                } else {
                    self.word(";");
                }
                if let Some(cond) = cond {
                    self.space();
                    self.print_expr(cond);
                } else {
                    self.zerobreak();
                }
                self.word(";");
                if let Some(next) = next {
                    self.space();
                    self.print_expr(next);
                } else {
                    self.zerobreak();
                }
                self.break_offset_if_not_bol(0, -self.ind);
                self.end();
                self.word(") ");
                self.neverbreak();
                self.end();
                self.print_stmt_as_block(body, BlockFormat::Regular);
                self.end();
            }
            ast::StmtKind::If(cond, then, els_opt) => {
                self.cbox(0);
                self.ibox(0);
                self.print_if_no_else(cond, then);
                let mut els_opt = els_opt.as_deref();
                while let Some(els) = els_opt {
                    self.print_comments_skip_ws(els.span.lo());
                    if self.ends_with('}') {
                        self.nbsp();
                    } else {
                        self.hardbreak_if_not_bol();
                    }
                    self.ibox(0);
                    self.word("else ");
                    if let ast::StmtKind::If(cond, then, els) = &els.kind {
                        self.print_if_no_else(cond, then);
                        els_opt = els.as_deref();
                        continue;
                    } else {
                        self.print_stmt_as_block(els, BlockFormat::Compact(true));
                    }
                    break;
                }
                self.end();
            }
            ast::StmtKind::Return(expr) => {
                self.word("return");
                if let Some(expr) = expr {
                    self.nbsp();
                    self.print_expr(expr);
                }
            }
            ast::StmtKind::Revert(path, args) => self.print_emit_revert("revert", path, args),
            ast::StmtKind::Try(ast::StmtTry { expr, clauses }) => {
                self.cbox(0);
                if let Some((first, other)) = clauses.split_first() {
                    // Handle 'try' clause
                    let ast::TryCatchClause { args, block, span: try_span, .. } = first;
                    self.ibox(0);
                    self.word("try ");
                    self.print_comments(expr.span.lo());
                    self.print_expr(expr);
                    self.print_comments_skip_ws(
                        args.first().map(|p| p.span.lo()).unwrap_or_else(|| expr.span.lo()),
                    );
                    if !self.is_beginning_of_line() {
                        self.nbsp();
                    }
                    if !args.is_empty() {
                        self.word("returns ");
                        self.print_parameter_list(args, *try_span, ListFormat::Compact(false));
                        self.nbsp();
                    }
                    self.print_block(block, *try_span);

                    let mut skip_ind = false;
                    if self
                        .print_trailing_comment(try_span.hi(), other.first().map(|c| c.span.lo()))
                    {
                        // if a trailing comment is printed at the very end, we have to manually
                        // adjust the offset to avoid having a double break.
                        self.break_offset_if_not_bol(0, self.ind);
                        skip_ind = true;
                    };
                    self.end();

                    // Handle 'catch' clauses
                    let mut should_break = false;
                    for (pos, ast::TryCatchClause { name, args, block, span: catch_span }) in
                        other.iter().delimited()
                    {
                        if !pos.is_first || !skip_ind {
                            self.handle_try_catch_indent(&mut should_break, block.is_empty(), pos);
                        }
                        self.s.cbox(self.ind);
                        self.neverbreak();
                        self.print_comments(catch_span.lo());
                        self.word("catch ");
                        self.neverbreak();
                        if !args.is_empty() {
                            self.print_comments(args[0].span.lo());
                            if let Some(name) = name {
                                self.print_ident(name);
                            }
                            self.print_parameter_list(args, *catch_span, ListFormat::Inline);
                            self.nbsp();
                        }
                        self.s.cbox(-self.ind);
                        self.print_block(block, *catch_span);
                        self.end();
                        self.end();
                    }
                }
                self.end();
            }
            ast::StmtKind::UncheckedBlock(block) => {
                self.word("unchecked ");
                self.print_block(block, stmt.span);
            }
            ast::StmtKind::While(cond, stmt) => {
                self.ibox(0);
                self.print_if_cond("while", cond, stmt.span.lo());
                self.nbsp();
                self.end();
                // TODO: handle braces manually
                // self.print_stmt_as_block(stmt, BlockFormat::NoBraces(false));
                self.print_stmt_as_block(stmt, BlockFormat::Compact(true));
            }
            ast::StmtKind::Placeholder => self.word("_"),
        }
        if stmt_needs_semi(kind) {
            self.word(";");
        }
        self.print_comments(stmt.span.hi());
        self.print_trailing_comment(stmt.span.hi(), None);
    }

    fn print_if_no_else(&mut self, cond: &'ast ast::Expr<'ast>, then: &'ast ast::Stmt<'ast>) {
        self.print_if_cond("if", cond, then.span.lo());
        self.nbsp();
        self.end();
        self.print_stmt_as_block(then, BlockFormat::Compact(true));
    }

    fn print_if_cond(&mut self, kw: &'static str, cond: &'ast ast::Expr<'ast>, pos_hi: BytePos) {
        self.word_nbsp(kw);
        self.print_tuple(
            std::slice::from_ref(cond),
            cond.span.lo(),
            pos_hi,
            Self::print_expr,
            get_span!(),
            ListFormat::Compact(true),
        );
    }

    fn print_emit_revert(
        &mut self,
        kw: &'static str,
        path: &'ast ast::PathSlice,
        args: &'ast ast::CallArgs<'ast>,
    ) {
        self.word_nbsp(kw);
        self.print_path(path);
        self.print_call_args(args);
    }

    fn print_block(&mut self, block: &'ast [ast::Stmt<'ast>], span: Span) {
        self.print_block_inner(block, BlockFormat::Regular, Self::print_stmt, |b| b.span, span);
    }

    fn print_block_without_braces(
        &mut self,
        block: &'ast [ast::Stmt<'ast>],
        span: Span,
        new_line: bool,
    ) {
        self.print_block_inner(
            block,
            BlockFormat::NoBraces(new_line),
            Self::print_stmt,
            |b| b.span,
            span,
        );
    }

    // Body of a if/loop.
    fn print_stmt_as_block(&mut self, stmt: &'ast ast::Stmt<'ast>, format: BlockFormat) {
        let stmts = if let ast::StmtKind::Block(stmts) = &stmt.kind {
            stmts
        } else {
            std::slice::from_ref(stmt)
        };
        // TODO: figure out:
        //  > why stmts are not indented
        //  > why braces are not always printed?
        if matches!(format, BlockFormat::NoBraces(_)) {
            self.word("{");
            self.zerobreak();
            self.s.cbox(self.ind);
        }
        self.print_block_inner(
            stmts,
            format,
            // if attempt_single_line { BlockFormat::Compact(true) } else { BlockFormat::Regular },
            Self::print_stmt,
            |b| b.span,
            stmt.span,
        );
        if matches!(format, BlockFormat::NoBraces(_)) {
            self.s.offset(-self.ind);
            self.end();
            self.word("}");
        }
    }

    fn print_yul_block(
        &mut self,
        block: &'ast [yul::Stmt<'ast>],
        span: Span,
        attempt_single_line: bool,
    ) {
        self.print_block_inner(
            block,
            if attempt_single_line { BlockFormat::Compact(false) } else { BlockFormat::Regular },
            Self::print_yul_stmt,
            |b| b.span,
            span,
        );
    }

    fn print_block_inner<T>(
        &mut self,
        block: &'ast [T],
        block_format: BlockFormat,
        mut print: impl FnMut(&mut Self, &'ast T),
        mut get_block_span: impl FnMut(&'ast T) -> Span,
        span: Span,
    ) {
        // Braces are explicitly handled by the caller
        if let BlockFormat::NoBraces(new_line) = block_format {
            if new_line {
                self.hardbreak_if_nonempty();
            }
            for stmt in block {
                print(self, stmt);
                self.hardbreak_if_not_bol();
            }
            self.print_comments_skip_ws(block.last().map_or(span.hi(), |b| get_block_span(b).hi()));
        }
        // Attempt to print in a single line
        else if block_format.attempt_single_line() &&
            block.len() == 1 &&
            self.single_line_block(span)
        {
            self.s.cbox(self.ind);
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.scan_break(BreakToken { pre_break: Some('{'), ..Default::default() });
            } else {
                self.word("{");
                self.space();
            }
            print(self, &block[0]);
            self.print_comments_skip_ws(get_block_span(&block[0]).hi());
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.s.scan_break(BreakToken { post_break: Some('}'), ..Default::default() });
                self.s.offset(-self.ind);
            } else {
                self.space_if_not_bol();
                self.s.offset(-self.ind);
                self.word("}");
            }
            self.end();
        }
        // Empty blocks with comments require special attention
        else if block.is_empty() {
            if let Some(comment) = self.peek_comment() {
                if comment.style.is_trailing() {
                    self.word("{}");
                    self.print_comments_skip_ws(span.hi());
                } else {
                    self.word("{");
                    self.s.cbox(self.ind);
                    self.print_comments_skip_ws(span.hi());
                    // manually adjust offset to ensure that the closing brace is properly indented.
                    // if the last cmnt was breaking, we replace offset to avoid e a double //
                    // break.
                    if self.is_bol_or_only_ind() {
                        self.break_offset_if_not_bol(0, -self.ind);
                    } else {
                        self.s.break_offset(0, -self.ind);
                    }
                    self.end();
                    self.word("}");
                }
            } else {
                self.word("{}");
            }
        }
        // Regular blocks
        else {
            self.word("{");
            self.s.cbox(self.ind);
            self.hardbreak_if_nonempty();
            for stmt in block {
                print(self, stmt);
                self.hardbreak_if_not_bol();
            }
            self.print_comments_skip_ws(block.last().map_or(span.hi(), |b| get_block_span(b).hi()));
            self.s.offset(-self.ind);
            self.end();
            self.word("}");
        }
    }

    fn single_line_block(&self, span: Span) -> bool {
        match self.config.single_line_statement_blocks {
            config::SingleLineBlockStyle::Preserve => !self.sm.is_multiline(span),
            config::SingleLineBlockStyle::Single => true,
            config::SingleLineBlockStyle::Multi => false,
        }
    }
}

/// Yul.
impl<'ast> State<'_, 'ast> {
    fn print_yul_stmt(&mut self, stmt: &'ast yul::Stmt<'ast>) {
        let yul::Stmt { ref docs, span, ref kind } = *stmt;
        self.print_docs(docs);
        if self.handle_span(span) {
            return;
        }

        match kind {
            yul::StmtKind::Block(stmts) => self.print_yul_block(stmts, span, false),
            yul::StmtKind::AssignSingle(path, expr) => {
                self.print_path(path);
                self.word(" := ");
                self.neverbreak();
                self.print_yul_expr(expr);
            }
            yul::StmtKind::AssignMulti(paths, expr_call) => {
                self.commasep(
                    paths,
                    stmt.span.lo(),
                    stmt.span.hi(),
                    |this, path| this.print_path(path),
                    get_span!(()),
                    ListFormat::Consistent(false),
                );
                self.word(" := ");
                self.neverbreak();
                self.print_yul_expr_call(expr_call);
            }
            yul::StmtKind::Expr(expr_call) => self.print_yul_expr_call(expr_call),
            yul::StmtKind::If(expr, stmts) => {
                self.word("if ");
                self.print_yul_expr(expr);
                self.nbsp();
                self.print_yul_block(stmts, span, true);
            }
            yul::StmtKind::For { init, cond, step, body } => {
                // TODO(dani): boxes
                self.ibox(0);

                self.word("for ");
                self.print_yul_block(init, span, true);

                self.space();
                self.print_yul_expr(cond);

                self.space();
                self.print_yul_block(step, span, true);

                self.space();
                self.print_yul_block(body, span, true);

                self.end();
            }
            yul::StmtKind::Switch(yul::StmtSwitch { selector, branches, default_case }) => {
                self.word("switch ");
                self.print_yul_expr(selector);

                self.print_trailing_comment(selector.span.hi(), None);

                for yul::StmtSwitchCase { constant, body } in branches.iter() {
                    self.hardbreak_if_not_bol();
                    self.word("case ");
                    self.print_lit(constant);
                    self.nbsp();
                    self.print_yul_block(body, span, true);

                    self.print_trailing_comment(selector.span.hi(), None);
                }

                if let Some(default_case) = default_case {
                    self.hardbreak_if_not_bol();
                    self.word("default ");
                    self.print_yul_block(default_case, span, true);
                }
            }
            yul::StmtKind::Leave => self.word("leave"),
            yul::StmtKind::Break => self.word("break"),
            yul::StmtKind::Continue => self.word("continue"),
            yul::StmtKind::FunctionDef(yul::Function { name, parameters, returns, body }) => {
                self.cbox(0);
                self.ibox(0);
                self.word("function ");
                self.print_ident(name);
                self.print_tuple(
                    parameters,
                    span.lo(),
                    span.hi(),
                    Self::print_ident,
                    get_span!(),
                    ListFormat::Consistent(false),
                );
                self.nbsp();
                if !returns.is_empty() {
                    self.word("-> ");
                    self.commasep(
                        returns,
                        stmt.span.lo(),
                        stmt.span.hi(),
                        Self::print_ident,
                        get_span!(),
                        ListFormat::Consistent(false),
                    );
                    self.nbsp();
                }
                self.end();
                self.print_yul_block(body, span, false);
                self.end();
            }
            yul::StmtKind::VarDecl(idents, expr) => {
                self.ibox(0);
                self.word("let ");
                self.commasep(
                    idents,
                    stmt.span.lo(),
                    stmt.span.hi(),
                    Self::print_ident,
                    get_span!(),
                    ListFormat::Consistent(false),
                );
                if let Some(expr) = expr {
                    self.word(" := ");
                    self.neverbreak();
                    self.end();
                    self.print_yul_expr(expr);
                } else {
                    self.end();
                }
            }
        }
        self.print_comments(span.hi());
        self.print_trailing_comment(span.hi(), None);
        self.hardbreak_if_not_bol();
    }

    fn print_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) {
        let yul::Expr { span, ref kind } = *expr;
        if self.handle_span(span) {
            return;
        }

        match kind {
            yul::ExprKind::Path(path) => self.print_path(path),
            yul::ExprKind::Call(call) => self.print_yul_expr_call(call),
            yul::ExprKind::Lit(lit) => self.print_lit(lit),
        }
    }

    fn print_yul_expr_call(&mut self, expr: &'ast yul::ExprCall<'ast>) {
        let yul::ExprCall { name, arguments } = expr;
        self.print_ident(name);
        self.print_tuple(
            arguments,
            Span::DUMMY.lo(),
            Span::DUMMY.hi(),
            Self::print_yul_expr,
            get_span!(),
            ListFormat::Consistent(false),
        );
    }

    fn handle_try_catch_indent(
        &mut self,
        should_break: &mut bool,
        empty_block: bool,
        pos: IteratorPosition,
    ) {
        // Add extra indent if all prev 'catch' stmts are empty
        if *should_break {
            if self.is_bol_or_only_ind() {
                self.zerobreak();
            } else {
                self.nbsp();
            }
        } else if empty_block {
            if self.is_bol_or_only_ind() {
                self.zerobreak();
            } else {
                self.space();
            }
            self.s.offset(self.ind);
        } else {
            if pos.is_first {
                self.nbsp();
            } else if self.is_bol_or_only_ind() {
                self.zerobreak();
            } else {
                self.space();
            }
            *should_break = true;
        }
    }

    fn print_fn_attribute(
        &mut self,
        span: Span,
        map: &mut HashMap<BytePos, (Vec<Comment>, Vec<Comment>)>,
        print_fn: &mut dyn FnMut(&mut Self),
        is_first: bool,
    ) {
        match map.remove(&span.lo()) {
            Some((pre_comments, post_comments)) => {
                for cmnt in pre_comments {
                    self.print_comment(cmnt, false);
                }
                if !is_first && !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                    self.space();
                }
                self.ibox(0);
                print_fn(self);
                for cmnt in post_comments {
                    self.print_comment(cmnt, true);
                }
                self.end();
            }
            // Fallback for attributes with no comments
            None => {
                if !is_first && !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                    self.space();
                }
                print_fn(self);
            }
        }
    }
}

// -- HELPERS -----------------------------------------------------------------
// TODO(rusowsky): move to its own file

/// Formatting style for comma-separated lists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListFormat {
    /// Breaks all elements if any break.
    Consistent(bool),
    /// Attempts to fit all elements in one line, before breaking consistently.
    /// The boolean indicates whether mixed comments should force a break.
    Compact(bool),
    /// If the list contains just one element, it will print unboxed (will not break).
    /// Otherwise, will break consistently.
    Inline,
}

impl ListFormat {
    pub(crate) fn breaks_comments(&self) -> bool {
        match self {
            Self::Consistent(yes) => *yes,
            Self::Compact(yes) => *yes,
            Self::Inline => false,
        }
    }

    pub(crate) fn is_compact(&self) -> bool {
        matches!(self, Self::Compact(_))
    }
}

/// Formatting style for code blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockFormat {
    Regular,
    /// Attempts to fit all elements in one line, before breaking consistently. Flags whether to
    /// use braces or not.
    Compact(bool),
    /// Prints blocks without braces. Flags whether the block starts at a new line or not.
    /// Usefull for complex box/break setups.
    NoBraces(bool),
}

impl BlockFormat {
    pub(crate) fn attempt_single_line(&self) -> bool {
        matches!(self, Self::Compact(_))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum AttributeKind<'ast> {
    Visibility(ast::Visibility),
    StateMutability(ast::StateMutability),
    Virtual,
    Override(&'ast ast::Override<'ast>),
    Modifier(&'ast ast::Modifier<'ast>),
}

#[derive(Debug, Clone)]
pub(crate) struct AttributeInfo<'ast> {
    pub(crate) kind: AttributeKind<'ast>,
    pub(crate) span: Span,
}

impl<'ast> AttributeInfo<'ast> {
    fn is_non_payable(&self) -> bool {
        matches!(self.kind, AttributeKind::StateMutability(ast::StateMutability::NonPayable))
    }
}

/// Helper struct to map attributes to their associated comments in function headers.
pub(crate) struct AttributeCommentMapper<'ast> {
    limit_pos: BytePos,
    comments: Vec<Comment>,
    attributes: Vec<AttributeInfo<'ast>>,
}

impl<'ast> AttributeCommentMapper<'ast> {
    pub(crate) fn new(limit_pos: BytePos) -> Self {
        Self { limit_pos, comments: Vec::new(), attributes: Vec::new() }
    }

    pub(crate) fn build(
        mut self,
        state: &mut State<'_, 'ast>,
        header: &'ast ast::FunctionHeader<'ast>,
    ) -> (Vec<AttributeInfo<'ast>>, HashMap<BytePos, (Vec<Comment>, Vec<Comment>)>) {
        self.collect_attributes(header);
        self.cache_comments(state);
        self.map()
    }

    fn map(mut self) -> (Vec<AttributeInfo<'ast>>, HashMap<BytePos, (Vec<Comment>, Vec<Comment>)>) {
        let mut map = HashMap::new();
        for a in 0..self.attributes.len() {
            let is_last = a == self.attributes.len() - 1;
            let mut before = Vec::new();
            let mut after = Vec::new();

            let before_limit = self.attributes[a].span.lo();
            let after_limit =
                if !is_last { self.attributes[a + 1].span.lo() } else { self.limit_pos };

            let mut c = 0;
            while c < self.comments.len() {
                if self.comments[c].pos() <= before_limit {
                    before.push(self.comments.remove(c));
                } else if (after.is_empty() || is_last) && self.comments[c].pos() <= after_limit {
                    after.push(self.comments.remove(c));
                } else {
                    c += 1;
                }
            }
            map.insert(before_limit, (before, after));
        }

        (self.attributes, map)
    }

    fn collect_attributes(&mut self, header: &'ast ast::FunctionHeader<'ast>) {
        if let Some(v) = header.visibility {
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::Visibility(*v), span: v.span });
        }
        self.attributes.push(AttributeInfo {
            kind: AttributeKind::StateMutability(*header.state_mutability),
            span: header.state_mutability.span,
        });
        if let Some(v) = header.virtual_ {
            self.attributes.push(AttributeInfo { kind: AttributeKind::Virtual, span: v });
        }
        if let Some(ref o) = header.override_ {
            self.attributes.push(AttributeInfo { kind: AttributeKind::Override(o), span: o.span });
        }
        for m in header.modifiers.iter() {
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::Modifier(m), span: m.span() });
        }
        self.attributes.sort_by_key(|attr| attr.span.lo());
    }

    fn cache_comments(&mut self, state: &mut State<'_, 'ast>) {
        let mut pending = None;
        for cmnt in state.comments.iter() {
            if cmnt.pos() >= self.limit_pos {
                break;
            }
            match pending {
                Some(ref p) => pending = Some(p + 1),
                None => pending = Some(0),
            }
        }
        while let Some(p) = pending {
            if p == 0 {
                pending = None;
            } else {
                pending = Some(p - 1);
            }
            let cmnt = state.next_comment().unwrap();
            if cmnt.style == CommentStyle::BlankLine {
                continue;
            }
            self.comments.push(cmnt);
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
