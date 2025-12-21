#![allow(clippy::too_many_arguments)]
use crate::{
    FormatterConfig, InlineConfig,
    pp::{self, BreakToken, SIZE_INFINITY, Token},
    state::sol::BinOpGroup,
};
use foundry_common::{
    comments::{Comment, CommentStyle, Comments, estimate_line_width, line_with_tabs},
    iter::IterDelimited,
};
use foundry_config::fmt::{DocCommentStyle, IndentStyle};
use solar::parse::{
    ast::{self, Span},
    interface::{BytePos, SourceMap},
    token,
};
use std::{borrow::Cow, ops::Deref, sync::Arc};

mod common;
mod sol;
mod yul;

/// Specifies the nature of a complex call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallContextKind {
    /// A chained method call, `a().b()`.
    Chained,

    /// A nested function call, `a(b())`.
    Nested,
}

/// Formatting context for a call expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallContext {
    /// The kind call.
    pub(super) kind: CallContextKind,

    /// The size of the callee's head, excluding its arguments.
    pub(super) size: usize,
}

impl CallContext {
    pub(super) fn nested(size: usize) -> Self {
        Self { kind: CallContextKind::Nested, size }
    }

    pub(super) fn chained(size: usize) -> Self {
        Self { kind: CallContextKind::Chained, size }
    }

    pub(super) fn is_nested(&self) -> bool {
        matches!(self.kind, CallContextKind::Nested)
    }

    pub(super) fn is_chained(&self) -> bool {
        matches!(self.kind, CallContextKind::Chained)
    }
}

#[derive(Debug, Default)]
pub(super) struct CallStack {
    stack: Vec<CallContext>,
    precall_size: usize,
}

impl Deref for CallStack {
    type Target = [CallContext];
    fn deref(&self) -> &Self::Target {
        &self.stack
    }
}

impl CallStack {
    pub(crate) fn push(&mut self, call: CallContext) {
        self.stack.push(call);
    }

    pub(crate) fn pop(&mut self) -> Option<CallContext> {
        self.stack.pop()
    }

    pub(crate) fn add_precall(&mut self, size: usize) {
        self.precall_size += size;
    }

    pub(crate) fn reset_precall(&mut self) {
        self.precall_size = 0;
    }

    pub(crate) fn is_nested(&self) -> bool {
        self.last().is_some_and(|call| call.is_nested())
    }

    pub(crate) fn is_chain(&self) -> bool {
        self.last().is_some_and(|call| call.is_chained())
    }
}

pub(super) struct State<'sess, 'ast> {
    // CORE COMPONENTS
    pub(super) s: pp::Printer,
    ind: isize,

    sm: &'sess SourceMap,
    pub(super) comments: Comments,
    config: Arc<FormatterConfig>,
    inline_config: InlineConfig<()>,
    cursor: SourcePos,

    // FORMATTING CONTEXT:
    // Whether the source file uses CRLF (`\r\n`) line endings.
    has_crlf: bool,
    // The current contract being formatted, if inside a contract definition.
    contract: Option<&'ast ast::ItemContract<'ast>>,
    // Current block nesting depth (incremented for each `{...}` block entered).
    block_depth: usize,
    // Stack tracking nested and chained function calls.
    call_stack: CallStack,

    // Whether the current statement should be formatted as a single line, or not.
    single_line_stmt: Option<bool>,
    // The current binary expression chain context, if inside one.
    binary_expr: Option<BinOpGroup>,
    // Whether inside a `return` statement that contains a binary expression, or not.
    return_bin_expr: bool,
    // Whether inside a call with call options and at least one argument.
    call_with_opts_and_args: bool,
    // Whether to skip the index soft breaks because the callee fits inline.
    skip_index_break: bool,
    // Whether inside an `emit` or `revert` call with a qualified path, or not.
    emit_or_revert: bool,
    // Whether inside a variable initialization expression, or not.
    var_init: bool,
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

    pub(super) fn next_line(&mut self, is_at_crlf: bool) {
        self.pos += if is_at_crlf { 2 } else { 1 };
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
    fn print(&self, p: &mut pp::Printer, cursor: &mut SourcePos, is_at_crlf: bool) {
        match self {
            Self::Nbsp => p.nbsp(),
            Self::Space => p.space(),
            Self::Hardbreak => p.hardbreak(),
            Self::SpaceOrNbsp(breaks) => p.space_or_nbsp(*breaks),
        }

        cursor.next_line(is_at_crlf);
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
            has_crlf: false,
            contract: None,
            single_line_stmt: None,
            call_with_opts_and_args: false,
            skip_index_break: false,
            binary_expr: None,
            return_bin_expr: false,
            emit_or_revert: false,
            var_init: false,
            block_depth: 0,
            call_stack: CallStack::default(),
        }
    }

    /// Checks a span of the source for a carriage return (`\r`) to determine if the file
    /// uses CRLF line endings.
    ///
    /// If a `\r` is found, `self.has_crlf` is set to `true`. This is intended to be
    /// called once at the beginning of the formatting process for efficiency.
    fn check_crlf(&mut self, span: Span) {
        if let Ok(snip) = self.sm.span_to_snippet(span)
            && snip.contains('\r')
        {
            self.has_crlf = true;
        }
    }

    /// Checks if the cursor is currently positioned at the start of a CRLF sequence (`\r\n`).
    /// The check is only meaningful if `self.has_crlf` is true.
    fn is_at_crlf(&self) -> bool {
        self.has_crlf && self.char_at(self.cursor.pos) == Some('\r')
    }

    /// Computes the space left, bounded by the max space left.
    fn space_left(&self) -> usize {
        std::cmp::min(self.s.space_left(), self.max_space_left(0))
    }

    /// Computes the maximum space left given the context information available:
    /// `block_depth`, `tab_width`, and a user-defined unavailable size `prefix_len`.
    fn max_space_left(&self, prefix_len: usize) -> usize {
        self.config
            .line_length
            .saturating_sub(self.block_depth * self.config.tab_width + prefix_len)
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
    fn char_at(&self, pos: BytePos) -> Option<char> {
        let res = self.sm.lookup_byte_offset(pos);
        res.sf.src.get(res.pos.to_usize()..)?.chars().next()
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
        if self.handle_span(
            self.cursor.span(self.cursor.pos + if self.is_at_crlf() { 2 } else { 1 }),
            true,
        ) {
            return;
        }

        self.print_sep_unhandled(sep);
    }

    fn print_sep_unhandled(&mut self, sep: Separator) {
        let is_at_crlf = self.is_at_crlf();
        sep.print(&mut self.s, &mut self.cursor, is_at_crlf);
    }

    fn print_ident(&mut self, ident: &ast::Ident) {
        if self.handle_span(ident.span, true) {
            return;
        }

        self.print_comments(ident.span.lo(), CommentConfig::skip_ws());
        self.word(ident.to_string());
    }

    fn print_inside_parens<F>(&mut self, f: F)
    where
        F: FnOnce(&mut Self),
    {
        self.print_word("(");
        f(self);
        self.print_word(")");
    }

    fn estimate_size(&self, span: Span) -> usize {
        if let Ok(snip) = self.sm.span_to_snippet(span) {
            let (mut size, mut first, mut prev_needs_space) = (0, true, false);

            for line in snip.lines() {
                let line = line.trim();

                if prev_needs_space {
                    size += 1;
                } else if !first && let Some(char) = line.chars().next() {
                    // A line break or a space are required if this line:
                    // - starts with an operator.
                    // - starts with one of the ternary operators
                    // - starts with a bracket and fmt config forces bracket spacing.
                    match char {
                        '&' | '|' | '=' | '>' | '<' | '+' | '-' | '*' | '/' | '%' | '^' | '?'
                        | ':' => size += 1,
                        '}' | ')' | ']' if self.config.bracket_spacing => size += 1,
                        _ => (),
                    }
                }
                first = false;

                // trim spaces before and after mixed comments
                let mut search = line;
                loop {
                    if let Some((lhs, comment)) = search.split_once(r#"/*"#) {
                        size += lhs.trim_end().len() + 2;
                        search = comment;
                    } else if let Some((comment, rhs)) = search.split_once(r#"*/"#) {
                        size += comment.len() + 2;
                        search = rhs;
                    } else {
                        size += search.trim().len();
                        break;
                    }
                }

                // Next line requires a line break if this one:
                // - ends with a bracket and fmt config forces bracket spacing.
                // - ends with ',' a line break or a space are required.
                // - ends with ';' a line break is required.
                prev_needs_space = match line.chars().next_back() {
                    Some('[') | Some('(') | Some('{') => self.config.bracket_spacing,
                    Some(',') | Some(';') => true,
                    _ => false,
                };
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
    fn handle_comment(&mut self, cmnt: Comment, skip_break: bool) -> Option<Comment> {
        if self.cursor.enabled {
            if self.inline_config.is_disabled(cmnt.span) {
                if cmnt.style.is_trailing() && !self.last_token_is_space() {
                    self.nbsp();
                }
                self.print_span_cold(cmnt.span);
                if !skip_break && (cmnt.style.is_isolated() || cmnt.style.is_trailing()) {
                    self.print_sep(Separator::Hardbreak);
                }
                return None;
            }
        } else if self.print_span_if_disabled(cmnt.span) {
            if !skip_break && (cmnt.style.is_isolated() || cmnt.style.is_trailing()) {
                self.print_sep(Separator::Hardbreak);
            }
            return None;
        }
        Some(cmnt)
    }

    fn cmnt_config(&self) -> CommentConfig {
        CommentConfig { ..Default::default() }
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
            let mut cmnt = self.next_comment().unwrap();
            let style_cache = cmnt.style;

            // Merge consecutive line doc comments when converting to block style
            if self.config.docs_style == foundry_config::fmt::DocCommentStyle::Block
                && cmnt.is_doc
                && cmnt.kind == ast::CommentKind::Line
            {
                let mut ref_line = self.sm.lookup_char_pos(cmnt.span.hi()).line;
                while let Some(next_cmnt) = self.peek_comment() {
                    if !next_cmnt.is_doc
                        || next_cmnt.kind != ast::CommentKind::Line
                        || ref_line + 1 != self.sm.lookup_char_pos(next_cmnt.span.lo()).line
                    {
                        break;
                    }

                    let next_to_merge = self.next_comment().unwrap();
                    cmnt.lines.extend(next_to_merge.lines);
                    cmnt.span = cmnt.span.to(next_to_merge.span);
                    ref_line += 1;
                }
            }

            // Ensure breaks are never skipped when there are multiple comments
            if self.peek_comment_before(pos).is_some() {
                config.iso_no_break = false;
                config.trailing_no_break = false;
            }

            // Handle disabled comments
            let Some(cmnt) = self.handle_comment(
                cmnt,
                if style_cache.is_isolated() {
                    config.iso_no_break
                } else {
                    config.trailing_no_break
                },
            ) else {
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
                    config.mixed_no_break_prev = true;
                    config.mixed_no_break_post = true;
                    config.mixed_post_nbsp = cmnt.style.is_mixed();
                }

                // Ensure consecutive mixed comments don't have a double-space
                if last_style.is_some_and(|s| s.is_mixed()) {
                    config.mixed_no_break_prev = true;
                    config.mixed_no_break_post = true;
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

        fn post_break_prefix(prefix: &'static str, has_content: bool) -> &'static str {
            if !has_content {
                return prefix;
            }
            match prefix {
                "///" => "/// ",
                "//" => "// ",
                "/*" => "/* ",
                " *" => " * ",
                _ => prefix,
            }
        }

        self.ibox(0);
        self.word(prefix);

        let content = &line[prefix.len()..];
        let content = if is_doc {
            // Doc comments preserve leading whitespaces (right after the prefix) as nbps.
            let ws_len = content
                .char_indices()
                .take_while(|(_, c)| c.is_whitespace())
                .last()
                .map_or(0, |(idx, c)| idx + c.len_utf8());
            let (leading_ws, rest) = content.split_at(ws_len);
            if !leading_ws.is_empty() {
                self.word(leading_ws.to_owned());
            }
            rest
        } else {
            // Non-doc comments: replace first whitespace with nbsp, rest of content continues
            if let Some(first_char) = content.chars().next() {
                if first_char.is_whitespace() {
                    self.nbsp();
                    &content[first_char.len_utf8()..]
                } else {
                    content
                }
            } else {
                ""
            }
        };

        let post_break = post_break_prefix(prefix, !content.is_empty());

        // Process content character by character to preserve consecutive whitespaces
        let (mut chars, mut current_word) = (content.chars().peekable(), String::new());
        while let Some(ch) = chars.next() {
            if ch.is_whitespace() {
                // Print current word
                if !current_word.is_empty() {
                    self.word(std::mem::take(&mut current_word));
                }

                // Preserve multiple spaces while adding a single break
                let mut ws_count = 1;
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    ws_count += 1;
                    chars.next();
                }
                self.s.scan_break(BreakToken {
                    offset: break_offset,
                    blank_space: ws_count,
                    post_break: if post_break.starts_with("/*") { None } else { Some(post_break) },
                    ..Default::default()
                });
                continue;
            }

            current_word.push(ch);
        }

        // Print final word
        if !current_word.is_empty() {
            self.word(current_word);
        }

        self.end();
    }

    /// Merges consecutive line comments to avoid orphan words.
    fn merge_comment_lines(&self, lines: &[String], prefix: &str) -> Vec<String> {
        // Do not apply smart merging to block comments
        if lines.is_empty() || lines.len() < 2 || !prefix.starts_with("//") {
            return lines.to_vec();
        }

        let mut result = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let current_line = &lines[i];

            // Keep empty lines, and non-prefixed lines, untouched
            if current_line.trim().is_empty() || !current_line.starts_with(prefix) {
                result.push(current_line.clone());
                i += 1;
                continue;
            }

            if i + 1 < lines.len() {
                let next_line = &lines[i + 1];

                // Check if next line is has the same prefix and is not empty
                if next_line.starts_with(prefix) && !next_line.trim().is_empty() {
                    // Only merge if the current line doesn't fit within available width
                    if estimate_line_width(current_line, self.config.tab_width) > self.space_left()
                    {
                        // Merge the lines and let the wrapper handle breaking if needed
                        let merged_line = format!(
                            "{current_line} {next_content}",
                            next_content = &next_line[prefix.len()..].trim_start()
                        );
                        result.push(merged_line);

                        // Skip both lines since they are merged
                        i += 2;
                        continue;
                    }
                }
            }

            // No merge possible, keep the line as-is
            result.push(current_line.clone());
            i += 1;
        }

        result
    }

    fn print_comment(&mut self, mut cmnt: Comment, mut config: CommentConfig) {
        self.cursor.advance_to(cmnt.span.hi(), true);

        if cmnt.is_doc {
            cmnt = style_doc_comment(self.config.docs_style, cmnt);
        }

        match cmnt.style {
            CommentStyle::Mixed => {
                let Some(prefix) = cmnt.prefix() else { return };
                let never_break = self.last_token_is_neverbreak();
                if !self.is_bol_or_only_ind() {
                    match (never_break || config.mixed_no_break_prev, config.mixed_prev_space) {
                        (false, true) => config.space(&mut self.s),
                        (false, false) => config.zerobreak(&mut self.s),
                        (true, true) => self.nbsp(),
                        (true, false) => (),
                    };
                }
                if self.config.wrap_comments {
                    // Merge and wrap comments
                    let merged_lines = self.merge_comment_lines(&cmnt.lines, prefix);
                    for (pos, line) in merged_lines.into_iter().delimited() {
                        self.print_wrapped_line(&line, prefix, 0, cmnt.is_doc);
                        if !pos.is_last {
                            self.hardbreak();
                        }
                    }
                } else {
                    // No wrapping, print as-is
                    for (pos, line) in cmnt.lines.into_iter().delimited() {
                        self.word(line);
                        if !pos.is_last {
                            self.hardbreak();
                        }
                    }
                }
                if config.mixed_post_nbsp {
                    config.nbsp_or_space(self.config.wrap_comments, &mut self.s);
                    self.cursor.advance(1);
                } else if !config.mixed_no_break_post {
                    config.space(&mut self.s);
                    self.cursor.advance(1);
                }
            }
            CommentStyle::Isolated => {
                let Some(mut prefix) = cmnt.prefix() else { return };
                if !config.iso_no_break {
                    config.hardbreak_if_not_bol(self.is_bol_or_only_ind(), &mut self.s);
                }

                if self.config.wrap_comments {
                    // Merge and wrap comments
                    let merged_lines = self.merge_comment_lines(&cmnt.lines, prefix);
                    for (pos, line) in merged_lines.into_iter().delimited() {
                        let hb = |this: &mut Self| {
                            this.hardbreak();
                            if pos.is_last {
                                this.cursor.next_line(this.is_at_crlf());
                            }
                        };
                        if line.is_empty() {
                            hb(self);
                            continue;
                        }
                        if pos.is_first {
                            self.ibox(config.offset);
                            if cmnt.is_doc && matches!(prefix, "/**") {
                                self.word(prefix);
                                hb(self);
                                prefix = " * ";
                                continue;
                            }
                        }

                        self.print_wrapped_line(&line, prefix, 0, cmnt.is_doc);

                        if pos.is_last {
                            self.end();
                            if !config.iso_no_break {
                                hb(self);
                            }
                        } else {
                            hb(self);
                        }
                    }
                } else {
                    // No wrapping, print as-is
                    for (pos, line) in cmnt.lines.into_iter().delimited() {
                        let hb = |this: &mut Self| {
                            this.hardbreak();
                            if pos.is_last {
                                this.cursor.next_line(this.is_at_crlf());
                            }
                        };
                        if line.is_empty() {
                            hb(self);
                            continue;
                        }
                        if pos.is_first {
                            self.ibox(config.offset);
                            if cmnt.is_doc && matches!(prefix, "/**") {
                                self.word(prefix);
                                hb(self);
                                prefix = " * ";
                                continue;
                            }
                        }

                        self.word(line);

                        if pos.is_last {
                            self.end();
                            if !config.iso_no_break {
                                hb(self);
                            }
                        } else {
                            hb(self);
                        }
                    }
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
                    if cmnt.is_doc || matches!(cmnt.kind, ast::CommentKind::Line) {
                        config.offset = 0;
                    } else {
                        config.offset = self.ind;
                    }
                    for (lpos, line) in cmnt.lines.into_iter().delimited() {
                        if !line.is_empty() {
                            self.print_wrapped_line(&line, prefix, config.offset, cmnt.is_doc);
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
                    self.cursor.next_line(self.is_at_crlf());
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
                    self.cursor.next_line(self.is_at_crlf());
                }
                config.hardbreak(&mut self.s);
                self.cursor.next_line(self.is_at_crlf());
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

    fn has_comment_before_with<F>(&self, pos: BytePos, f: F) -> bool
    where
        F: FnMut(&Comment) -> bool,
    {
        self.comments.iter().take_while(|c| c.pos() < pos).any(f)
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
                if let Some(cmnt) = self.handle_comment(cmnt, config.trailing_no_break) {
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

    fn print_remaining_comments(&mut self, skip_leading_ws: bool) {
        // If there aren't any remaining comments, then we need to manually
        // make sure there is a line break at the end.
        if self.peek_comment().is_none() && !self.is_bol_or_only_ind() {
            self.hardbreak();
            return;
        }

        let mut is_leading = true;
        while let Some(cmnt) = self.next_comment() {
            if cmnt.style.is_blank() && skip_leading_ws && is_leading {
                continue;
            }

            is_leading = false;
            if let Some(cmnt) = self.handle_comment(cmnt, false) {
                self.print_comment(cmnt, CommentConfig::default());
            } else if self.peek_comment().is_none() && !self.is_bol_or_only_ind() {
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
}

#[derive(Default, Clone, Copy)]
pub(crate) struct CommentConfig {
    // Config: all
    skip_blanks: Option<Skip>,
    offset: isize,

    // Config: isolated comments
    iso_no_break: bool,
    // Config: trailing comments
    trailing_no_break: bool,
    // Config: mixed comments
    mixed_prev_space: bool,
    mixed_post_nbsp: bool,
    mixed_no_break_prev: bool,
    mixed_no_break_post: bool,
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

    pub(crate) fn no_breaks(mut self) -> Self {
        self.iso_no_break = true;
        self.trailing_no_break = true;
        self.mixed_no_break_prev = true;
        self.mixed_no_break_post = true;
        self
    }

    pub(crate) fn trailing_no_break(mut self) -> Self {
        self.trailing_no_break = true;
        self
    }

    pub(crate) fn mixed_no_break(mut self) -> Self {
        self.mixed_no_break_prev = true;
        self.mixed_no_break_post = true;
        self
    }

    pub(crate) fn mixed_no_break_post(mut self) -> Self {
        self.mixed_no_break_post = true;
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

/// Formats a doc comment with the requested style.
///
/// NOTE: assumes comments have already been normalized.
fn style_doc_comment(style: DocCommentStyle, mut cmnt: Comment) -> Comment {
    match style {
        DocCommentStyle::Line if cmnt.kind == ast::CommentKind::Block => {
            let mut new_lines = Vec::new();
            for (pos, line) in cmnt.lines.iter().delimited() {
                if pos.is_first || pos.is_last {
                    // Skip the opening '/**' and closing '*/' lines
                    continue;
                }

                // Convert ' * {content}' to '/// {content}'
                let trimmed = line.trim_start();
                if let Some(content) = trimmed.strip_prefix('*') {
                    new_lines.push(format!("///{content}"));
                } else if !trimmed.is_empty() {
                    new_lines.push(format!("/// {trimmed}"));
                }
            }

            cmnt.lines = new_lines;
            cmnt.kind = ast::CommentKind::Line;
            cmnt
        }
        DocCommentStyle::Block if cmnt.kind == ast::CommentKind::Line => {
            let mut new_lines = vec!["/**".to_string()];

            for line in &cmnt.lines {
                // Convert '/// {content}' to ' * {content}'
                new_lines.push(format!(" *{content}", content = &line[3..]))
            }

            new_lines.push(" */".to_string());
            cmnt.lines = new_lines;
            cmnt.kind = ast::CommentKind::Block;
            cmnt
        }
        // Otherwise, no conversion needed.
        _ => cmnt,
    }
}
