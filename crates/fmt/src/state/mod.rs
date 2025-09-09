#![allow(clippy::too_many_arguments)]
use crate::{
    FormatterConfig, InlineConfig,
    pp::{self, BreakToken, SIZE_INFINITY, Token},
};
use foundry_common::{
    comments::{Comment, CommentStyle, Comments, line_with_tabs},
    iter::IterDelimited,
};
use foundry_config::fmt::IndentStyle;
use solar::parse::{
    ast::{self, Span},
    interface::{BytePos, SourceMap},
    token,
};
use std::{borrow::Cow, sync::Arc};

mod common;
mod sol;
mod yul;

pub(super) struct State<'sess, 'ast> {
    pub(super) s: pp::Printer,
    ind: isize,

    sm: &'sess SourceMap,
    pub(super) comments: Comments,
    config: Arc<FormatterConfig>,
    inline_config: InlineConfig<()>,
    cursor: SourcePos,

    contract: Option<&'ast ast::ItemContract<'ast>>,
    single_line_stmt: Option<bool>,
    call_expr_named: bool,
    binary_expr: bool,
    member_expr: bool,
    var_init: bool,
    fn_body: bool,
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

struct SourcePos {
    pos: BytePos,
    enabled: bool,
}

impl SourcePos {
    pub(super) fn advance(&mut self, bytes: u32) {
        self.pos += BytePos(bytes);
    }

    pub(super) fn advance_to(&mut self, pos: BytePos, enabled: bool) {
        self.pos = std::cmp::max(pos, self.pos);
        self.enabled = enabled;
    }

    pub(super) fn span(&self, to: BytePos) -> Span {
        Span::new(self.pos, to)
    }
}

pub(super) enum Separator {
    Nbsp,
    Space,
    Hardbreak,
    SpaceOrNbsp(bool),
}

impl Separator {
    fn print(&self, p: &mut pp::Printer, cursor: &mut SourcePos) {
        match self {
            Self::Nbsp => p.nbsp(),
            Self::Space => p.space(),
            Self::Hardbreak => p.hardbreak(),
            Self::SpaceOrNbsp(breaks) => p.space_or_nbsp(*breaks),
        }
        cursor.advance(1);
    }
}

/// Generic methods
impl<'sess> State<'sess, '_> {
    pub(super) fn new(
        sm: &'sess SourceMap,
        config: Arc<FormatterConfig>,
        inline_config: InlineConfig<()>,
        comments: Comments,
    ) -> Self {
        Self {
            s: pp::Printer::new(
                config.line_length,
                if matches!(config.style, IndentStyle::Tab) {
                    Some(config.tab_width)
                } else {
                    None
                },
            ),
            ind: config.tab_width as isize,
            sm,
            comments,
            config,
            inline_config,
            cursor: SourcePos { pos: BytePos::from_u32(0), enabled: true },
            contract: None,
            single_line_stmt: None,
            call_expr_named: false,
            binary_expr: false,
            member_expr: false,
            var_init: false,
            fn_body: false,
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
        } else if off != 0
            && let Some(last_token) = self.last_token_still_buffered()
            && last_token.is_hardbreak()
        {
            // We do something pretty sketchy here: tuck the nonzero offset-adjustment we
            // were going to deposit along with the break into the previous hardbreak.
            self.replace_last_token_still_buffered(pp::Printer::hardbreak_tok_offset(off));
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

    fn print_span(&mut self, span: Span) {
        match self.sm.span_to_snippet(span) {
            Ok(s) => self.s.word(if matches!(self.config.style, IndentStyle::Tab) {
                snippet_with_tabs(s, self.config.tab_width)
            } else {
                s
            }),
            Err(e) => panic!("failed to print {span:?}: {e:#?}"),
        }
        // Drop comments that are included in the span.
        while let Some(cmnt) = self.peek_comment() {
            if cmnt.pos() >= span.hi() {
                break;
            }
            let _ = self.next_comment().unwrap();
        }
        // Update cursor
        self.cursor.advance_to(span.hi(), false);
    }

    /// Returns `true` if the span is disabled and has been printed as-is.
    #[must_use]
    fn handle_span(&mut self, span: Span, skip_prev_cmnts: bool) -> bool {
        if !skip_prev_cmnts {
            self.print_comments(span.lo(), CommentConfig::default());
        }
        self.print_span_if_disabled(span)
    }

    /// Returns `true` if the span is disabled and has been printed as-is.
    #[inline]
    #[must_use]
    fn print_span_if_disabled(&mut self, span: Span) -> bool {
        let cursor_span = self.cursor.span(span.hi());
        if self.inline_config.is_disabled(cursor_span) {
            self.print_span_cold(cursor_span);
            return true;
        }
        if self.inline_config.is_disabled(span) {
            self.print_span_cold(span);
            return true;
        }
        false
    }

    #[cold]
    fn print_span_cold(&mut self, span: Span) {
        self.print_span(span);
    }

    fn print_tokens(&mut self, tokens: &[token::Token]) {
        // Leave unchanged.
        let span = Span::join_first_last(tokens.iter().map(|t| t.span));
        self.print_span(span);
    }

    fn print_word(&mut self, w: impl Into<Cow<'static, str>>) {
        let cow = w.into();
        self.cursor.advance(cow.len() as u32);
        self.word(cow);
    }

    fn print_sep(&mut self, sep: Separator) {
        if self.handle_span(self.cursor.span(self.cursor.pos + BytePos(1)), true) {
            return;
        }

        sep.print(&mut self.s, &mut self.cursor);
    }

    fn print_ident(&mut self, ident: &ast::Ident) {
        if self.handle_span(ident.span, false) {
            return;
        }

        self.print_comments(ident.span.lo(), CommentConfig::skip_ws());
        self.word(ident.to_string());
    }

    fn estimate_size(&self, span: Span) -> usize {
        if let Ok(snip) = self.sm.span_to_snippet(span) {
            let mut size = 0;
            for line in snip.lines() {
                size += line.trim().len();
            }
            return size;
        }

        span.to_range().len()
    }

    fn same_source_line(&self, a: BytePos, b: BytePos) -> bool {
        self.sm.lookup_char_pos(a).line == self.sm.lookup_char_pos(b).line
    }
}

/// Comment-related methods.
impl<'sess> State<'sess, '_> {
    /// Returns `None` if the span is disabled and has been printed as-is.
    #[must_use]
    fn handle_comment(&mut self, cmnt: Comment) -> Option<Comment> {
        if self.cursor.enabled {
            if self.inline_config.is_disabled(cmnt.span) {
                if cmnt.style.is_trailing() && !self.last_token_is_space() {
                    self.nbsp();
                }
                self.print_span_cold(cmnt.span);
                if cmnt.style.is_isolated() || cmnt.style.is_trailing() {
                    self.print_sep(Separator::Hardbreak);
                }
                return None;
            }
        } else if self.print_span_if_disabled(cmnt.span) {
            if cmnt.style.is_isolated() || cmnt.style.is_trailing() {
                self.print_sep(Separator::Hardbreak);
            }
            return None;
        }
        Some(cmnt)
    }

    fn cmnt_config(&self) -> CommentConfig {
        CommentConfig { current_ind: self.ind, ..Default::default() }
    }

    fn cmnt_config_skip_ws(&self) -> CommentConfig {
        CommentConfig { current_ind: self.ind, skip_blanks: Some(Skip::All), ..Default::default() }
    }

    fn print_docs(&mut self, docs: &'_ ast::DocComments<'_>) {
        // Intetionally no-op. Handled with `self.comments`.
        let _ = docs;
    }

    /// Prints comments that are before the given position.
    ///
    /// Returns `Some` with the style of the last comment printed, or `None` if no comment was
    /// printed.
    fn print_comments(&mut self, pos: BytePos, mut config: CommentConfig) -> Option<CommentStyle> {
        let mut last_style: Option<CommentStyle> = None;
        let mut is_leading = true;
        let config_cache = config;
        let mut buffered_blank = None;
        while self.peek_comment().is_some_and(|c| c.pos() < pos) {
            let cmnt = self.next_comment().unwrap();
            let style_cache = cmnt.style;
            let Some(cmnt) = self.handle_comment(cmnt) else {
                last_style = Some(style_cache);
                continue;
            };

            if cmnt.style.is_blank() {
                match config.skip_blanks {
                    Some(Skip::All) => continue,
                    Some(Skip::Leading { resettable: true }) if is_leading => continue,
                    Some(Skip::Leading { resettable: false }) if last_style.is_none() => continue,
                    Some(Skip::Trailing) => {
                        buffered_blank = Some(cmnt);
                        continue;
                    }
                    _ => (),
                }
            // Never print blank lines after docs comments
            } else if !cmnt.is_doc {
                is_leading = false;
            }

            if let Some(blank) = buffered_blank.take() {
                self.print_comment(blank, config);
            }

            // Handle mixed with follow-up comment
            if cmnt.style.is_mixed() {
                if let Some(cmnt) = self.peek_comment_before(pos) {
                    config.mixed_no_break = true;
                    config.mixed_post_nbsp = cmnt.style.is_mixed();
                }

                // Ensure consecutive mixed comments don't have a double-space
                if last_style.is_some_and(|s| s.is_mixed()) {
                    config.mixed_no_break = true;
                    config.mixed_prev_space = false;
                }
            } else if config.offset != 0
                && cmnt.style.is_isolated()
                && last_style.is_some_and(|s| s.is_isolated())
            {
                self.offset(config.offset);
            }

            last_style = Some(cmnt.style);
            self.print_comment(cmnt, config);
            config = config_cache;
        }
        last_style
    }

    /// Prints a line, wrapping it if it starts with the given prefix.
    fn print_wrapped_line(
        &mut self,
        line: &str,
        prefix: &'static str,
        break_offset: isize,
        is_doc: bool,
    ) {
        if !line.starts_with(prefix) {
            self.word(line.to_owned());
            return;
        }

        let post_break_prefix = |prefix: &'static str, line_len: usize| -> &'static str {
            match prefix {
                "///" if line_len > 3 => "/// ",
                "//" if line_len > 2 => "// ",
                "/*" if line_len > 2 => "/* ",
                " *" if line_len > 2 => " * ",
                _ => prefix,
            }
        };

        self.ibox(0);
        let (prefix, content) = if is_doc {
            // Doc comments preserve leading whitespaces (right after the prefix).
            self.word(prefix);
            let content = &line[prefix.len()..];
            let (leading_ws, rest) =
                content.split_at(content.chars().take_while(|&c| c.is_whitespace()).count());
            if !leading_ws.is_empty() {
                self.word(leading_ws.to_owned());
            }
            let prefix = post_break_prefix(prefix, rest.len());
            (prefix, rest)
        } else {
            let content = line[prefix.len()..].trim();
            let prefix = post_break_prefix(prefix, content.len());
            self.word(prefix);
            (prefix, content)
        };

        // Split the rest of the content into words.
        let mut words = content.split_whitespace().peekable();
        while let Some(word) = words.next() {
            self.word(word.to_owned());
            if let Some(next_word) = words.peek() {
                if *next_word == "*/" {
                    self.nbsp();
                } else {
                    self.s.scan_break(BreakToken {
                        offset: break_offset,
                        blank_space: 1,
                        post_break: if matches!(prefix, "/* ") { None } else { Some(prefix) },
                        ..Default::default()
                    });
                }
            }
        }
        self.end();
    }

    fn print_comment(&mut self, mut cmnt: Comment, mut config: CommentConfig) {
        self.cursor.advance_to(cmnt.span.hi(), true);
        match cmnt.style {
            CommentStyle::Mixed => {
                let Some(prefix) = cmnt.prefix() else { return };
                let never_break = self.last_token_is_neverbreak();
                if !self.is_bol_or_only_ind() {
                    match (never_break || config.mixed_no_break, config.mixed_prev_space) {
                        (false, true) => config.space(&mut self.s),
                        (false, false) => config.zerobreak(&mut self.s),
                        (true, true) => self.nbsp(),
                        (true, false) => (),
                    };
                }
                for (pos, line) in cmnt.lines.into_iter().delimited() {
                    if self.config.wrap_comments {
                        self.print_wrapped_line(&line, prefix, 0, cmnt.is_doc);
                    } else {
                        self.word(line);
                    }
                    if !pos.is_last {
                        self.hardbreak();
                    }
                }
                if config.mixed_post_nbsp {
                    config.nbsp_or_space(self.config.wrap_comments, &mut self.s);
                    self.cursor.advance(1);
                } else if !config.mixed_no_break {
                    config.space(&mut self.s);
                    self.cursor.advance(1);
                }
            }
            CommentStyle::Isolated => {
                let Some(mut prefix) = cmnt.prefix() else { return };
                config.hardbreak_if_not_bol(self.is_bol_or_only_ind(), &mut self.s);
                for (pos, line) in cmnt.lines.into_iter().delimited() {
                    if line.is_empty() {
                        self.hardbreak();
                        continue;
                    }
                    if pos.is_first {
                        self.ibox(config.offset);
                        if self.config.wrap_comments && cmnt.is_doc && matches!(prefix, "/**") {
                            self.word(prefix);
                            self.hardbreak();
                            prefix = " * ";
                            continue;
                        }
                    }

                    if self.config.wrap_comments {
                        self.print_wrapped_line(&line, prefix, 0, cmnt.is_doc);
                    } else {
                        self.word(line);
                    }
                    if pos.is_last {
                        self.end();
                    }
                    self.print_sep(Separator::Hardbreak);
                }
            }
            CommentStyle::Trailing => {
                let Some(prefix) = cmnt.prefix() else { return };
                self.neverbreak();
                if !self.is_bol_or_only_ind() {
                    self.nbsp();
                }

                if !self.config.wrap_comments && cmnt.lines.len() == 1 {
                    self.word(cmnt.lines.pop().unwrap());
                } else if self.config.wrap_comments {
                    config.offset = self.ind;
                    for (lpos, line) in cmnt.lines.into_iter().delimited() {
                        if !line.is_empty() {
                            self.print_wrapped_line(
                                &line,
                                prefix,
                                if cmnt.is_doc { 0 } else { config.offset },
                                cmnt.is_doc,
                            );
                        }
                        if !lpos.is_last {
                            config.hardbreak(&mut self.s);
                        }
                    }
                } else {
                    self.visual_align();
                    for (pos, line) in cmnt.lines.into_iter().delimited() {
                        if !line.is_empty() {
                            self.word(line);
                            if !pos.is_last {
                                self.hardbreak();
                            }
                        }
                    }
                    self.end();
                }

                if !config.trailing_no_break {
                    self.print_sep(Separator::Hardbreak);
                }
            }

            CommentStyle::BlankLine => {
                // Pre-requisite: ensure that blank links are printed at the beginning of new line.
                if !self.last_token_is_break() && !self.is_bol_or_only_ind() {
                    config.hardbreak(&mut self.s);
                    self.cursor.advance(1);
                }

                // We need to do at least one, possibly two hardbreaks.
                let twice = match self.last_token() {
                    Some(Token::String(s)) => ";" == s,
                    Some(Token::Begin(_)) => true,
                    Some(Token::End) => true,
                    _ => false,
                };
                if twice {
                    config.hardbreak(&mut self.s);
                    self.cursor.advance(1);
                }
                config.hardbreak(&mut self.s);
                self.cursor.advance(1);
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

    fn has_comment_between(&self, start_pos: BytePos, end_pos: BytePos) -> bool {
        self.comments.iter().filter(|c| c.pos() > start_pos && c.pos() < end_pos).any(|_| true)
    }

    pub(crate) fn next_comment(&mut self) -> Option<Comment> {
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
        self.comments.peek_trailing(self.sm, span_pos, next_pos).map(|(cmnt, _)| cmnt)
    }

    fn print_trailing_comment_inner(
        &mut self,
        span_pos: BytePos,
        next_pos: Option<BytePos>,
        config: Option<CommentConfig>,
    ) -> bool {
        let mut printed = 0;
        if let Some((_, n)) = self.comments.peek_trailing(self.sm, span_pos, next_pos) {
            let config =
                config.unwrap_or(CommentConfig::skip_ws().mixed_no_break().mixed_prev_space());
            while printed <= n {
                let cmnt = self.comments.next().unwrap();
                if let Some(cmnt) = self.handle_comment(cmnt) {
                    self.print_comment(cmnt, config);
                };
                printed += 1;
            }
        }
        printed != 0
    }

    fn print_trailing_comment(&mut self, span_pos: BytePos, next_pos: Option<BytePos>) -> bool {
        self.print_trailing_comment_inner(span_pos, next_pos, None)
    }

    fn print_trailing_comment_no_break(&mut self, span_pos: BytePos, next_pos: Option<BytePos>) {
        self.print_trailing_comment_inner(
            span_pos,
            next_pos,
            Some(CommentConfig::skip_ws().trailing_no_break().mixed_no_break().mixed_prev_space()),
        );
    }

    fn print_remaining_comments(&mut self) {
        // If there aren't any remaining comments, then we need to manually
        // make sure there is a line break at the end.
        if self.peek_comment().is_none() && !self.is_bol_or_only_ind() {
            self.hardbreak();
        }

        while let Some(cmnt) = self.next_comment() {
            if let Some(cmnt) = self.handle_comment(cmnt) {
                self.print_comment(cmnt, CommentConfig::default());
            } else if self.peek_comment().is_none() {
                self.hardbreak();
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Skip {
    All,
    Leading { resettable: bool },
    Trailing,
    LeadingNoReset,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct CommentConfig {
    // Config: all
    skip_blanks: Option<Skip>,
    current_ind: isize,
    offset: isize,
    // Config: trailing comments
    trailing_no_break: bool,
    // Config: mixed comments
    mixed_prev_space: bool,
    mixed_post_nbsp: bool,
    mixed_no_break: bool,
}

impl CommentConfig {
    pub(crate) fn skip_ws() -> Self {
        Self { skip_blanks: Some(Skip::All), ..Default::default() }
    }

    pub(crate) fn skip_leading_ws(resettable: bool) -> Self {
        Self { skip_blanks: Some(Skip::Leading { resettable }), ..Default::default() }
    }

    pub(crate) fn skip_trailing_ws() -> Self {
        Self { skip_blanks: Some(Skip::Trailing), ..Default::default() }
    }

    pub(crate) fn offset(mut self, off: isize) -> Self {
        self.offset = off;
        self
    }

    pub(crate) fn trailing_no_break(mut self) -> Self {
        self.trailing_no_break = true;
        self
    }

    pub(crate) fn mixed_no_break(mut self) -> Self {
        self.mixed_no_break = true;
        self
    }

    pub(crate) fn mixed_prev_space(mut self) -> Self {
        self.mixed_prev_space = true;
        self
    }

    pub(crate) fn mixed_post_nbsp(mut self) -> Self {
        self.mixed_post_nbsp = true;
        self
    }

    pub(crate) fn hardbreak_if_not_bol(&self, is_bol: bool, p: &mut pp::Printer) {
        if self.offset != 0 && !is_bol {
            self.hardbreak(p);
        } else {
            p.hardbreak_if_not_bol();
        }
    }

    pub(crate) fn hardbreak(&self, p: &mut pp::Printer) {
        p.break_offset(SIZE_INFINITY as usize, self.offset);
    }

    pub(crate) fn space(&self, p: &mut pp::Printer) {
        p.break_offset(1, self.offset);
    }

    pub(crate) fn nbsp_or_space(&self, breaks: bool, p: &mut pp::Printer) {
        if breaks {
            self.space(p);
        } else {
            p.nbsp();
        }
    }

    pub(crate) fn zerobreak(&self, p: &mut pp::Printer) {
        p.break_offset(0, self.offset);
    }
}

fn snippet_with_tabs(s: String, tab_width: usize) -> String {
    // process leading breaks
    let trimmed = s.trim_start_matches('\n');
    let num_breaks = s.len() - trimmed.len();
    let mut formatted = std::iter::repeat_n('\n', num_breaks).collect::<String>();

    // process lines
    for (pos, line) in trimmed.lines().delimited() {
        line_with_tabs(&mut formatted, line, tab_width, None);
        if !pos.is_last {
            formatted.push('\n');
        }
    }

    formatted
}
