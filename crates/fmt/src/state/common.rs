use super::{CommentConfig, Separator, State};
use crate::pp::{BreakToken, Printer, SIZE_INFINITY};
use foundry_common::iter::IterDelimited;
use foundry_config::fmt as config;
use itertools::{Either, Itertools};
use solar::parse::{
    Cursor,
    ast::{self, Span},
    interface::BytePos,
};
use std::{borrow::Cow, fmt::Debug};

pub(crate) trait LitExt<'ast> {
    fn is_str_concatenation(&self) -> bool;
}

impl<'ast> LitExt<'ast> for ast::Lit<'ast> {
    /// Checks if a the input literal is a string literal with multiple parts.
    fn is_str_concatenation(&self) -> bool {
        if let ast::LitKind::Str(_, _, parts) = &self.kind { !parts.is_empty() } else { false }
    }
}

/// Language-specific pretty printing. Common for both: Solidity + Yul.
impl<'ast> State<'_, 'ast> {
    pub(super) fn print_lit(&mut self, lit: &'ast ast::Lit<'ast>) {
        let ast::Lit { span, symbol, ref kind } = *lit;
        if self.handle_span(span, false) {
            return;
        }

        match *kind {
            ast::LitKind::Str(kind, ..) => {
                self.s.ibox(0);
                for (pos, (span, symbol)) in lit.literals().delimited() {
                    if !self.handle_span(span, false) {
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
            if b && s.contains('_') { Cow::Owned(s.replace('_', "")) } else { Cow::Borrowed(s) }
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
        let is_dec = !["0x", "0b", "0o"].iter().any(|prefix| source.starts_with(prefix));

        let (val, exp) = if !is_dec {
            (source, "")
        } else {
            source.split_once(['e', 'E']).unwrap_or((source, ""))
        };
        let (val, fract) = val.split_once('.').unwrap_or((val, ""));

        let strip_underscores = !config.is_preserve();
        let mut val = &strip_underscores_if(strip_underscores, val)[..];
        let mut exp = &strip_underscores_if(strip_underscores, exp)[..];
        let mut fract = &strip_underscores_if(strip_underscores, fract)[..];

        // strip any padded 0's
        let mut exp_sign = "";
        if is_dec {
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

    /// `s` should be the *unescaped contents of the string literal*.
    pub(super) fn print_str_lit(&mut self, kind: ast::StrKind, quote_pos: BytePos, s: &str) {
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
        let s = solar::parse::interface::data_structures::fmt::from_fn(move |f| {
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

    pub(super) fn print_tuple_empty(&mut self, pos_lo: BytePos, pos_hi: BytePos) {
        if self.handle_span(Span::new(pos_lo, pos_hi), true) {
            return;
        }

        self.print_word("(");
        self.s.cbox(self.ind);
        if let Some(cmnt) = self.print_comments(pos_hi, CommentConfig::skip_ws().mixed_prev_space())
        {
            if cmnt.is_mixed() {
                self.s.offset(-self.ind);
            } else {
                self.break_offset_if_not_bol(0, -self.ind, false);
            }
        }
        self.end();
        self.print_word(")");
    }

    pub(super) fn print_tuple<'a, T, P, S>(
        &mut self,
        values: &'a [T],
        pos_lo: BytePos,
        pos_hi: BytePos,
        mut print: P,
        mut get_span: S,
        format: ListFormat,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span> + Copy,
    {
        if self.handle_span(Span::new(pos_lo, pos_hi), true) {
            return;
        }

        if values.is_empty() {
            self.print_tuple_empty(pos_lo, pos_hi);
            return;
        }

        // Format single-item inline lists directly without boxes
        if values.len() == 1 && matches!(format, ListFormat::Inline) {
            self.print_word("(");
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

            self.print_word(")");
            return;
        }

        // Otherwise, use commasep
        self.print_word("(");
        self.commasep(values, pos_lo, pos_hi, print, get_span, format);
        self.print_word(")");
    }

    pub(super) fn print_array<'a, T, P, S>(
        &mut self,
        values: &'a [T],
        span: Span,
        print: P,
        get_span: S,
    ) where
        P: FnMut(&mut Self, &'a T),
        S: FnMut(&T) -> Option<Span>,
    {
        if self.handle_span(span, false) {
            return;
        }

        self.print_word("[");
        self.commasep(
            values,
            span.lo(),
            span.hi(),
            print,
            get_span,
            ListFormat::Compact {
                break_single: false,
                cmnts_break: false,
                with_space: false,
                with_delimiters: true,
            },
        );
        self.print_word("]");
    }

    pub(super) fn commasep_opening_logic<T, S>(
        &mut self,
        values: &[T],
        mut get_span: S,
        format: ListFormat,
    ) -> bool
    where
        S: FnMut(&T) -> Option<Span>,
    {
        let pos = if let Some(span) = values.first().and_then(&mut get_span) {
            span.lo()
        } else {
            return false;
        };

        // Check for comments before the first item.
        if let Some((cmnt_span, cmnt_style)) =
            self.peek_comment_before(pos).map(|c| (c.span, c.style))
        {
            let cmnt_disabled = self.inline_config.is_disabled(cmnt_span);

            // Handle special formatting for disabled code with isolated comments.
            if self.cursor.enabled && cmnt_disabled && cmnt_style.is_isolated() {
                self.print_sep(Separator::Hardbreak);
                if !format.with_delimiters() {
                    self.s.offset(self.ind);
                }
            };

            // Apply spacing based on comment styles.
            if let Some(last_style) = self.print_comments(
                pos,
                if format.with_delimiters() {
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space()
                } else {
                    CommentConfig::skip_ws().no_breaks().mixed_prev_space().offset(self.ind)
                },
            ) {
                if cmnt_style.is_mixed() && last_style.is_mixed() {
                    if format.breaks_comments() {
                        self.hardbreak();
                    } else {
                        self.space();
                    }
                    if !format.with_delimiters() && !cmnt_disabled {
                        self.s.offset(self.ind);
                    }
                } else if !cmnt_style.is_mixed() && last_style.is_mixed() {
                    self.nbsp();
                } else if !last_style.is_mixed() && !format.with_delimiters() && !cmnt_disabled {
                    self.hardbreak();
                    self.s.offset(self.ind);
                }
            }

            if self.cursor.enabled {
                self.cursor.advance_to(pos, true);
            }

            return true;
        }

        if self.cursor.enabled {
            self.cursor.advance_to(pos, true);
        }

        if !values.is_empty() && !format.with_delimiters() {
            self.zerobreak();
            self.s.offset(self.ind);
            return true;
        }

        false
    }

    pub(super) fn commasep<'a, T, P, S>(
        &mut self,
        values: &'a [T],
        _pos_lo: BytePos,
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

        let is_single_without_cmnts = values.len() == 1
            && !format.break_single()
            && self.peek_comment_before(pos_hi).is_none();

        let skip_first_break = if format.with_delimiters() {
            self.s.cbox(self.ind);
            if is_single_without_cmnts {
                true
            } else {
                self.commasep_opening_logic(values, &mut get_span, format)
            }
        } else {
            let res = self.commasep_opening_logic(values, &mut get_span, format);
            self.s.cbox(self.ind);
            res
        };

        if let Some(sym) = format.prev_symbol() {
            self.word_space(sym);
        } else if is_single_without_cmnts && format.with_space() {
            self.nbsp();
        } else if !skip_first_break {
            format.add_break(true, values.len(), &mut self.s);
        }
        if format.is_compact() {
            self.s.cbox(0);
        }

        let mut skip_last_break = is_single_without_cmnts || !format.with_delimiters();
        for (i, value) in values.iter().enumerate() {
            let (is_last, span) = (i == values.len() - 1, get_span(value));
            if let Some(span) = span
                && self
                    .print_comments(span.lo(), CommentConfig::skip_ws().mixed_prev_space())
                    .is_some_and(|cmnt| cmnt.is_mixed())
                && format.breaks_comments()
            {
                self.hardbreak(); // trailing and isolated comments already hardbreak
            }

            print(self, value);
            if !is_last {
                self.print_word(",");
            }
            let next_span = if is_last { None } else { get_span(&values[i + 1]) };
            let next_pos = next_span.map(Span::lo).unwrap_or(pos_hi);
            if !is_last
                && format.breaks_comments()
                && self.peek_comment_before(next_pos).is_some_and(|cmnt| {
                    let disabled = self.inline_config.is_disabled(cmnt.span);
                    (cmnt.style.is_mixed() && !disabled) || (cmnt.style.is_isolated() && disabled)
                })
            {
                self.hardbreak(); // trailing and isolated comments already hardbreak
            }
            self.print_comments(
                next_pos,
                if !is_last || format.with_delimiters() {
                    CommentConfig::skip_ws().mixed_no_break().mixed_prev_space()
                } else {
                    CommentConfig::skip_ws().no_breaks().mixed_prev_space()
                },
            );

            if is_last && self.is_bol_or_only_ind() {
                // if a trailing comment is printed at the very end, we have to manually adjust
                // the offset to avoid having a double break.
                self.break_offset_if_not_bol(0, -self.ind, false);
                skip_last_break = true;
            }
            if let Some(next_span) = next_span
                && !self.is_bol_or_only_ind()
                && !self.inline_config.is_disabled(next_span)
            {
                format.add_break(false, values.len(), &mut self.s);
            }
        }

        if format.is_compact() {
            self.end();
        }
        if !skip_last_break {
            if let Some(sym) = format.post_symbol() {
                format.add_break(false, values.len(), &mut self.s);
                self.s.offset(-self.ind);
                self.word(sym);
            } else {
                format.add_break(true, values.len(), &mut self.s);
                self.s.offset(-self.ind);
            }
        } else if is_single_without_cmnts && format.with_space() {
            self.nbsp();
        } else if let Some(sym) = format.post_symbol() {
            self.nbsp();
            self.word(sym);
        }
        self.end();
        self.cursor.advance_to(pos_hi, true);

        if !format.with_delimiters() {
            self.zerobreak();
        }
    }

    pub(super) fn print_path(&mut self, path: &'ast ast::PathSlice, consistent_break: bool) {
        if consistent_break {
            self.s.cbox(self.ind);
        } else {
            self.s.ibox(self.ind);
        }
        for (pos, ident) in path.segments().iter().delimited() {
            self.print_ident(ident);
            if !pos.is_last {
                self.zerobreak();
                self.word(".");
            }
        }
        self.end();
    }

    pub(super) fn print_block_inner<T: Debug>(
        &mut self,
        block: &'ast [T],
        block_format: BlockFormat,
        mut print: impl FnMut(&mut Self, &'ast T),
        mut get_block_span: impl FnMut(&'ast T) -> Span,
        pos_hi: BytePos,
    ) {
        // Attempt to print in a single line
        if block_format.attempt_single_line() && block.len() == 1 {
            self.s.cbox(self.ind);
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.scan_break(BreakToken { pre_break: Some("{"), ..Default::default() });
            } else {
                self.word("{");
                self.space();
            }
            print(self, &block[0]);
            self.print_comments(get_block_span(&block[0]).hi(), CommentConfig::default());
            if matches!(block_format, BlockFormat::Compact(true)) {
                self.s.scan_break(BreakToken { post_break: Some("}"), ..Default::default() });
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
            let cmnt = self.peek_comment_before(pos_hi);
            if cmnt.is_none_or(|cmnt| cmnt.style.is_trailing()) {
                if self.config.bracket_spacing {
                    if block_format.with_braces() {
                        self.word("{ }");
                    } else {
                        self.nbsp();
                    }
                } else if block_format.with_braces() {
                    self.word("{}");
                }
                self.print_comments(pos_hi, CommentConfig::skip_ws());
            }
            // Other comments are printed inside the block
            else {
                if block_format.with_braces() {
                    self.word("{");
                }
                let offset =
                    if let BlockFormat::NoBraces(Some(off)) = block_format { off } else { 0 };
                self.print_comments(
                    pos_hi,
                    self.cmnt_config()
                        .offset(offset)
                        .mixed_no_break()
                        .mixed_prev_space()
                        .mixed_post_nbsp(),
                );
                self.print_comments(
                    pos_hi,
                    CommentConfig::default().mixed_no_break().mixed_prev_space().mixed_post_nbsp(),
                );

                if block_format.with_braces() {
                    self.word("}");
                }
            }
            return;
        }

        let first_stmt = get_block_span(&block[0]);
        let block_lo = first_stmt.lo();
        let is_block_lo_disabled =
            self.inline_config.is_disabled(Span::new(block_lo, block_lo + BytePos(1)));
        match block_format {
            BlockFormat::NoBraces(None) => {
                if !self.handle_span(self.cursor.span(block_lo), false) {
                    self.print_comments(block_lo, CommentConfig::default());
                }
                self.s.cbox(0);
            }
            BlockFormat::NoBraces(Some(offset)) => {
                let prev_cmnt =
                    self.peek_comment_before(block_lo).map(|cmnt| (cmnt.span, cmnt.style));
                if is_block_lo_disabled {
                    // We don't use `print_sep()` because we want to introduce the breakpoint
                    if prev_cmnt.is_none() && self.cursor.enabled {
                        Separator::Space.print(&mut self.s, &mut self.cursor);
                        self.s.offset(offset);
                        self.cursor.advance_to(block_lo, true);
                    } else if prev_cmnt.is_some_and(|(_, style)| style.is_isolated()) {
                        Separator::Space.print(&mut self.s, &mut self.cursor);
                        self.s.offset(offset);
                    }
                } else if !self.handle_span(self.cursor.span(block_lo), false) {
                    if let Some((span, style)) = prev_cmnt {
                        if !self.inline_config.is_disabled(span) || style.is_isolated() {
                            self.cursor.advance_to(span.lo(), true);
                            self.break_offset(SIZE_INFINITY as usize, offset);
                        }
                        if let Some(cmnt) =
                            self.print_comments(block_lo, CommentConfig::default().offset(offset))
                            && !cmnt.is_mixed()
                            && !cmnt.is_blank()
                        {
                            self.s.offset(offset);
                        }
                    } else {
                        self.zerobreak();
                        self.s.offset(offset);
                    }
                }
                self.s.cbox(self.ind);
            }
            _ => {
                self.print_word("{");
                self.s.cbox(self.ind);
                if !self.handle_span(self.cursor.span(block_lo), false)
                    && self
                        .print_comments(block_lo, CommentConfig::default())
                        .is_none_or(|cmnt| cmnt.is_mixed())
                {
                    self.hardbreak_if_nonempty();
                }
            }
        }

        for (i, stmt) in block.iter().enumerate() {
            let is_last = i == block.len() - 1;
            print(self, stmt);

            let is_disabled = self.inline_config.is_disabled(get_block_span(stmt));
            let (next_enabled, next_lo) = if !is_last {
                let next_span = get_block_span(&block[i + 1]);
                let next_lo = if self.peek_comment_before(next_span.lo()).is_none() {
                    Some(next_span.lo())
                } else {
                    None
                };

                (!self.inline_config.is_disabled(next_span), next_lo)
            } else {
                (false, None)
            };

            // when this stmt and the next one are enabled, break normally (except if last stmt)
            if !is_disabled
                && next_enabled
                && (!is_last
                    || self.peek_comment_before(pos_hi).is_some_and(|cmnt| cmnt.style.is_mixed()))
            {
                self.hardbreak_if_not_bol();
                continue;
            }
            // when this stmt is disabled and the next one is enabled, break if there is no
            // enabled preceding comment. Otherwise the breakpoint is handled by the comment.
            if is_disabled
                && next_enabled
                && let Some(next_lo) = next_lo
                && self
                    .peek_comment_before(next_lo)
                    .is_none_or(|cmnt| self.inline_config.is_disabled(cmnt.span))
            {
                self.hardbreak_if_not_bol()
            }
        }
        self.print_comments(
            pos_hi,
            CommentConfig::skip_trailing_ws().mixed_no_break().mixed_prev_space(),
        );
        if !block_format.breaks() {
            if !self.last_token_is_break() {
                self.hardbreak();
            }
            self.s.offset(-self.ind);
        }
        self.end();
        if block_format.with_braces() {
            self.print_word("}");
        }
    }
}

/// Formatting style for comma-separated lists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListFormat {
    /// Always breaks for multiple elements. If only one element, will print it isolated depending
    /// on the `break_single` flag.
    AlwaysBreak { break_single: bool, with_space: bool },
    /// Breaks all elements if any break.
    Consistent { break_single: bool, cmnts_break: bool, with_space: bool, with_delimiters: bool },
    /// Attempts to fit all elements in one line, before breaking consistently.
    Compact { break_single: bool, cmnts_break: bool, with_space: bool, with_delimiters: bool },
    /// If the list contains just one element, it will print unboxed (will not break).
    /// Otherwise, will break consistently.
    Inline,
    /// Since yul return values aren't wrapped in parenthesis, we need to manually handle the
    /// adjacent symbols to achieve the desired format.
    ///
    /// Behaves like `Self::Consistent`.
    Yul { sym_prev: Option<&'static str>, sym_post: Option<&'static str> },
}

impl ListFormat {
    pub(crate) fn break_single(&self) -> bool {
        match self {
            Self::AlwaysBreak { break_single, .. } => *break_single,
            Self::Consistent { break_single, .. } => *break_single,
            Self::Compact { break_single, .. } => *break_single,
            Self::Inline | Self::Yul { .. } => false,
        }
    }

    pub(crate) fn with_delimiters(&self) -> bool {
        match self {
            Self::AlwaysBreak { .. } | Self::Yul { .. } => true,
            Self::Consistent { with_delimiters, .. } => *with_delimiters,
            Self::Compact { with_delimiters, .. } => *with_delimiters,
            Self::Inline => false,
        }
    }

    pub(crate) fn breaks_comments(&self) -> bool {
        match self {
            Self::AlwaysBreak { .. } | Self::Yul { .. } => true,
            Self::Consistent { cmnts_break, .. } => *cmnts_break,
            Self::Compact { cmnts_break, .. } => *cmnts_break,
            Self::Inline => false,
        }
    }

    pub(crate) fn with_space(&self) -> bool {
        match self {
            Self::AlwaysBreak { with_space, .. } => *with_space,
            Self::Consistent { with_space, .. } => *with_space,
            Self::Compact { with_space, .. } => *with_space,
            Self::Inline | Self::Yul { .. } => false,
        }
    }

    pub(crate) fn prev_symbol(&self) -> Option<&'static str> {
        if let Self::Yul { sym_prev, .. } = self { *sym_prev } else { None }
    }

    pub(crate) fn post_symbol(&self) -> Option<&'static str> {
        if let Self::Yul { sym_post, .. } = self { *sym_post } else { None }
    }

    pub(crate) fn add_break(&self, soft: bool, elems: usize, p: &mut Printer) {
        if let Self::AlwaysBreak { break_single, .. } = self
            && (elems > 1 || (*break_single && elems == 1))
        {
            p.hardbreak();
        } else if soft && !self.with_space() {
            p.zerobreak();
        } else {
            p.space();
        }
    }

    pub(crate) fn is_compact(&self) -> bool {
        matches!(self, Self::Compact { .. })
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
    /// Useful when the caller needs to manually handle the braces.
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
