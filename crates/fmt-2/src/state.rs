use crate::{
    comment::{Comment, CommentStyle},
    comments::Comments,
    iter::{IterDelimited, IteratorPosition},
    pp::{self, BreakToken, Token, SIZE_INFINITY},
    FormatterConfig, InlineConfig,
};
use foundry_config::fmt as config;
use itertools::{Either, Itertools};
use solar_parse::{
    ast::{self, token, yul, Span},
    interface::{BytePos, SourceMap},
    Cursor,
};
use std::{borrow::Cow, collections::HashMap, fmt::Debug};

pub(super) struct State<'sess, 'ast> {
    pub(crate) s: pp::Printer,
    ind: isize,

    sm: &'sess SourceMap,
    comments: Comments,
    config: FormatterConfig,
    inline_config: InlineConfig,

    contract: Option<&'ast ast::ItemContract<'ast>>,
    single_line_stmt: Option<bool>,
    binary_expr: bool,
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
            single_line_stmt: None,
            binary_expr: false,
        }
    }

    /// Prints comments that are before the given position.
    ///
    /// Returns `Some` with the style of the last comment printed, or `None` if no comment was
    /// printed.
    fn print_comments(&mut self, pos: BytePos, mut config: CommentConfig) -> Option<CommentStyle> {
        let mut last_style: Option<CommentStyle> = None;
        let mut all_blank = true;
        let config_cache = config;
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= pos {
                break;
            }
            let cmnt = self.next_comment().unwrap();
            if cmnt.style.is_blank() {
                match config.skip_blanks {
                    Some(Skip::All) => continue,
                    Some(Skip::First) => {
                        if all_blank {
                            continue;
                        }
                    }
                    None => (),
                }
            // Never print blank lines after docs comments
            } else if !cmnt.is_doc {
                all_blank = false;
            }

            // Handle mixed with follow-up comment
            if cmnt.style.is_mixed() {
                if let Some(cmnt) = self.peek_comment_before(pos) {
                    config.mixed_no_break = true;
                    config.mixed_post_nbsp = cmnt.style.is_mixed();
                }

                // Ensure consecutive mixed comments don't have a double-space
                if let Some(style) = last_style {
                    if style.is_mixed() && (config.mixed_post_nbsp || !config.mixed_no_break) {
                        config.mixed_prev_space = false;
                    }
                }
            }

            last_style = Some(cmnt.style);
            self.print_comment(cmnt, config);
            config = config_cache;
        }
        last_style
    }

    fn print_comment(&mut self, mut cmnt: Comment, config: CommentConfig) {
        // // DEBUG
        // if !cmnt.style.is_blank() {
        //     println!("{cmnt:?}");
        //     println!(" > BOL? {}\n", self.is_bol_or_only_ind());
        // }
        match cmnt.style {
            CommentStyle::Mixed => {
                let never_break = self.last_token_is_neverbreak();
                if !self.is_bol_or_only_ind() {
                    match (never_break || config.mixed_no_break, config.mixed_prev_space) {
                        (false, true) => self.space(),
                        (false, false) => self.zerobreak(),
                        (true, true) => self.nbsp(),
                        (true, false) => (),
                    };
                }
                if let Some(last) = cmnt.lines.pop() {
                    self.ibox(0);
                    for line in cmnt.lines {
                        self.word(line);
                        self.hardbreak();
                    }
                    self.word(last);
                    if config.mixed_post_nbsp || never_break {
                        self.nbsp();
                    } else if !config.mixed_no_break {
                        self.space();
                    }
                    self.end();
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
        self.comments.iter().take_while(|c| c.pos() < pos).find(|c| !c.style.is_blank())
    }

    fn peek_comment_between<'b>(&'b self, pos_lo: BytePos, pos_hi: BytePos) -> Option<&'b Comment>
    where
        'sess: 'b,
    {
        self.comments
            .iter()
            .take_while(|c| pos_lo < c.pos() && c.pos() < pos_hi)
            .find(|c| !c.style.is_blank())
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
            self.print_comment(cmnt, CommentConfig::default());
            return true;
        }

        false
    }

    fn print_trailing_comment_no_break(&mut self, span_pos: BytePos, next_pos: Option<BytePos>) {
        if let Some(mut cmnt) = self.comments.trailing_comment(self.sm, span_pos, next_pos) {
            if !self.is_bol_or_only_ind() {
                self.nbsp();
            }

            if cmnt.lines.len() == 1 {
                self.word(cmnt.lines.pop().unwrap());
            } else {
                self.visual_align();
                for (pos, line) in cmnt.lines.into_iter().delimited() {
                    if !line.is_empty() {
                        self.word(line);
                    }
                    if !pos.is_last {
                        self.hardbreak();
                    } else {
                        self.zerobreak();
                    }
                }
                self.end();
            }
        }
    }

    fn print_remaining_comments(&mut self) {
        // If there aren't any remaining comments, then we need to manually
        // make sure there is a line break at the end.
        if self.peek_comment().is_none() && !self.is_bol_or_only_ind() {
            self.hardbreak();
        }
        while let Some(cmnt) = self.next_comment() {
            self.print_comment(cmnt, CommentConfig::default());
        }
    }

    fn break_offset_if_not_bol(&mut self, n: usize, off: isize, search: bool) {
        // When searching, the break token is expected to be inside a closed box. Thus, we will
        // traverse the buffer and evaluate the first non-end token.
        if search {
            // We do something pretty sketchy here: tuck the nonzero offset-adjustment we
            // were going to deposit along with the break into the previous hardbreak.
            self.find_and_replace_last_token_still_buffered(
                pp::Printer::hardbreak_tok_offset(off),
                |token| token.is_hardbreak(),
            );
            return;
        }

        // When not explicitly searching, the break token is expected to be the last token.
        if !self.is_beginning_of_line() {
            self.break_offset(n, off)
        } else if off != 0 {
            if let Some(last_token) = self.last_token_still_buffered() {
                if last_token.is_hardbreak() {
                    // We do something pretty sketchy here: tuck the nonzero offset-adjustment we
                    // were going to deposit along with the break into the previous hardbreak.
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
        is_binary_expr: bool,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        // Format single-item inline lists directly without boxes
        if values.len() == 1 && matches!(format, ListFormat::Inline) {
            self.word("(");
            if let Some(span) = get_span(&values[0]) {
                self.s.cbox(self.ind);
                let mut skip_break = true;
                if self.peek_comment_before(span.hi()).is_some() {
                    self.hardbreak();
                    skip_break = false;
                }
                self.print_comments(span.lo(), CommentConfig::skip_ws().mixed_prev_space());
                print(self, &values[0]);
                if !self.print_trailing_comment(span.hi(), None) && skip_break {
                    self.neverbreak();
                } else {
                    self.break_offset_if_not_bol(0, -self.ind, false);
                }
                self.end();
            } else {
                print(self, &values[0]);
            }

            self.word(")");
            return;
        }

        // Otherwise, use commasep
        self.word("(");
        self.commasep(values, pos_lo, pos_hi, print, get_span, format, is_binary_expr);
        self.word(")");
    }

    fn print_array<'a, T, P, S>(&mut self, values: &'a [T], span: Span, print: P, get_span: S)
    where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        self.word("[");
        self.commasep(
            values,
            span.lo(),
            span.hi(),
            print,
            get_span,
            ListFormat::Consistent(false),
            false,
        );
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
        is_binary_expr: bool,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        if values.is_empty() {
            return;
        }

        let (is_single_without_cmnts, is_binary_with_trailing) = if values.len() == 1 {
            let value_pos = get_span(&values[0]).map(Span::lo).unwrap_or(pos_hi);
            (
                !is_binary_expr && self.peek_comment_before(pos_hi).is_none(),
                is_binary_expr && self.peek_trailing_comment(value_pos, None).is_some(),
            )
        } else {
            (false, false)
        };

        self.s.cbox(self.ind);
        let mut skip_first_break = is_single_without_cmnts;
        if let Some(first_pos) = get_span(&values[0]).map(Span::lo) {
            if self.peek_comment_before(first_pos).is_some() {
                if format.breaks_comments() {
                    // If cmnts should break + comment before the 1st item, force hardbreak.
                    self.hardbreak();
                }
                skip_first_break = true;
            }
            self.print_trailing_comment(pos_lo, Some(first_pos));
        }
        if !skip_first_break {
            self.zerobreak();
        }
        if format.is_compact() {
            self.s.cbox(0);
        }

        let mut skip_last_break = is_single_without_cmnts;
        for (i, value) in values.iter().enumerate() {
            let is_last = i == values.len() - 1;
            let span = get_span(value);
            if let Some(span) = span {
                if self
                    .print_comments(span.lo(), CommentConfig::skip_ws().mixed_prev_space())
                    .map_or(false, |cmnt| cmnt.is_mixed()) &&
                    format.breaks_comments()
                {
                    self.hardbreak(); // trailing and isolated comments already hardbreak
                }
            }

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
            self.print_comments(
                next_pos,
                CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
            );

            if is_binary_with_trailing {
                // binary expressions prints trailing comment inside its boxes, we have to manually
                // adjust the offset to avoid having a double break.
                self.break_offset_if_not_bol(0, -2 * self.ind, true);
                skip_last_break = true;
            } else if is_last && self.is_bol_or_only_ind() {
                // if a trailing comment is printed at the very end, we have to manually adjust
                // the offset to avoid having a double break.
                self.break_offset_if_not_bol(0, -self.ind, false);
                skip_last_break = true;
            }
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
        self.print_comments(span.lo(), CommentConfig::default());
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
        let mut items = source_unit.items.iter().peekable();
        let mut is_first = true;
        while let Some(item) = items.next() {
            self.print_item(item, is_first);
            is_first = false;

            if let Some(next_item) = items.peek() {
                self.separate_items(next_item);
            }
        }
        self.print_remaining_comments();
    }

    fn separate_items(&mut self, next_item: &'ast ast::Item<'ast>) {
        if item_needs_iso(&next_item.kind) &&
            !self.comments.iter().any(|c| c.pos() < next_item.span.lo())
        {
            self.hardbreak();
        }
    }

    fn print_item(&mut self, item: &'ast ast::Item<'ast>, skip_ws: bool) {
        let ast::Item { ref docs, span, ref kind } = *item;
        self.print_docs(docs);
        let add_zero_break = if skip_ws {
            self.print_comments(span.lo(), CommentConfig::skip_ws().mixed_prev_space())
        } else {
            self.print_comments(span.lo(), CommentConfig::default())
        }
        .is_some_and(|cmnt| cmnt.is_mixed());
        if add_zero_break {
            self.zerobreak();
        }

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
        self.print_comments(span.hi(), CommentConfig::default());
        self.print_trailing_comment(span.hi(), None);
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
            if self.config.contract_new_lines {
                self.hardbreak();
            }
            if self.peek_comment_before(body[0].span.lo()).is_some() {
                self.print_comments(body[0].span.lo(), CommentConfig::skip_first_ws());
            }

            let mut items = body.iter().peekable();
            let mut is_first = true;
            while let Some(item) = items.next() {
                self.print_item(item, is_first);
                is_first = false;
                if let Some(next_item) = items.peek() {
                    self.separate_items(next_item);
                }
            }

            if let Some(cmnt) = self.print_comments(span.hi(), CommentConfig::skip_ws()) {
                if self.config.contract_new_lines && !cmnt.is_blank() {
                    self.hardbreak();
                }
            }
            self.s.offset(-self.ind);
            self.end();
            if self.config.contract_new_lines {
                self.hardbreak_if_nonempty();
            }
        } else {
            if self.print_comments(span.hi(), CommentConfig::skip_ws()).is_some() {
                self.zerobreak();
            } else if self.config.bracket_spacing {
                self.nbsp();
            };
            self.end();
        }
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
        self.print_comments(span.hi(), CommentConfig::skip_ws());
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
        self.print_comments(span.hi(), CommentConfig::skip_ws());
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

        self.s.cbox(self.ind);

        // Print fn name and params
        self.word(kind.to_str());
        if let Some(name) = name {
            self.nbsp();
            self.print_ident(&name);
        }
        self.s.cbox(-self.ind);
        self.print_parameter_list(parameters, parameters.span, ListFormat::Consistent(true));
        self.end();

        // Map attributes to their corresponding comments
        let (attributes, mut map) = AttributeCommentMapper::new(
            returns.as_ref().map_or(body_span.lo(), |ret| ret.span.lo()),
        )
        .build(self, header);

        // Print fn attributes in correct order
        self.s.cbox(0);
        let mut is_first = true;
        if let Some(v) = visibility {
            self.print_fn_attribute(v.span, &mut map, &mut |s| s.word(v.to_str()), is_first);
            is_first = false;
        }
        if let Some(sm) = sm {
            if !matches!(*sm, ast::StateMutability::NonPayable) {
                self.print_fn_attribute(sm.span, &mut map, &mut |s| s.word(sm.to_str()), is_first);
                is_first = false;
            }
        }
        if let Some(v) = virtual_ {
            self.print_fn_attribute(v, &mut map, &mut |s| s.word("virtual"), is_first);
            is_first = false;
        }
        if let Some(o) = override_ {
            self.print_fn_attribute(
                o.span,
                &mut map,
                &mut |s| s.print_override(o),
                is_first && o.paths.is_empty(),
            );
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
        let mut empty_returns = true;
        if let Some(ret) = returns {
            if !ret.is_empty() {
                if !self.is_bol_or_only_ind() && !self.last_token_is_space() {
                    self.space();
                }
                self.word("returns ");
                self.print_parameter_list(ret, ret.span, ListFormat::Consistent(false));
                empty_returns = false;
            }
        }

        // Print fn body
        if let Some(body) = body {
            if self.peek_comment_before(body_span.lo()).map_or(true, |cmnt| cmnt.style.is_mixed()) {
                if attributes.len() == 1 && empty_returns && override_.is_none() {
                    self.nbsp();
                    self.zerobreak();
                } else {
                    self.space();
                }
                self.s.offset(-self.ind);
                self.print_comments(body_span.lo(), CommentConfig::skip_ws());
            } else {
                self.zerobreak();
                self.s.offset(-self.ind);
                self.print_comments(body_span.lo(), CommentConfig::skip_ws());
                self.s.offset(-self.ind);
            }
            self.word("{");
            self.end();
            self.end();
            self.print_block_without_braces(body, body_span, Some(self.ind));
            self.word("}");
        } else {
            self.print_comments(body_span.lo(), CommentConfig::skip_ws().mixed_prev_space());
            self.end();
            self.end();
            self.neverbreak();
            self.word(";");
        }

        if self.peek_trailing_comment(body_span.hi(), None).is_some() {
            // trailing comments after the fn body are isolated
            if self.config.wrap_comments {
                self.hardbreak();
                self.hardbreak();
            }
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

        self.ibox(0);
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
            // TODO(rusowsky): make `Spanned` and print comments up to the span
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
        self.print_tuple(
            parameters,
            span.lo(),
            span.hi(),
            Self::print_var,
            get_span!(),
            format,
            false,
        );
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
        self.print_comments(ident.span.lo(), CommentConfig::skip_ws());
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
                    self.ibox(0);
                    if !self.handle_span(span) {
                        let quote_pos = span.lo() + kind.prefix().len() as u32;
                        self.print_str_lit(kind, quote_pos, symbol.as_str());
                    }
                    if !pos.is_last {
                        if !self.print_trailing_comment(span.hi(), None) {
                            self.space_if_not_bol();
                        }
                    } else {
                        self.neverbreak();
                    }
                    self.end();
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
        self.print_comments(quote_pos, CommentConfig::default());
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
                if let Some(sm) = state_mutability {
                    if !matches!(**sm, ast::StateMutability::NonPayable) {
                        self.word(sm.to_str());
                        self.nbsp();
                    }
                }
                if let Some(ret) = returns {
                    if !ret.is_empty() {
                        self.word("returns");
                        self.nbsp();
                        self.print_parameter_list(ret, ret.span, ListFormat::Consistent(false));
                    }
                }
                self.end();
            }
            ast::TypeKind::Mapping(ast::TypeMapping { key, key_name, value, value_name }) => {
                self.word("mapping(");
                self.s.cbox(self.ind);
                if let Some(cmnt) = self.peek_comment_before(key.span.lo()) {
                    let is_mixed = cmnt.style.is_mixed();
                    if is_mixed {
                        self.print_comments(
                            key.span.lo(),
                            CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                        );
                        self.break_offset_if_not_bol(SIZE_INFINITY as usize, 0, false);
                    } else {
                        self.print_comments(key.span.lo(), CommentConfig::skip_ws());
                    }
                }
                // Fitting a mapping in one line takes, at least, 16 chars (one-char var name):
                // 'mapping(' + {key} + ' => ' {value} ') ' + {name} + ';'
                // To be more conservative, we use 18 to decide whether to force a break or not.
                else if (18 +
                    self.estimate_size(key.span) as isize +
                    key_name.map(|k| self.estimate_size(k.span)).unwrap_or(0) as isize +
                    self.estimate_size(value.span) as isize +
                    value_name.map(|v| self.estimate_size(v.span)).unwrap_or(0) as isize) >=
                    self.space_left()
                {
                    self.hardbreak();
                } else {
                    self.zerobreak();
                }
                self.s.cbox(0);
                self.print_ty(key);
                if let Some(ident) = key_name {
                    if self
                        .print_comments(
                            ident.span.lo(),
                            CommentConfig::skip_ws()
                                .mixed_no_break()
                                .mixed_prev_space()
                                .mixed_post_nbsp(),
                        )
                        .is_none()
                    {
                        self.nbsp();
                    }
                    self.print_ident(ident);
                }
                // NOTE(rusowsky): unless we add more spans to solar, using `value.span.lo()`
                // consumes "comment6" of which should be printed after the `=>`
                self.print_comments(
                    value.span.lo(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                );
                self.space();
                self.s.offset(self.ind);
                self.word("=> ");
                self.s.ibox(self.ind);
                self.print_ty(value);
                if let Some(ident) = value_name {
                    self.neverbreak();
                    if self
                        .print_comments(
                            ident.span.lo(),
                            CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                        )
                        .is_none()
                    {
                        self.nbsp();
                    }
                    self.print_ident(ident);
                    if self
                        .peek_comment_before(ty.span.hi())
                        .is_some_and(|cmnt| cmnt.style.is_mixed())
                    {
                        self.neverbreak();
                        self.print_comments(
                            value.span.lo(),
                            CommentConfig::skip_ws().mixed_no_break(),
                        );
                    }
                }
                self.end();
                self.end();
                if self
                    .print_comments(
                        ty.span.hi(),
                        CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                    )
                    .is_some_and(|cmnt| !cmnt.is_mixed())
                {
                    self.break_offset_if_not_bol(0, -self.ind, false);
                } else {
                    self.zerobreak();
                    self.s.offset(-self.ind);
                }
                self.end();
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
                false,
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
                let is_parent = matches!(lhs.kind, ast::ExprKind::Binary(..)) ||
                    matches!(rhs.kind, ast::ExprKind::Binary(..));
                let is_child = self.binary_expr;
                if !is_child && is_parent {
                    // top-level expression of the chain -> set cache
                    self.binary_expr = true;
                    self.s.ibox(self.ind);
                } else if !is_child || !is_parent {
                    self.ibox(0);
                }

                self.print_expr(lhs);
                if !matches!(kind, ast::ExprKind::Assign(..)) &&
                    self.peek_trailing_comment(rhs.span.hi(), None).is_none() &&
                    self.peek_comment_before(rhs.span.hi())
                        .map_or(true, |cmnt| cmnt.style.is_mixed())
                {
                    self.space();
                } else if !self.is_bol_or_only_ind() {
                    self.nbsp();
                }

                // box expressions with complex sucessors to accomodate their own indentation
                if !is_child && is_parent {
                    if has_complex_succesor(&rhs.kind, true) {
                        self.s.ibox(-self.ind);
                    } else if has_complex_succesor(&rhs.kind, false) {
                        self.s.ibox(0);
                    }
                }
                self.word(bin_op.kind.to_str());
                if matches!(kind, ast::ExprKind::Assign(..)) {
                    self.word("=");
                }
                self.nbsp();
                self.print_expr(rhs);
                self.print_trailing_comment(rhs.span.hi(), None);

                if (has_complex_succesor(&rhs.kind, false) || has_complex_succesor(&rhs.kind, true)) &&
                    (!is_child && is_parent)
                {
                    self.end();
                }

                if !is_child {
                    // top-level expression of the chain -> clear cache
                    self.binary_expr = false;
                    self.end();
                } else if !is_parent {
                    self.end();
                }
            }
            ast::ExprKind::Call(expr, call_args) => {
                self.print_expr(expr);
                self.print_call_args(call_args);
            }
            ast::ExprKind::CallOptions(expr, named_args) => {
                self.print_expr(expr);
                self.print_named_args(named_args, span.hi());
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
                            if self
                                .print_comments(expr0.span.lo(), CommentConfig::skip_ws())
                                .map_or(true, |s| s.is_mixed())
                            {
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
                            self.print_comments(
                                expr1.span.lo(),
                                CommentConfig::skip_ws()
                                    .mixed_prev_space()
                                    .mixed_no_break()
                                    .mixed_post_nbsp(),
                            );
                            self.print_expr(expr1);
                        }

                        let mut is_trailing = false;
                        if let Some(style) = self.print_comments(
                            span.hi(),
                            CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                        ) {
                            skip_break = true;
                            is_trailing = style.is_trailing();
                        }

                        // Manually revert indentation if there is `expr1` and/or comments.
                        if skip_break && expr1.is_some() {
                            self.break_offset_if_not_bol(0, -2 * self.ind, false);
                            self.end();
                            // if a trailing comment is printed at the very end, we have to manually
                            // adjust the offset to avoid having a double break.
                            if !is_trailing {
                                self.break_offset_if_not_bol(0, -self.ind, false);
                            }
                        } else if skip_break {
                            self.break_offset_if_not_bol(0, -self.ind, false);
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
                    self.break_offset_if_not_bol(0, self.ind, false);
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
                self.print_comments(cond.span.lo(), CommentConfig::skip_ws());
                self.print_expr(cond);
                let cmnt = self.peek_comment_before(then.span.lo());
                if cmnt.is_some() {
                    self.space();
                }
                self.print_comments(then.span.lo(), CommentConfig::skip_ws());
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
                self.print_comments(els.span.lo(), CommentConfig::skip_ws());
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
                is_binary_expr(&expr.kind),
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
                    false,
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
                    false,
                );
            }
            ast::CallArgsKind::Named(named_args) => {
                self.word("(");
                self.print_named_args(named_args, span.hi());
                self.word(")");
            }
        }
    }

    fn print_named_args(&mut self, args: &'ast [ast::NamedArg<'ast>], pos_hi: BytePos) {
        self.word("{");

        // Use the start position of the first argument's name for comment processing.
        let list_lo = args.first().map_or(pos_hi, |arg| arg.name.span.lo());
        let ind = self.ind;

        self.commasep(
            args,
            list_lo,
            pos_hi,
            // Closure to print a single named argument (`name: value`)
            |s, arg| {
                s.cbox(ind);
                s.print_ident(&arg.name);
                // Use a non-breaking space after the colon to keep `name: value` together.
                s.word(":");
                if !s.print_trailing_comment(arg.name.span.hi(), None) {
                    s.nbsp();
                }
                s.print_comments(
                    arg.value.span.lo(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_post_nbsp(),
                );
                s.print_expr(&arg.value);
                s.end();
            },
            // Closure to provide the full span of a single named argument.
            // This is crucial for `commasep` to handle comments around each argument correctly.
            |arg| Some(ast::Span::new(arg.name.span.lo(), arg.value.span.hi())),
            // Use Compact format: it tries to fit everything on one line,
            // and breaks consistently if it doesn't fit or if comments interfere.
            ListFormat::Consistent(true),
            false,
        );

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
                        false,
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
                    false,
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
                self.print_stmt_as_block(stmt, false);
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
                self.break_offset_if_not_bol(0, -self.ind, false);
                self.end();
                self.word(") ");
                self.neverbreak();
                self.end();
                self.print_stmt_as_block(body, false);
                self.end();
            }
            ast::StmtKind::If(cond, then, els_opt) => {
                // Check if blocks should be inlined and update cache if necessary
                let inline = self.is_single_line_block(cond, then, els_opt.as_ref());
                if !inline.is_cached && self.single_line_stmt.is_none() {
                    self.single_line_stmt = Some(inline.outcome);
                }

                self.cbox(0);
                self.ibox(0);
                // Print if stmt
                self.print_if_no_else(cond, then, inline.outcome);
                // Print else (if) stmts, if any
                let mut els_opt = els_opt.as_deref();
                while let Some(els) = els_opt {
                    if self.ends_with('}') {
                        match self.print_comments(els.span.lo(), CommentConfig::skip_ws()) {
                            Some(cmnt) => {
                                if cmnt.is_mixed() {
                                    self.hardbreak()
                                }
                            }
                            None => self.nbsp(),
                        }
                    } else {
                        self.hardbreak_if_not_bol();
                        self.print_comments(els.span.lo(), CommentConfig::skip_ws());
                    }
                    self.ibox(0);
                    self.word("else ");
                    if let ast::StmtKind::If(cond, then, els) = &els.kind {
                        self.print_if_no_else(cond, then, inline.outcome);
                        els_opt = els.as_deref();
                        continue;
                    } else {
                        self.print_stmt_as_block(els, inline.outcome);
                        self.end();
                    }
                    break;
                }
                self.end();

                // Clear cache if necessary
                if !inline.is_cached && self.single_line_stmt.is_some() {
                    self.single_line_stmt = None;
                }
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
                    self.print_comments(expr.span.lo(), CommentConfig::skip_ws());
                    self.print_expr(expr);
                    self.print_comments(
                        args.first().map(|p| p.span.lo()).unwrap_or_else(|| expr.span.lo()),
                        CommentConfig::skip_ws(),
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
                        self.break_offset_if_not_bol(0, self.ind, false);
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
                        self.print_comments(catch_span.lo(), CommentConfig::skip_ws());
                        self.word("catch ");
                        self.neverbreak();
                        if !args.is_empty() {
                            self.print_comments(
                                args[0].span.lo(),
                                CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
                            );
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
                // Check if blocks should be inlined and update cache if necessary
                let inline = self.is_single_line_block(cond, stmt, None);
                if !inline.is_cached && self.single_line_stmt.is_none() {
                    self.single_line_stmt = Some(inline.outcome);
                }

                // Print while cond and its statement
                self.print_if_cond("while", cond, stmt.span.lo());
                self.nbsp();
                self.print_stmt_as_block(stmt, inline.outcome);

                // Clear cache if necessary
                if !inline.is_cached && self.single_line_stmt.is_some() {
                    self.single_line_stmt = None;
                }
            }
            ast::StmtKind::Placeholder => self.word("_"),
        }
        if stmt_needs_semi(kind) {
            self.word(";");
        }
        self.print_comments(stmt.span.hi(), CommentConfig::skip_ws());
    }

    fn print_if_no_else(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        inline: bool,
    ) {
        // NOTE(rusowsky): unless we add bracket spans to solar, using `then.span.lo()` consumes
        // "cmnt12" of the IfStatement test inside the preceeding clause
        // self.print_if_cond("if", cond, cond.span.hi());
        self.print_if_cond("if", cond, then.span.lo());
        self.space();
        self.end();
        self.print_stmt_as_block(then, inline);
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
            is_binary_expr(&cond.kind),
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
        offset: Option<isize>,
    ) {
        self.print_block_inner(
            block,
            BlockFormat::NoBraces(offset),
            Self::print_stmt,
            |b| b.span,
            span,
        );
    }

    // Body of a if/loop.
    fn print_stmt_as_block(&mut self, stmt: &'ast ast::Stmt<'ast>, inline: bool) {
        let stmts = if let ast::StmtKind::Block(stmts) = &stmt.kind {
            stmts
        } else {
            std::slice::from_ref(stmt)
        };

        if inline && !stmts.is_empty() {
            self.neverbreak();
            self.print_block_without_braces(stmts, stmt.span, None);
        } else {
            self.word("{");
            self.print_block_without_braces(stmts, stmt.span, Some(self.ind));
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

    fn print_block_inner<T: Debug>(
        &mut self,
        block: &'ast [T],
        block_format: BlockFormat,
        mut print: impl FnMut(&mut Self, &'ast T),
        mut get_block_span: impl FnMut(&'ast T) -> Span,
        span: Span,
    ) {
        // Attempt to print in a single line
        if block_format.attempt_single_line() && block.len() == 1 {
            self.s.cbox(self.ind);
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.scan_break(BreakToken { pre_break: Some('{'), ..Default::default() });
            } else {
                self.word("{");
                self.space();
            }
            print(self, &block[0]);
            self.print_comments(get_block_span(&block[0]).hi(), CommentConfig::skip_ws());
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.s.scan_break(BreakToken { post_break: Some('}'), ..Default::default() });
                self.s.offset(-self.ind);
            } else {
                self.space_if_not_bol();
                self.s.offset(-self.ind);
                self.word("}");
            }
            self.end();
            return;
        }

        // Empty blocks with comments require special attention
        if block.is_empty() {
            // Trailing comments are printed after the block
            if self.peek_comment_before(span.hi()).map_or(true, |cmnt| cmnt.style.is_trailing()) {
                if self.config.bracket_spacing {
                    if block_format.with_braces() {
                        self.word("{ }");
                    } else {
                        self.nbsp();
                    }
                } else if block_format.with_braces() {
                    self.word("{}");
                }
                self.print_comments(span.hi(), CommentConfig::skip_ws());
            }
            // Other comments are printed inside the block
            else {
                if let BlockFormat::NoBraces(offset) = block_format {
                    match offset {
                        Some(offset) => self.s.cbox(offset),
                        None => self.cbox(0),
                    }
                } else {
                    self.word("{");
                    self.s.cbox(self.ind);
                }
                self.print_comments(
                    span.hi(),
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space().mixed_post_nbsp(),
                );
                // manually adjust offset to ensure that the closing brace is properly indented.
                // if the last cmnt was breaking, we replace offset to avoid e a double break.
                if self.is_bol_or_only_ind() {
                    self.break_offset_if_not_bol(0, -self.ind, false);
                } else {
                    self.s.break_offset(0, -self.ind);
                }
                self.end();
                if block_format.with_braces() {
                    self.word("}");
                }
            }
            return;
        }

        match block_format {
            BlockFormat::NoBraces(None) => {
                self.print_comments(get_block_span(&block[0]).lo(), CommentConfig::skip_ws());
                self.s.cbox(0);
            }
            BlockFormat::NoBraces(Some(offset)) => {
                if self.peek_comment_before(get_block_span(&block[0]).lo()).is_some() {
                    self.hardbreak();
                    self.break_offset_if_not_bol(0, offset, false);
                    self.print_comments(get_block_span(&block[0]).lo(), CommentConfig::skip_ws());
                } else {
                    self.zerobreak();
                }
                self.s.offset(offset);
                self.s.cbox(self.ind);
            }
            _ => {
                self.word("{");
                self.s.cbox(self.ind);
                if self
                    .print_comments(get_block_span(&block[0]).lo(), CommentConfig::skip_ws())
                    .map_or(true, |cmnt| cmnt.is_mixed())
                {
                    self.hardbreak_if_nonempty();
                }
            }
        }
        for stmt in block {
            print(self, stmt);
            if block_format.breaks() {
                self.print_trailing_comment_no_break(get_block_span(stmt).hi(), None);
            } else if !self.print_trailing_comment(get_block_span(stmt).hi(), None) {
                self.hardbreak_if_not_bol();
            }
        }
        self.print_comments(
            block.last().map_or(span.hi(), |b| get_block_span(b).hi()),
            CommentConfig::skip_ws().mixed_no_break().mixed_prev_space(),
        );
        if !block_format.breaks() {
            if !self.last_token_is_break() {
                self.hardbreak();
            }
            self.s.offset(-self.ind);
        }
        self.end();
        if block_format.with_braces() {
            self.word("}");
        }
    }

    /// Determines if an `if/else` block should be inlined.
    /// Also returns if the value was cached, so that it can be cleaned afterwards.
    ///
    /// # Returns
    ///
    /// A tuple `(should_inline, was_cached)`. The second boolean is `true` if the
    /// decision was retrieved from the cache or is a final decision based on config,
    /// preventing the caller from clearing a cache value that was never set.
    fn is_single_line_block(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        els_opt: Option<&'ast &'ast mut ast::Stmt<'ast>>,
    ) -> Decision {
        // If a decision is already cached from a parent, use it directly.
        if let Some(cached_decision) = self.single_line_stmt {
            return Decision { outcome: cached_decision, is_cached: true };
        }

        // If possible, take an early decision based on the block style configuration.
        match self.config.single_line_statement_blocks {
            config::SingleLineBlockStyle::Preserve => {
                if self.is_stmt_in_new_line(cond, then) || self.is_multiline_block_stmt(then) {
                    return Decision { outcome: false, is_cached: true };
                }
            }
            config::SingleLineBlockStyle::Single => {
                if self.is_multiline_block_stmt(then) {
                    return Decision { outcome: false, is_cached: true };
                }
            }
            config::SingleLineBlockStyle::Multi => {
                return Decision { outcome: false, is_cached: true };
            }
        };

        // If no decision was made, estimate the length to be formatted.
        // NOTE: conservative check -> worst-case scenario is formatting as multi-line block.
        Decision { outcome: self.can_stmts_fit_on_one_line(cond, then, els_opt), is_cached: false }
    }

    fn is_inline_stmt(&self, stmt: &'ast ast::Stmt<'ast>, cond_len: usize) -> bool {
        if let ast::StmtKind::If(cond, then, els_opt) = &stmt.kind {
            let if_span = Span::new(cond.span.lo(), then.span.hi());
            if self.sm.is_multiline(if_span) &&
                matches!(
                    self.config.single_line_statement_blocks,
                    config::SingleLineBlockStyle::Preserve
                )
            {
                return false;
            }
            if (cond_len + self.estimate_size(if_span)) as isize >= self.space_left() {
                return false;
            }
            if let Some(els) = els_opt {
                if !self.is_inline_stmt(els, 6) {
                    return false;
                }
            }
        } else {
            if matches!(
                self.config.single_line_statement_blocks,
                config::SingleLineBlockStyle::Preserve
            ) && self.sm.is_multiline(stmt.span)
            {
                return false;
            }
            if (cond_len + self.estimate_size(stmt.span)) as isize >= self.space_left() {
                return false;
            }
        }
        true
    }

    /// Checks if a statement was explicitly written in a new line.
    fn is_stmt_in_new_line(
        &self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
    ) -> bool {
        let span_between = cond.span.between(then.span);
        if let Ok(snip) = self.sm.span_to_snippet(span_between) {
            // Check for newlines after the closing parenthesis of the `if (...)`.
            if let Some((_, after_paren)) = snip.split_once(')') {
                return after_paren.lines().count() > 1;
            }
        }
        false
    }

    /// Checks if a block statement `{ ... }` contains more than one line of actual code.
    fn is_multiline_block_stmt(&self, stmt: &'ast ast::Stmt<'ast>) -> bool {
        if matches!(stmt.kind, ast::StmtKind::Block(_)) && self.sm.is_multiline(stmt.span) {
            if let Ok(snip) = self.sm.span_to_snippet(stmt.span) {
                let code_lines = snip.lines().filter(|line| {
                    let trimmed = line.trim();
                    // Ignore empty lines and lines with only '{' or '}'
                    !trimmed.is_empty() && trimmed != "{" && trimmed != "}"
                });
                return code_lines.count() > 1;
            }
        }
        false
    }

    /// Performs a size estimation to see if the if/else can fit on one line.
    fn can_stmts_fit_on_one_line(
        &mut self,
        cond: &'ast ast::Expr<'ast>,
        then: &'ast ast::Stmt<'ast>,
        els_opt: Option<&'ast &'ast mut ast::Stmt<'ast>>,
    ) -> bool {
        let cond_len = self.estimate_size(cond.span);

        // If the condition fits in one line, 6 chars: 'if (' + {cond} + ') ' + {then}
        // Otherwise chars: ') ' + {then}
        let then_margin =
            if (6 + cond_len as isize) < self.space_left() { 6 + cond_len } else { 2 };

        if !self.is_inline_stmt(then, then_margin) {
            return false;
        }

        // Always 6 chars for the else: 'else '
        els_opt.map_or(true, |els| self.is_inline_stmt(els, 6))
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
                    false,
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
                    false,
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
                        false,
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
                    false,
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
        self.print_comments(span.hi(), CommentConfig::default());
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
            false,
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
                    self.print_comment(cmnt, CommentConfig::default());
                }
                if !self.is_bol_or_only_ind() {
                    self.space();
                }
                self.ibox(0);
                print_fn(self);
                for cmnt in post_comments {
                    self.print_comment(cmnt, CommentConfig::default().mixed_prev_space());
                }
                self.end();
            }
            // Fallback for attributes not in the map (should never happen)
            None => {
                if !is_first && !self.is_bol_or_only_ind() {
                    self.space();
                }
                print_fn(self);
            }
        }
    }

    fn estimate_size(&self, span: Span) -> usize {
        if let Ok(snip) = self.sm.span_to_snippet(span) {
            let mut size = 0;
            for line in snip.lines() {
                size += line.trim().len();
            }
            return size;
        }

        span.hi().to_usize() - span.lo().to_usize()
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
    /// Doesn't print braces. Flags the offset that should be applied before opening the block box.
    /// Usefull when the caller needs to manually handle the braces.
    NoBraces(Option<isize>),
}

impl BlockFormat {
    pub(crate) fn with_braces(&self) -> bool {
        !matches!(self, Self::NoBraces(_))
    }
    pub(crate) fn breaks(&self) -> bool {
        matches!(self, Self::NoBraces(None))
    }

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

impl<'ast> AttributeKind<'ast> {
    fn is_visibility(&self) -> bool {
        matches!(self, Self::Visibility(_))
    }

    fn is_state_mutability(&self) -> bool {
        matches!(self, Self::StateMutability(_))
    }

    fn is_non_payable(&self) -> bool {
        matches!(self, Self::StateMutability(ast::StateMutability::NonPayable))
    }

    fn is_virtual(&self) -> bool {
        matches!(self, Self::Virtual)
    }

    fn is_override(&self) -> bool {
        matches!(self, Self::Override(_))
    }

    fn is_modifier(&self) -> bool {
        matches!(self, Self::Modifier(_))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttributeInfo<'ast> {
    pub(crate) kind: AttributeKind<'ast>,
    pub(crate) span: Span,
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
        if let Some(sm) = header.state_mutability {
            self.attributes
                .push(AttributeInfo { kind: AttributeKind::StateMutability(*sm), span: sm.span });
        }
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

fn stmt_needs_semi(stmt: &ast::StmtKind<'_>) -> bool {
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

fn item_needs_iso(item: &ast::ItemKind<'_>) -> bool {
    match item {
        ast::ItemKind::Contract(..) | ast::ItemKind::Struct(..) | ast::ItemKind::Enum(..) => true,

        ast::ItemKind::Pragma(..) |
        ast::ItemKind::Function(..) |
        ast::ItemKind::Import(..) |
        ast::ItemKind::Using(..) |
        ast::ItemKind::Variable(..) |
        ast::ItemKind::Udvt(..) |
        ast::ItemKind::Error(..) |
        ast::ItemKind::Event(..) => false,
    }
}

#[derive(Clone, Copy)]
pub enum Skip {
    First,
    All,
}

pub struct Decision {
    outcome: bool,
    is_cached: bool,
}

fn is_binary_expr(expr_kind: &ast::ExprKind<'_>) -> bool {
    matches!(expr_kind, ast::ExprKind::Binary(..))
}

fn has_complex_succesor(expr_kind: &ast::ExprKind<'_>, left: bool) -> bool {
    match expr_kind {
        ast::ExprKind::Binary(lhs, _, rhs) => {
            if left {
                has_complex_succesor(&lhs.kind, left)
            } else {
                has_complex_succesor(&rhs.kind, left)
            }
        }
        ast::ExprKind::Unary(_, expr) => has_complex_succesor(&expr.kind, left),
        ast::ExprKind::Lit(..) | ast::ExprKind::Ident(_) => false,
        _ => true,
    }
}

#[derive(Default, Clone, Copy)]
struct CommentConfig {
    skip_blanks: Option<Skip>,
    mixed_prev_space: bool,
    mixed_post_nbsp: bool,
    mixed_no_break: bool,
}

impl CommentConfig {
    fn skip_ws() -> Self {
        Self { skip_blanks: Some(Skip::All), ..Default::default() }
    }

    fn skip_first_ws() -> Self {
        Self { skip_blanks: Some(Skip::First), ..Default::default() }
    }

    fn mixed_no_break(mut self) -> Self {
        self.mixed_no_break = true;
        self
    }

    fn mixed_prev_space(mut self) -> Self {
        self.mixed_prev_space = true;
        self
    }

    fn mixed_post_nbsp(mut self) -> Self {
        self.mixed_post_nbsp = true;
        self
    }
}
