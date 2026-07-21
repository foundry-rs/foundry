use super::{
    BeginToken, BreakToken, Breaks, Doc, Document, FitId, GroupId, IndentStyle, LineSuffixHandle,
    Printer, SIZE_INFINITY, Token,
};
use std::borrow::Cow;

impl Printer {
    /// "raw box"
    pub fn rbox(&mut self, indent: isize, breaks: Breaks) {
        self.scan_begin(BeginToken {
            indent: IndentStyle::Block { offset: indent },
            breaks,
            group: None,
            probe: None,
            probe_size: None,
            probe_line_offset: None,
            force_break: false,
        });
    }

    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    fn group_box(&mut self, indent: isize, breaks: Breaks) -> GroupId {
        let group = GroupId(self.next_group);
        self.next_group += 1;
        self.scan_begin(BeginToken {
            indent: IndentStyle::Block { offset: indent },
            breaks,
            group: Some(group),
            probe: None,
            probe_size: None,
            probe_line_offset: None,
            force_break: false,
        });
        group
    }

    fn fit_box(&mut self, indent: isize, breaks: Breaks) -> (GroupId, FitId) {
        let group = GroupId(self.next_group);
        self.next_group += 1;
        let fit = FitId(self.next_choice);
        self.next_choice += 1;
        self.scan_begin(BeginToken {
            indent: IndentStyle::Block { offset: indent },
            breaks,
            group: Some(group),
            probe: Some(fit),
            probe_size: None,
            probe_line_offset: None,
            force_break: false,
        });
        (group, fit)
    }

    fn line_fit_box(
        &mut self,
        indent: isize,
        line_offset: isize,
        breaks: Breaks,
    ) -> (GroupId, FitId) {
        let group = GroupId(self.next_group);
        self.next_group += 1;
        let fit = FitId(self.next_choice);
        self.next_choice += 1;
        self.scan_begin(BeginToken {
            indent: IndentStyle::Block { offset: indent },
            breaks,
            group: Some(group),
            probe: Some(fit),
            probe_size: None,
            probe_line_offset: Some(line_offset),
            force_break: false,
        });
        (group, fit)
    }

    /// Inconsistent breaking box
    pub fn ibox(&mut self, indent: isize) {
        self.rbox(indent, Breaks::Inconsistent);
    }

    /// Consistent breaking box
    pub fn cbox(&mut self, indent: isize) {
        self.rbox(indent, Breaks::Consistent);
    }

    /// Begins an inconsistent box and returns an identifier for conditional content in it.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn ibox_with_id(&mut self, indent: isize) -> GroupId {
        self.group_box(indent, Breaks::Inconsistent)
    }

    /// Begins a consistent box and returns an identifier for conditional content in it.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn cbox_with_id(&mut self, indent: isize) -> GroupId {
        self.group_box(indent, Breaks::Consistent)
    }

    /// Begins an inconsistent box and returns identifiers for its layout and whether it fits.
    pub fn ibox_with_fit(&mut self, indent: isize) -> (GroupId, FitId) {
        self.fit_box(indent, Breaks::Inconsistent)
    }

    /// Begins an inconsistent box and probes whether it fits on a continuation line.
    pub fn ibox_with_line_fit(&mut self, indent: isize, line_offset: isize) -> (GroupId, FitId) {
        self.line_fit_box(indent, line_offset, Breaks::Inconsistent)
    }

    pub fn visual_align(&mut self) {
        self.scan_begin(BeginToken {
            indent: IndentStyle::Visual,
            breaks: Breaks::Consistent,
            group: None,
            probe: None,
            probe_size: None,
            probe_line_offset: None,
            force_break: false,
        });
    }

    pub fn break_offset(&mut self, n: usize, off: isize) {
        self.scan_break(BreakToken { offset: off, blank_space: n, ..BreakToken::default() });
    }

    pub fn end(&mut self) {
        self.scan_end();
    }

    pub fn eof(self) -> String {
        assert_eq!(self.line_suffix_depth, 0, "unclosed line suffix");
        self.render_document()
    }

    /// Begins capturing content that will be emitted immediately before the current line ends.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn begin_line_suffix(&mut self) -> LineSuffixHandle {
        let handle = LineSuffixHandle { depth: self.line_suffix_depth };
        self.line_suffix_depth += 1;
        self.document.nodes.push(Doc::LineSuffixStart);
        handle
    }

    /// Ends a line-suffix capture.
    #[track_caller]
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn end_line_suffix(&mut self, handle: LineSuffixHandle) {
        assert_eq!(
            handle.depth + 1,
            self.line_suffix_depth,
            "line suffixes must end in LIFO order"
        );
        self.line_suffix_depth -= 1;
        self.document.nodes.push(Doc::LineSuffixEnd);
    }

    /// Emits one of two documents according to the final layout of `group`.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn if_break(
        &mut self,
        group: GroupId,
        broken: impl FnOnce(&mut Self),
        flat: impl FnOnce(&mut Self),
    ) {
        let broken = self.capture(broken);
        let flat = self.capture(flat);
        self.document.nodes.push(Doc::IfBreak { group, broken, flat: flat.clone() });
        self.preview(&flat);
    }

    /// Forces all boxes enclosing this marker to use their broken layouts.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn break_parent(&mut self) {
        self.document.nodes.push(Doc::BreakParent);
    }

    /// Forces every box nested inside `group` to use its broken layout.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn break_children(&mut self, group: GroupId) {
        self.document.nodes.push(Doc::BreakChildren(group));
    }

    /// Prevents non-forced breaks nested inside `group`.
    pub fn flatten_children(&mut self, group: GroupId) {
        self.document.nodes.push(Doc::FlattenChildren(group));
    }

    /// Changes the indentation of `group` in the selected layout.
    pub fn set_indent(&mut self, group: GroupId, indent: isize) {
        self.document.nodes.push(Doc::SetIndent(group, indent));
    }

    /// Emits one of two documents according to whether the probed box fits.
    pub fn if_fits(
        &mut self,
        fit: FitId,
        fits: impl FnOnce(&mut Self),
        overflow: impl FnOnce(&mut Self),
    ) {
        let fits = self.capture(fits);
        let overflow = self.capture(overflow);
        self.document.nodes.push(Doc::IfFits { id: fit, fits: fits.clone(), overflow });
        self.preview(&fits);
    }

    /// Emits `preferred` when it fits, otherwise emits `fallback`.
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    pub fn choice(&mut self, preferred: impl FnOnce(&mut Self), fallback: impl FnOnce(&mut Self)) {
        let preferred = self.capture(preferred);
        let fallback = self.capture(fallback);
        let id = FitId(self.next_choice);
        self.next_choice += 1;
        self.document.nodes.push(Doc::Choice { id, preferred: preferred.clone(), fallback });
        self.preview(&preferred);
    }

    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    fn capture(&mut self, f: impl FnOnce(&mut Self)) -> Document {
        let mut child = Self::new(self.margin as usize, self.indent_config, self.tab_width);
        child.next_group = self.next_group;
        child.next_choice = self.next_choice;
        child.line_suffix_depth = self.line_suffix_depth;
        f(&mut child);
        assert_eq!(child.line_suffix_depth, self.line_suffix_depth, "unclosed line suffix");
        self.next_group = child.next_group;
        self.next_choice = child.next_choice;
        child.document
    }

    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    fn preview(&mut self, document: &Document) {
        let tokens = document.resolve(&Default::default(), &Default::default());
        let record_document = self.record_document;
        self.record_document = false;
        for token in tokens {
            self.scan_token(token);
        }
        self.record_document = record_document;
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

    pub fn last_token_is_neverbreak(&self) -> bool {
        if let Some(token) = self.last_token() {
            return token.is_neverbreak();
        }

        false
    }

    pub fn last_token_is_break(&self) -> bool {
        if let Some(token) = self.last_token() {
            return matches!(token, Token::Break(_));
        }
        false
    }

    pub fn last_token_is_space(&self) -> bool {
        if let Some(token) = self.last_token()
            && token.is_space()
        {
            return true;
        }

        self.out.ends_with(' ')
    }

    pub fn is_beginning_of_line(&self) -> bool {
        match self.last_token() {
            Some(last_token) => last_token.is_hardbreak(),
            None => self.out.is_empty() || self.out.ends_with('\n'),
        }
    }

    /// Attempts to identify whether the current position is:
    ///   1. the beginning of a line (empty)
    ///   2. a line with only indentation (just whitespaces)
    ///
    /// NOTE: this is still an educated guess, based on a heuristic.
    pub fn is_bol_or_only_ind(&self) -> bool {
        for i in self.buf.index_range().rev() {
            let token = &self.buf[i].token;
            if token.is_hardbreak() {
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
            Token::Break(BreakToken { pre_break: Some(s), .. }) => !s.trim().is_empty(),
            _ => false,
        }
    }

    pub(crate) fn hardbreak_tok_offset(offset: isize) -> Token {
        Token::Break(BreakToken {
            offset,
            blank_space: SIZE_INFINITY as usize,
            ..BreakToken::default()
        })
    }

    pub fn hardbreak_if_nonempty(&mut self) {
        self.scan_break(BreakToken {
            blank_space: SIZE_INFINITY as usize,
            if_nonempty: true,
            ..BreakToken::default()
        });
    }

    pub fn neverbreak(&mut self) {
        self.scan_break(BreakToken { never_break: true, ..BreakToken::default() });
    }
}

impl Token {
    pub(crate) const fn is_neverbreak(&self) -> bool {
        if let Self::Break(BreakToken { never_break, .. }) = *self {
            return never_break;
        }
        false
    }

    pub(crate) const fn is_hardbreak(&self) -> bool {
        if let Self::Break(BreakToken { blank_space, never_break, .. }) = *self {
            return blank_space == SIZE_INFINITY as usize && !never_break;
        }
        false
    }

    pub(crate) fn is_space(&self) -> bool {
        match self {
            Self::Break(BreakToken { offset, blank_space, .. }) => {
                *offset == 0 && *blank_space == 1
            }
            Self::String(s) => s.ends_with(' '),
            _ => false,
        }
    }
}
