//! Adapted from [`rustc_ast_pretty`](https://github.com/rust-lang/rust/blob/07d3fd1d9b9c1f07475b96a9d168564bf528db68/compiler/rustc_ast_pretty/src/pp.rs)
//! and [`prettyplease`](https://github.com/dtolnay/prettyplease/blob/8eb8c14649aea32e810732bd4d64fe519e6b752a/src/algorithm.rs).

use crate::{DEBUG, DEBUG_INDENT};
use ring::RingBuffer;
use std::{
    borrow::Cow,
    cmp,
    collections::{HashMap, HashSet, VecDeque},
    iter,
};
use unicode_width::UnicodeWidthChar;

mod convenience;
mod helpers;
mod ring;

// Every line is allowed at least this much space, even if highly indented.
const MIN_SPACE: isize = 40;

/// How to break. Described in more detail in the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breaks {
    Consistent,
    Inconsistent,
}

/// Identifies a box whose final layout can be referenced by conditional documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(usize);

/// Identifies a line-suffix capture in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Used by formatter migrations to the retained document API.
pub struct LineSuffixHandle {
    depth: usize,
}

/// Identifies a retained fit probe used by conditional documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FitId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndentStyle {
    /// Vertically aligned under whatever column this block begins at.
    /// ```ignore
    /// fn demo(arg1: usize,
    ///         arg2: usize) {}
    /// ```
    Visual,
    /// Indented relative to the indentation level of the previous line.
    /// ```ignore
    /// fn demo(
    ///     arg1: usize,
    ///     arg2: usize,
    /// ) {}
    /// ```
    Block { offset: isize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BreakToken {
    pub(crate) offset: isize,
    pub(crate) blank_space: usize,
    pub(crate) pre_break: Option<&'static str>,
    pub(crate) post_break: Option<&'static str>,
    pub(crate) if_nonempty: bool,
    pub(crate) never_break: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContinuationBreak {
    flat_width: isize,
    offset: isize,
    post_break_width: isize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BeginToken {
    indent: IndentStyle,
    breaks: Breaks,
    group: Option<GroupId>,
    probe: Option<FitId>,
    probe_size: Option<isize>,
    force_break: bool,
    continuation: bool,
    continuation_break: Option<ContinuationBreak>,
    continuation_head: bool,
    continuation_head_size: Option<isize>,
    continuation_prefers_nested: bool,
    continuation_nested_slack: isize,
    transparent: bool,
    isolated: bool,
    isolated_slack: isize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    // In practice a string token contains either a `&'static str` or a
    // `String`. `Cow` is overkill for this because we never modify the data,
    // but it's more convenient than rolling our own more specialized type.
    String(Cow<'static, str>),
    Break(BreakToken),
    Begin(BeginToken),
    End,
    LineSuffix(Vec<Self>),
    BreakChildren(GroupId),
    FlattenChildren(GroupId),
    SetIndent(GroupId, isize),
}

/// Retained input to the pretty printer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Document {
    nodes: Vec<Doc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Used by formatter migrations to the retained document API.
enum Doc {
    Token(Token),
    IfBreak { group: GroupId, broken: Document, flat: Document },
    IfFits { id: FitId, fits: Document, overflow: Document },
    Choice { id: FitId, preferred: Document, fallback: Document },
    BreakParent,
    BreakChildren(GroupId),
    FlattenChildren(GroupId),
    SetIndent(GroupId, isize),
    LineSuffixStart,
    LineSuffixEnd,
}

#[derive(Copy, Clone, Debug)]
enum PrintFrame {
    Fits(Breaks),
    Broken(usize, Breaks),
}

pub(crate) const SIZE_INFINITY: isize = 0xffff;

#[derive(Debug)]
pub struct Printer {
    /// The authoritative token stream. The other fields form a live preview used by the
    /// imperative inspection API while the document is being built.
    document: Document,
    record_document: bool,
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    next_group: usize,
    #[allow(dead_code)] // Used by formatter migrations to the retained document API.
    next_choice: usize,
    line_suffix_depth: usize,
    out: String,
    /// Number of spaces left on line.
    space: isize,
    /// Ring-buffer of tokens and calculated sizes.
    buf: RingBuffer<BufEntry>,
    /// Running size of stream "...left".
    left_total: isize,
    /// Running size of stream "...right".
    right_total: isize,
    /// Pseudo-stack, really a ring too. Holds the
    /// primary-ring-buffers index of the Begin that started the
    /// current block, possibly with the most recent Break after that
    /// Begin (if there is any) on top of it. Stuff is flushed off the
    /// bottom as it becomes irrelevant due to the primary ring-buffer
    /// advancing.
    scan_stack: VecDeque<usize>,
    /// Stack of blocks-in-progress being flushed by print.
    print_stack: Vec<PrintFrame>,
    /// Named groups corresponding to `print_stack` frames.
    group_stack: Vec<Option<GroupId>>,
    flatten_stack: Vec<bool>,
    flatten_depth: usize,
    indent_restore_stack: Vec<Option<usize>>,
    group_states: HashMap<GroupId, bool>,
    choice_states: HashMap<FitId, bool>,
    /// Level of indentation of current line.
    indent: usize,
    /// Buffered indentation to avoid writing trailing whitespace.
    pending_indentation: usize,
    /// The token most recently popped from the left boundary of the
    /// ring-buffer for printing.
    last_printed: Option<Token>,
    pending_line_suffixes: Vec<(Vec<Token>, Vec<GroupId>)>,

    /// Target line width.
    margin: isize,
    /// Display width of a tab in source text.
    tab_width: usize,
    /// If `Some(tab_width)` the printer will use tabs for indentation.
    indent_config: Option<usize>,
}

#[derive(Debug)]
pub struct BufEntry {
    token: Token,
    size: isize,
    document_index: Option<usize>,
}

impl Printer {
    pub fn new(margin: usize, use_tab_with_size: Option<usize>, tab_width: usize) -> Self {
        Self::new_inner(margin, use_tab_with_size, tab_width, true)
    }

    fn new_inner(
        margin: usize,
        use_tab_with_size: Option<usize>,
        tab_width: usize,
        record_document: bool,
    ) -> Self {
        let margin = (margin as isize).clamp(MIN_SPACE, SIZE_INFINITY - 1);
        Self {
            document: Document::default(),
            record_document,
            next_group: 0,
            next_choice: 0,
            line_suffix_depth: 0,
            out: String::new(),
            space: margin,
            buf: RingBuffer::new(),
            left_total: 0,
            right_total: 0,
            scan_stack: VecDeque::new(),
            print_stack: Vec::new(),
            group_stack: Vec::new(),
            flatten_stack: Vec::new(),
            flatten_depth: 0,
            indent_restore_stack: Vec::new(),
            group_states: HashMap::new(),
            choice_states: HashMap::new(),
            indent: 0,
            pending_indentation: 0,
            last_printed: None,
            pending_line_suffixes: Vec::new(),

            margin,
            tab_width,
            indent_config: use_tab_with_size,
        }
    }

    fn record(&mut self, token: &Token) -> Option<usize> {
        if !self.record_document {
            return None;
        }
        let index = self.document.nodes.len();
        self.document.nodes.push(Doc::Token(token.clone()));
        Some(index)
    }

    fn render(mut self) -> (String, HashMap<GroupId, bool>, HashMap<FitId, bool>) {
        self.scan_eof();
        (self.out, self.group_states, self.choice_states)
    }

    fn render_document(&self) -> String {
        let mut broken_groups = HashSet::new();
        let mut fallback_choices = HashSet::new();

        loop {
            let mut tokens = self.document.resolve(&broken_groups, &fallback_choices);
            annotate_probe_sizes(&mut tokens, self.tab_width);
            annotate_continuation_layouts(&mut tokens, self.tab_width);
            let mut renderer =
                Self::new_inner(self.margin as usize, self.indent_config, self.tab_width, false);
            for token in tokens {
                renderer.scan_token(token);
            }
            let (out, groups, choices) = renderer.render();
            let old_group_count = broken_groups.len();
            let old_choice_count = fallback_choices.len();
            broken_groups
                .extend(groups.into_iter().filter_map(|(id, broken)| broken.then_some(id)));
            fallback_choices
                .extend(choices.into_iter().filter_map(|(id, broken)| broken.then_some(id)));
            if broken_groups.len() == old_group_count && fallback_choices.len() == old_choice_count
            {
                return out;
            }
        }
    }

    fn scan_token(&mut self, token: Token) {
        match token {
            Token::String(string) => self.scan_string(string),
            Token::Break(token) => self.scan_break(token),
            Token::Begin(token) => self.scan_begin(token),
            Token::End => self.scan_end(),
            Token::LineSuffix(tokens) => self.scan_line_suffix(tokens),
            Token::BreakChildren(_) | Token::FlattenChildren(_) | Token::SetIndent(..) => {
                unreachable!("unresolved child-layout marker")
            }
        }
    }

    pub(crate) fn last_token(&self) -> Option<&Token> {
        self.last_token_still_buffered().or(self.last_printed.as_ref())
    }

    pub(crate) fn last_token_still_buffered(&self) -> Option<&Token> {
        if self.buf.is_empty() {
            return None;
        }
        Some(&self.buf.last().token)
    }

    /// Be very careful with this!
    pub(crate) fn replace_last_token_still_buffered(&mut self, token: Token) {
        let entry = self.buf.last_mut();
        if let Some(index) = entry.document_index {
            self.document.nodes[index] = Doc::Token(token.clone());
        }
        entry.token = token;
    }

    /// WARNING: Be very careful with this!
    ///
    /// Searches backwards through the buffer to find and replace the last token
    /// that satisfies a predicate. This is a specialized and sensitive operation.
    ///
    /// This function's traversal logic is specifically designed to handle cases
    /// where formatting boxes have been closed (e.g., after a multi-line
    /// comment). It will automatically skip over any trailing `Token::End`
    /// tokens to find the substantive token before them.
    ///
    /// The search stops as soon as it encounters any token other than `End`
    /// (i.e., a `String`, `Break`, or `Begin`). The provided predicate is then
    /// called on that token. If the predicate returns `true`, the token is
    /// replaced.
    ///
    /// This function will only ever evaluate the predicate on **one** token.
    pub(crate) fn find_and_replace_last_token_still_buffered<F>(
        &mut self,
        new_token: Token,
        predicate: F,
    ) where
        F: FnOnce(&Token) -> bool,
    {
        for i in self.buf.index_range().rev() {
            let token = &self.buf[i].token;
            if matches!(token, Token::End) {
                // It's safe to skip the end of a box.
                continue;
            }

            // Apply the predicate and return after the first non-end token.
            if predicate(token) {
                if let Some(index) = self.buf[i].document_index {
                    self.document.nodes[index] = Doc::Token(new_token.clone());
                }
                self.buf[i].token = new_token;
            }
            break;
        }
    }

    fn scan_eof(&mut self) {
        if !self.scan_stack.is_empty() {
            self.check_stack(0);
            self.advance_left();
        }
        self.flush_line_suffixes();
    }

    fn scan_begin(&mut self, token: BeginToken) {
        let document_index = self.record(&Token::Begin(token));
        if self.scan_stack.is_empty() {
            self.left_total = 1;
            self.right_total = 1;
            self.buf.clear();
        }
        let right = self.buf.push(BufEntry {
            token: Token::Begin(token),
            size: -self.right_total,
            document_index,
        });
        self.scan_stack.push_back(right);
    }

    fn scan_end(&mut self) {
        let document_index = self.record(&Token::End);
        if self.scan_stack.is_empty() {
            self.print_end();
        } else {
            if !self.buf.is_empty()
                && let Token::Break(break_token) = self.buf.last().token
            {
                if self.buf.len() >= 2
                    && let Token::Begin(_) = self.buf.second_last().token
                {
                    self.buf.pop_last();
                    self.buf.pop_last();
                    self.scan_stack.pop_back();
                    self.scan_stack.pop_back();
                    self.right_total -= break_token.blank_space as isize;
                    return;
                }
                if break_token.if_nonempty {
                    self.buf.pop_last();
                    self.scan_stack.pop_back();
                    self.right_total -= break_token.blank_space as isize;
                }
            }
            let right = self.buf.push(BufEntry { token: Token::End, size: -1, document_index });
            self.scan_stack.push_back(right);
        }
    }

    pub(crate) fn scan_break(&mut self, token: BreakToken) {
        let document_index = self.record(&Token::Break(token));
        if self.scan_stack.is_empty() {
            self.left_total = 1;
            self.right_total = 1;
            self.buf.clear();
        } else {
            self.check_stack(0);
        }
        let right = self.buf.push(BufEntry {
            token: Token::Break(token),
            size: -self.right_total,
            document_index,
        });
        self.scan_stack.push_back(right);
        self.right_total += token.blank_space as isize;
    }

    fn scan_string(&mut self, string: Cow<'static, str>) {
        let document_index = self.record(&Token::String(string.clone()));
        if self.scan_stack.is_empty() {
            self.print_string(&string);
        } else {
            let len = display_width(&string, self.tab_width);
            self.buf.push(BufEntry { token: Token::String(string), size: len, document_index });
            self.right_total += len;
            self.check_stream();
        }
    }

    fn scan_line_suffix(&mut self, tokens: Vec<Token>) {
        let size = flat_size(&tokens, self.tab_width);
        if self.scan_stack.is_empty() {
            let groups = self.group_stack.iter().flatten().copied().collect();
            self.pending_line_suffixes.push((tokens, groups));
        } else {
            self.buf.push(BufEntry {
                token: Token::LineSuffix(tokens),
                size,
                document_index: None,
            });
            self.right_total = self.right_total.saturating_add(size).min(SIZE_INFINITY);
            self.check_stream();
        }
    }

    #[track_caller]
    pub(crate) fn offset(&mut self, offset: isize) {
        let entry = self.buf.last_mut();
        match &mut entry.token {
            Token::Break(token) => token.offset += offset,
            Token::Begin(_) => {}
            Token::String(_)
            | Token::End
            | Token::LineSuffix(_)
            | Token::BreakChildren(_)
            | Token::FlattenChildren(_)
            | Token::SetIndent(..) => {
                unreachable!()
            }
        }
        if let Some(index) = entry.document_index {
            self.document.nodes[index] = Doc::Token(entry.token.clone());
        }
    }

    pub(crate) fn ends_with(&self, ch: char) -> bool {
        for i in self.buf.index_range().rev() {
            if let Token::String(token) = &self.buf[i].token {
                return token.ends_with(ch);
            }
        }
        self.out.ends_with(ch)
    }

    fn check_stream(&mut self) {
        while self.right_total - self.left_total > self.space {
            if *self.scan_stack.front().unwrap() == self.buf.index_range().start {
                self.scan_stack.pop_front().unwrap();
                self.buf.first_mut().size = SIZE_INFINITY;
            }

            self.advance_left();

            if self.buf.is_empty() {
                break;
            }
        }
    }

    fn advance_left(&mut self) {
        while self.buf.first().size >= 0 {
            let left = self.buf.pop_first();

            match &left.token {
                Token::String(string) => {
                    self.left_total += left.size;
                    self.print_string(string);
                }
                Token::Break(token) => {
                    self.left_total += token.blank_space as isize;
                    self.print_break(*token, left.size);
                }
                Token::Begin(token) => self.print_begin(*token, left.size),
                Token::End => self.print_end(),
                Token::LineSuffix(tokens) => {
                    self.left_total = self.left_total.saturating_add(left.size).min(SIZE_INFINITY);
                    let groups = self.group_stack.iter().flatten().copied().collect();
                    self.pending_line_suffixes.push((tokens.clone(), groups));
                }
                Token::BreakChildren(_) | Token::FlattenChildren(_) | Token::SetIndent(..) => {
                    unreachable!("unresolved child-layout marker")
                }
            }

            self.last_printed = Some(left.token);

            if self.buf.is_empty() {
                break;
            }
        }
    }

    fn check_stack(&mut self, mut depth: usize) {
        while let Some(&index) = self.scan_stack.back() {
            let entry = &mut self.buf[index];
            match entry.token {
                Token::Begin(_) => {
                    if depth == 0 {
                        break;
                    }
                    self.scan_stack.pop_back().unwrap();
                    entry.size += self.right_total;
                    depth -= 1;
                }
                Token::End => {
                    // paper says + not =, but that makes no sense.
                    self.scan_stack.pop_back().unwrap();
                    entry.size = 1;
                    depth += 1;
                }
                _ => {
                    self.scan_stack.pop_back().unwrap();
                    entry.size += self.right_total;
                    if depth == 0 {
                        break;
                    }
                }
            }
        }
    }

    fn get_top(&self) -> PrintFrame {
        self.print_stack.last().copied().unwrap_or(PrintFrame::Broken(0, Breaks::Inconsistent))
    }

    fn print_begin(&mut self, token: BeginToken, size: isize) {
        if DEBUG {
            self.out.push(match token.breaks {
                Breaks::Consistent => '«',
                Breaks::Inconsistent => '‹',
            });
            if DEBUG_INDENT && let IndentStyle::Block { offset } = token.indent {
                self.out.extend(offset.to_string().chars().map(|ch| match ch {
                    '0'..='9' => ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉']
                        [(ch as u8 - b'0') as usize],
                    '-' => '₋',
                    _ => unreachable!(),
                }));
            }
        }

        if token.transparent {
            if let Some(group) = token.group {
                self.group_states.insert(group, false);
            }
            self.group_stack.push(token.group);
            self.flatten_stack.push(false);
            self.indent_restore_stack.push(Some(self.indent));
            self.print_stack.push(self.get_top());
            self.indent = match token.indent {
                IndentStyle::Block { offset } => {
                    usize::try_from(self.indent as isize + offset).unwrap()
                }
                IndentStyle::Visual => (self.margin - self.space) as usize,
            };
            return;
        }

        self.indent_restore_stack.push(None);

        let flat_size = token.probe_size.unwrap_or(size);
        let next_indent = match token.indent {
            IndentStyle::Block { offset } => self.indent as isize + offset,
            IndentStyle::Visual => self.margin - self.space,
        };
        let continuation_space = token.continuation_break.map_or(0, |leading| {
            self.margin
                .saturating_sub(next_indent.saturating_add(leading.offset))
                .max(0)
                .saturating_sub(leading.post_break_width)
        });
        let fits_continuation = token.continuation_break.is_some_and(|leading| {
            flat_size.saturating_sub(leading.flat_width) <= continuation_space
        });
        let needs_nested_slack = token.continuation_break.is_some_and(|leading| {
            flat_size
                .saturating_sub(leading.flat_width)
                .saturating_add(token.continuation_nested_slack)
                > continuation_space
        });
        let current_head_size = token.continuation_head_size.and_then(|head| {
            token.continuation_break.map(|leading| leading.flat_width.saturating_add(head))
        });
        let head_fits_current = (token.continuation_prefers_nested || needs_nested_slack)
            && current_head_size.is_some_and(|head| head <= self.space);
        let head_fits_continuation = token.continuation_head_size.is_some_and(|head| {
            current_head_size.is_some_and(|current| current > self.space)
                && head <= continuation_space
        });
        let layout_size = if token.continuation || token.isolated { flat_size } else { size };
        let layout_space = if token.isolated {
            self.space
                .min(self.margin.saturating_sub(self.indent as isize))
                .saturating_add(token.isolated_slack)
        } else {
            self.space
        };
        let inherited_flat = self.flatten_depth > 0;
        let isolates_children = token.isolated && !token.force_break && layout_size <= layout_space;
        let continuation_fits_own_line = token.continuation
            && !token.force_break
            && flat_size < SIZE_INFINITY
            && layout_size > layout_space
            && fits_continuation
            && !head_fits_current;
        let flattens_children = isolates_children || continuation_fits_own_line;
        let broken = token.force_break
            || !inherited_flat
                && layout_size > layout_space
                && (!token.continuation
                    || fits_continuation && !head_fits_current
                    || head_fits_continuation);
        if let Some(group) = token.group {
            self.group_states.insert(group, false);
        }
        self.group_stack.push(token.group);
        self.flatten_stack.push(flattens_children);
        self.flatten_depth += usize::from(flattens_children);
        if let Some(choice) = token.probe {
            self.choice_states
                .insert(choice, token.probe_size.is_some_and(|size| size > self.space));
        }
        if broken {
            self.print_stack.push(PrintFrame::Broken(self.indent, token.breaks));
            self.indent = match token.indent {
                IndentStyle::Block { offset } => {
                    usize::try_from(self.indent as isize + offset).unwrap()
                }
                IndentStyle::Visual => (self.margin - self.space) as usize,
            };
        } else {
            self.print_stack.push(PrintFrame::Fits(token.breaks));
        }
    }

    fn print_end(&mut self) {
        let restore_indent = self.indent_restore_stack.pop().expect("unmatched end token");
        if self.flatten_stack.pop().expect("unmatched end token") {
            self.flatten_depth -= 1;
        }
        self.group_stack.pop().expect("unmatched end token");
        let breaks = match self.print_stack.pop().unwrap() {
            PrintFrame::Broken(indent, breaks) => {
                self.indent = indent;
                breaks
            }
            PrintFrame::Fits(breaks) => breaks,
        };
        if let Some(indent) = restore_indent {
            self.indent = indent;
        }
        if DEBUG {
            self.out.push(match breaks {
                Breaks::Consistent => '»',
                Breaks::Inconsistent => '›',
            });
        }
    }

    fn print_break(&mut self, token: BreakToken, size: isize) {
        let fits = token.never_break
            || token.blank_space < SIZE_INFINITY as usize
                && match self.get_top() {
                    PrintFrame::Fits(..) => true,
                    PrintFrame::Broken(.., Breaks::Consistent) => false,
                    PrintFrame::Broken(.., Breaks::Inconsistent) => size <= self.space,
                };
        if fits {
            self.pending_indentation += token.blank_space;
            self.space -= token.blank_space as isize;
            if DEBUG {
                self.out.push('·');
            }
        } else {
            for group in self.group_stack.iter().flatten() {
                self.group_states.insert(*group, true);
            }
            if let Some(pre_break) = token.pre_break {
                self.print_indent();
                self.out.push_str(pre_break);
            }
            self.flush_line_suffixes();
            if DEBUG {
                self.out.push('·');
            }
            self.out.push('\n');
            let indent = self.indent as isize + token.offset;
            self.pending_indentation = usize::try_from(indent).expect("negative indentation");
            self.space = cmp::max(self.margin - indent, MIN_SPACE);
            if let Some(post_break) = token.post_break {
                self.print_indent();
                self.out.push_str(post_break);
                self.space -= display_width(post_break, self.tab_width);
            }
        }
    }

    fn print_string(&mut self, string: &str) {
        self.print_indent();
        self.out.push_str(string);
        self.space -= display_width(string, self.tab_width);
    }

    fn flush_line_suffixes(&mut self) {
        for (tokens, enclosing_groups) in std::mem::take(&mut self.pending_line_suffixes) {
            let mut renderer =
                Self::new_inner(self.margin as usize, self.indent_config, self.tab_width, false);
            renderer.space = self.space;
            renderer.indent = self.indent;
            renderer.pending_indentation = self.pending_indentation;
            for token in tokens {
                renderer.scan_token(token);
            }
            renderer.scan_eof();

            self.out.push_str(&renderer.out);
            self.space = renderer.space;
            self.pending_indentation = renderer.pending_indentation;
            self.group_states.extend(renderer.group_states);
            self.choice_states.extend(renderer.choice_states);
            if renderer.out.contains('\n') {
                for group in enclosing_groups {
                    self.group_states.insert(group, true);
                }
            }
        }
    }

    fn print_indent(&mut self) {
        self.out.reserve(self.pending_indentation);
        if let Some(tab_width) = self.indent_config {
            let num_tabs = self.pending_indentation / tab_width;
            self.out.extend(iter::repeat_n('\t', num_tabs));

            let remainder = self.pending_indentation % tab_width;
            self.out.extend(iter::repeat_n(' ', remainder));
        } else {
            self.out.extend(iter::repeat_n(' ', self.pending_indentation));
        }
        self.pending_indentation = 0;
    }
}

impl Document {
    fn resolve(
        &self,
        broken_groups: &HashSet<GroupId>,
        fallback_choices: &HashSet<FitId>,
    ) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut groups = Vec::new();
        self.resolve_into(broken_groups, fallback_choices, &mut groups, &mut tokens);
        force_break_children(&mut tokens);
        flatten_children(&mut tokens);
        set_group_indents(&mut tokens);
        tokens
    }

    fn resolve_into(
        &self,
        broken_groups: &HashSet<GroupId>,
        fallback_choices: &HashSet<FitId>,
        groups: &mut Vec<usize>,
        tokens: &mut Vec<Token>,
    ) {
        let mut index = 0;
        while index < self.nodes.len() {
            let node = &self.nodes[index];
            match node {
                Doc::Token(Token::Begin(token)) => {
                    groups.push(tokens.len());
                    tokens.push(Token::Begin(*token));
                }
                Doc::Token(Token::End) => {
                    groups.pop();
                    tokens.push(Token::End);
                }
                Doc::Token(token) => tokens.push(token.clone()),
                Doc::IfBreak { group, broken, flat } => {
                    let branch = if broken_groups.contains(group) { broken } else { flat };
                    branch.resolve_into(broken_groups, fallback_choices, groups, tokens);
                }
                Doc::IfFits { id, fits, overflow } => {
                    let branch = if fallback_choices.contains(id) { overflow } else { fits };
                    branch.resolve_into(broken_groups, fallback_choices, groups, tokens);
                }
                Doc::Choice { id, preferred, fallback } => {
                    if fallback_choices.contains(id) {
                        fallback.resolve_into(broken_groups, fallback_choices, groups, tokens);
                    } else {
                        let begin = BeginToken {
                            indent: IndentStyle::Block { offset: 0 },
                            breaks: Breaks::Inconsistent,
                            group: None,
                            probe: Some(*id),
                            probe_size: None,
                            force_break: false,
                            continuation: false,
                            continuation_break: None,
                            continuation_head: false,
                            continuation_head_size: None,
                            continuation_prefers_nested: false,
                            continuation_nested_slack: 0,
                            transparent: false,
                            isolated: false,
                            isolated_slack: 0,
                        };
                        groups.push(tokens.len());
                        tokens.push(Token::Begin(begin));
                        preferred.resolve_into(broken_groups, fallback_choices, groups, tokens);
                        groups.pop();
                        tokens.push(Token::End);
                    }
                }
                Doc::BreakParent => {
                    for &index in groups.iter() {
                        let Token::Begin(begin) = &mut tokens[index] else { unreachable!() };
                        begin.force_break = true;
                    }
                }
                Doc::BreakChildren(group) => tokens.push(Token::BreakChildren(*group)),
                Doc::FlattenChildren(group) => tokens.push(Token::FlattenChildren(*group)),
                Doc::SetIndent(group, indent) => tokens.push(Token::SetIndent(*group, *indent)),
                Doc::LineSuffixStart => {
                    let start = index + 1;
                    let mut depth = 1;
                    index = start;
                    while depth > 0 {
                        match self.nodes.get(index) {
                            Some(Doc::LineSuffixStart) => depth += 1,
                            Some(Doc::LineSuffixEnd) => depth -= 1,
                            Some(_) => {}
                            None => panic!("unclosed line suffix"),
                        }
                        index += 1;
                    }
                    let end = index - 1;
                    let document = Self { nodes: self.nodes[start..end].to_vec() };
                    let suffix_start = tokens.len();
                    document.resolve_into(broken_groups, fallback_choices, groups, tokens);
                    let suffix_tokens = tokens.drain(suffix_start..).collect();
                    tokens.push(Token::LineSuffix(suffix_tokens));
                    continue;
                }
                Doc::LineSuffixEnd => panic!("line suffix ended without a matching start"),
            }
            index += 1;
        }
    }
}

fn flat_size(tokens: &[Token], tab_width: usize) -> isize {
    tokens.iter().fold(0, |size, token| {
        let token_size = match token {
            Token::String(string) => display_width(string, tab_width),
            Token::Break(token) => token.blank_space as isize,
            Token::Begin(_) | Token::End => 0,
            Token::LineSuffix(tokens) => flat_size(tokens, tab_width),
            Token::BreakChildren(_) | Token::FlattenChildren(_) | Token::SetIndent(..) => 0,
        };
        size.saturating_add(token_size).min(SIZE_INFINITY)
    })
}

fn display_width(string: &str, tab_width: usize) -> isize {
    string
        .chars()
        .map(|ch| if ch == '\t' { tab_width } else { ch.width().unwrap_or(0) })
        .sum::<usize>()
        .min(SIZE_INFINITY as usize) as isize
}

fn annotate_probe_sizes(tokens: &mut [Token], tab_width: usize) {
    let mut stack = Vec::new();
    let mut probes = Vec::new();
    let mut total = 0usize;
    for index in 0..tokens.len() {
        match &mut tokens[index] {
            Token::Begin(begin) => stack.push((
                index,
                (begin.probe.is_some() || begin.continuation || begin.isolated).then_some(total),
            )),
            Token::End => {
                let (start, probe_start) = stack.pop().expect("unmatched end token");
                if let Some(probe_start) = probe_start {
                    let size = total.saturating_sub(probe_start).min(SIZE_INFINITY as usize);
                    probes.push((start, size as isize));
                }
            }
            Token::String(string) => {
                total = total.saturating_add(display_width(string, tab_width) as usize)
            }
            Token::Break(token) => total = total.saturating_add(token.blank_space),
            Token::LineSuffix(tokens) => {
                annotate_probe_sizes(tokens, tab_width);
                total = total.saturating_add(flat_size(tokens, tab_width) as usize)
            }
            Token::BreakChildren(_) | Token::FlattenChildren(_) | Token::SetIndent(..) => {}
        }
    }
    for (index, size) in probes {
        let Token::Begin(begin) = &mut tokens[index] else { unreachable!() };
        begin.probe_size = Some(size);
    }
}

fn annotate_continuation_layouts(tokens: &mut [Token], tab_width: usize) {
    for token in &mut *tokens {
        if let Token::LineSuffix(tokens) = token {
            annotate_continuation_layouts(tokens, tab_width);
        }
    }

    let starts = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| match token {
            Token::Begin(begin) if begin.continuation => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();

    for start in starts {
        let Token::Begin(start_token) = &tokens[start] else { unreachable!() };
        let prefers_nested =
            start_token.continuation_prefers_nested || start_token.continuation_nested_slack > 0;
        let mut depth = 0usize;
        let mut leading = None;
        let mut has_content = false;
        let mut end = tokens.len();
        for (index, token) in tokens.iter().enumerate().skip(start + 1) {
            match token {
                Token::Begin(_) => {
                    has_content = true;
                    depth += 1;
                }
                Token::End if depth == 0 => {
                    end = index;
                    break;
                }
                Token::End => depth -= 1,
                Token::Break(token) if depth == 0 && !has_content => {
                    leading = Some((index, *token));
                    break;
                }
                Token::String(string) if !string.is_empty() => has_content = true,
                Token::LineSuffix(tokens) if !tokens.is_empty() => has_content = true,
                _ => {}
            }
        }

        let Some((leading_index, leading)) = leading else { continue };

        if end == tokens.len() {
            depth = 0;
            for (index, token) in tokens.iter().enumerate().skip(leading_index + 1) {
                match token {
                    Token::Begin(_) => depth += 1,
                    Token::End if depth == 0 => {
                        end = index;
                        break;
                    }
                    Token::End => depth -= 1,
                    _ => {}
                }
            }
        }

        let mut min_break_depth = usize::MAX;
        let mut depth = 0usize;
        for token in &tokens[leading_index + 1..end] {
            match token {
                Token::Begin(_) => depth += 1,
                Token::End => depth -= 1,
                Token::Break(_) => min_break_depth = min_break_depth.min(depth),
                _ => {}
            }
        }

        depth = 0;
        let mut size = 0isize;
        for token in &tokens[leading_index + 1..end] {
            match token {
                Token::Begin(_) => depth += 1,
                Token::End => depth -= 1,
                Token::Break(_) if prefers_nested || depth == min_break_depth => break,
                Token::String(string) => {
                    size = size.saturating_add(display_width(string, tab_width));
                }
                Token::Break(token) => {
                    size = size.saturating_add(token.blank_space as isize);
                }
                Token::LineSuffix(tokens) => {
                    size = size.saturating_add(flat_size(tokens, tab_width));
                }
                Token::BreakChildren(_) | Token::FlattenChildren(_) | Token::SetIndent(..) => {}
            }
        }

        let Token::Begin(begin) = &mut tokens[start] else { unreachable!() };
        begin.continuation_break = Some(ContinuationBreak {
            flat_width: leading.blank_space as isize,
            offset: leading.offset,
            post_break_width: leading
                .post_break
                .map_or(0, |prefix| display_width(prefix, tab_width)),
        });
        if begin.continuation_head {
            begin.continuation_head_size = Some(size.min(SIZE_INFINITY));
        }
    }
}

fn force_break_children(tokens: &mut Vec<Token>) {
    fn collect(tokens: &[Token], targets: &mut HashSet<GroupId>) {
        for token in tokens {
            match token {
                Token::BreakChildren(group) => {
                    targets.insert(*group);
                }
                Token::LineSuffix(tokens) => collect(tokens, targets),
                _ => {}
            }
        }
    }

    fn apply(tokens: &mut Vec<Token>, targets: &HashSet<GroupId>, active: &mut usize) {
        let mut stack = Vec::new();
        for token in tokens.iter_mut() {
            match token {
                Token::Begin(begin) => {
                    if *active > 0 {
                        begin.force_break = true;
                    }
                    let targeted = begin.group.is_some_and(|group| targets.contains(&group));
                    stack.push(targeted);
                    *active += usize::from(targeted);
                }
                Token::End if stack.pop().expect("unmatched end token") => *active -= 1,
                Token::End => {}
                Token::LineSuffix(tokens) => apply(tokens, targets, active),
                _ => {}
            }
        }
        tokens.retain(|token| !matches!(token, Token::BreakChildren(_)));
    }

    let mut targets = HashSet::new();
    collect(tokens, &mut targets);
    if targets.is_empty() {
        return;
    }
    let mut active = 0usize;
    apply(tokens, &targets, &mut active);
}

fn flatten_children(tokens: &mut Vec<Token>) {
    fn collect(tokens: &[Token], targets: &mut HashSet<GroupId>) {
        for token in tokens {
            match token {
                Token::FlattenChildren(group) => {
                    targets.insert(*group);
                }
                Token::LineSuffix(tokens) => collect(tokens, targets),
                _ => {}
            }
        }
    }

    fn apply(
        tokens: &mut Vec<Token>,
        targets: &HashSet<GroupId>,
        active: &mut usize,
        forced: &mut usize,
    ) {
        let mut stack = Vec::new();
        for token in tokens.iter_mut() {
            match token {
                Token::Begin(begin) => {
                    let targeted = begin.group.is_some_and(|group| targets.contains(&group));
                    let force_break = (*active > 0 || targeted) && begin.force_break;
                    stack.push((targeted, force_break));
                    *active += usize::from(targeted);
                    *forced += usize::from(force_break);
                }
                Token::End => {
                    let (targeted, force_break) = stack.pop().expect("unmatched end token");
                    *active -= usize::from(targeted);
                    *forced -= usize::from(force_break);
                }
                Token::Break(token)
                    if *active > 0
                        && *forced == 0
                        && token.blank_space < SIZE_INFINITY as usize =>
                {
                    token.never_break = true;
                }
                Token::LineSuffix(tokens) => apply(tokens, targets, active, forced),
                _ => {}
            }
        }
        tokens.retain(|token| !matches!(token, Token::FlattenChildren(_)));
    }

    let mut targets = HashSet::new();
    collect(tokens, &mut targets);
    if targets.is_empty() {
        return;
    }
    let mut active = 0usize;
    let mut forced = 0usize;
    apply(tokens, &targets, &mut active, &mut forced);
}

fn set_group_indents(tokens: &mut Vec<Token>) {
    fn collect(tokens: &[Token], indents: &mut HashMap<GroupId, isize>) {
        for token in tokens {
            match token {
                Token::SetIndent(group, indent) => {
                    indents.insert(*group, *indent);
                }
                Token::LineSuffix(tokens) => collect(tokens, indents),
                _ => {}
            }
        }
    }

    fn apply(tokens: &mut Vec<Token>, indents: &HashMap<GroupId, isize>) {
        for token in tokens.iter_mut() {
            match token {
                Token::Begin(begin) => {
                    if let Some(group) = begin.group
                        && let Some(indent) = indents.get(&group)
                    {
                        begin.indent = IndentStyle::Block { offset: *indent };
                    }
                }
                Token::LineSuffix(tokens) => apply(tokens, indents),
                _ => {}
            }
        }
        tokens.retain(|token| !matches!(token, Token::SetIndent(..)));
    }

    let mut indents = HashMap::new();
    collect(tokens, &mut indents);
    if indents.is_empty() {
        return;
    }
    apply(tokens, &indents);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer() -> Printer {
        Printer::new(40, None, 4)
    }

    #[test]
    fn retained_document_preserves_imperative_output() {
        let mut p = printer();
        p.cbox(4);
        p.word("call(");
        p.zerobreak();
        p.word("argument_that_makes_the_group_too_wide,");
        p.space();
        p.word("other_argument");
        p.word(")");
        p.end();

        assert_eq!(
            p.eof(),
            "call(\n    argument_that_makes_the_group_too_wide,\n    other_argument)"
        );
    }

    #[test]
    fn if_break_uses_named_group_layout() {
        let mut flat = printer();
        let group = flat.cbox_with_id(4);
        flat.word("short");
        flat.space();
        flat.if_break(group, |p| p.word("broken"), |p| p.word("flat"));
        flat.end();
        assert_eq!(flat.eof(), "short flat");

        let mut broken = printer();
        let group = broken.ibox_with_id(4);
        broken.word("a_very_long_prefix_that_uses_the_line");
        broken.space();
        broken.if_break(group, |p| p.word("broken"), |p| p.word("flat"));
        broken.end();
        assert_eq!(broken.eof(), "a_very_long_prefix_that_uses_the_line\n    broken");
    }

    #[test]
    fn if_break_reflects_emitted_breaks() {
        let mut p = printer();
        let group = p.cbox_with_id(4);
        p.word("an_unbreakable_word_that_exceeds_the_margin");
        p.end();
        p.if_break(group, |p| p.word(" broken"), |p| p.word(" flat"));

        assert_eq!(p.eof(), "an_unbreakable_word_that_exceeds_the_margin flat");
    }

    #[test]
    fn break_parent_forces_enclosing_group() {
        let mut p = printer();
        p.cbox(4);
        p.word("left");
        p.space();
        p.break_parent();
        p.word("right");
        p.end();

        assert_eq!(p.eof(), "left\n    right");
    }

    #[test]
    fn break_children_composes_with_if_break() {
        let mut p = printer();
        let outer = p.cbox_with_id(0);
        let target = p.cbox_with_id(0);
        p.cbox(4);
        p.word("left");
        p.space();
        p.word("right");
        p.end();
        p.end();
        p.space();
        p.word("a_suffix_that_makes_the_outer_group_break");
        p.if_break(outer, |p| p.break_children(target), |_| {});
        p.end();

        assert_eq!(p.eof(), "left\n    right\na_suffix_that_makes_the_outer_group_break");
    }

    #[test]
    fn fit_condition_can_preserve_a_fitting_child_layout() {
        let mut p = printer();
        p.cbox(0);
        let (group, fit) = p.ibox_with_fit(4);
        p.word("fitting_child");
        p.space();
        p.word("content");
        p.end();
        p.space();
        p.word("a_suffix_that_makes_the_parent_too_wide");
        p.if_fits(fit, |p| p.flatten_children(group), |_| {});
        p.end();

        assert_eq!(p.eof(), "fitting_child content\na_suffix_that_makes_the_parent_too_wide");
    }

    #[test]
    fn fit_condition_leaves_an_overflowing_child_breakable() {
        let mut p = printer();
        let (group, fit) = p.ibox_with_fit(4);
        p.word("a_child_that_is_too_wide_for_the_margin");
        p.space();
        p.word("content");
        p.end();
        p.if_fits(fit, |p| p.flatten_children(group), |_| {});

        assert_eq!(p.eof(), "a_child_that_is_too_wide_for_the_margin\n    content");
    }

    #[test]
    fn fit_condition_can_change_group_indentation() {
        let mut p = printer();
        let outer = p.ibox_with_id(4);
        let (child, fit) = p.ibox_with_fit(0);
        p.word("fitting child");
        p.end();
        p.space();
        p.word("a_suffix_that_makes_the_outer_group_break");
        p.if_fits(
            fit,
            |p| {
                p.flatten_children(child);
                p.set_indent(outer, 0);
            },
            |_| {},
        );
        p.end();

        assert_eq!(p.eof(), "fitting child\na_suffix_that_makes_the_outer_group_break");
    }

    #[test]
    fn choice_uses_first_fitting_document() {
        let mut preferred = printer();
        preferred.word("prefix ");
        preferred.choice(|p| p.word("preferred"), |p| p.word("fallback"));
        assert_eq!(preferred.eof(), "prefix preferred");

        let mut fallback = printer();
        fallback.word("a_very_long_prefix_that_uses_the_line ");
        fallback.choice(|p| p.word("preferred"), |p| p.word("fallback"));
        assert_eq!(fallback.eof(), "a_very_long_prefix_that_uses_the_line fallback");
    }

    #[test]
    fn line_suffix_is_emitted_at_the_end_of_the_line() {
        let mut p = printer();
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.word(" // comment");
        p.end_line_suffix(suffix);
        p.word(";");
        p.hardbreak();
        p.word("next");

        assert_eq!(p.eof(), "value; // comment\nnext");
    }

    #[test]
    fn line_suffix_preserves_pending_space() {
        let mut p = printer();
        p.word("value");
        p.space();
        let suffix = p.begin_line_suffix();
        p.word("// comment");
        p.end_line_suffix(suffix);

        assert_eq!(p.eof(), "value // comment");
    }

    #[test]
    fn line_suffix_can_break_its_parent() {
        let mut p = printer();
        p.cbox(4);
        p.word("value");
        p.space();
        p.word("next");
        let suffix = p.begin_line_suffix();
        p.break_parent();
        p.end_line_suffix(suffix);
        p.end();

        assert_eq!(p.eof(), "value\n    next");
    }

    #[test]
    fn line_suffix_uses_retained_fit_probes() {
        let mut p = printer();
        p.word("a_very_long_prefix_that_uses_the_line");
        let suffix = p.begin_line_suffix();
        p.choice(|p| p.word(" preferred"), |p| p.word(" fallback"));
        p.end_line_suffix(suffix);

        assert_eq!(p.eof(), "a_very_long_prefix_that_uses_the_line fallback");
    }

    #[test]
    fn child_transforms_cross_line_suffixes() {
        let mut p = printer();
        let outer = p.cbox_with_id(0);
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.cbox(4);
        p.word("left");
        p.space();
        p.word("right");
        p.end();
        p.end_line_suffix(suffix);
        p.break_children(outer);
        p.end();

        assert_eq!(p.eof(), "valueleft\n    right");
    }

    #[test]
    fn line_suffix_break_updates_enclosing_group() {
        let mut p = printer();
        let outer = p.cbox_with_id(0);
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.hardbreak();
        p.end_line_suffix(suffix);
        p.end();
        p.if_break(outer, |p| p.word("broken"), |p| p.word("flat"));

        assert_eq!(p.eof(), "valuebroken\n");
    }

    #[test]
    fn line_suffix_is_flushed_at_eof() {
        let mut p = printer();
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.word(" // comment");
        p.end_line_suffix(suffix);
        p.word(";");

        assert_eq!(p.eof(), "value; // comment");
    }

    #[test]
    fn line_suffix_participates_in_choice_fit() {
        let mut p = printer();
        p.choice(
            |p| {
                p.word("a_preferred_document_of_thirty_chars");
                let suffix = p.begin_line_suffix();
                p.word(" // trailing comment");
                p.end_line_suffix(suffix);
            },
            |p| p.word("fallback"),
        );

        assert_eq!(p.eof(), "fallback");
    }

    #[test]
    fn line_suffix_preserves_live_preview() {
        let mut p = printer();
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.word(" // comment");
        p.end_line_suffix(suffix);

        assert!(p.ends_with('t'));
    }

    #[test]
    fn line_suffix_uses_layout_engine_after_pre_break() {
        let mut p = printer();
        p.cbox(0);
        p.word("value");
        let suffix = p.begin_line_suffix();
        p.cbox(4);
        p.word(" // a_trailing_comment_that_must");
        p.space();
        p.word("wrap");
        p.end();
        p.end_line_suffix(suffix);
        p.break_parent();
        p.scan_break(BreakToken { pre_break: Some("{"), ..BreakToken::default() });
        p.word("next");
        p.end();

        assert_eq!(p.eof(), "value{ // a_trailing_comment_that_must\n    wrap\nnext");
    }

    #[test]
    fn continuation_breaks_when_contents_fit_on_the_next_line() {
        let mut p = printer();
        p.word("a_prefix_that_uses_most_of_the_line");
        p.continuation_box(4, false);
        p.space();
        p.word("short_content");
        p.end();

        assert_eq!(p.eof(), "a_prefix_that_uses_most_of_the_line\n    short_content");
    }

    #[test]
    fn continuation_stays_flat_at_the_current_line_boundary() {
        let mut p = printer();
        p.word("1234567890123456789012345678901234");
        p.continuation_box(4, false);
        p.space();
        p.word("abcde");
        p.end();

        assert_eq!(p.eof(), "1234567890123456789012345678901234 abcde");
    }

    #[test]
    fn continuation_breaks_at_the_continuation_line_boundary() {
        let mut p = printer();
        p.word("1234567890123456789012345678901234567890");
        p.continuation_box(4, false);
        p.space();
        p.word("123456789012345678901234567890123456");
        p.end();

        assert_eq!(
            p.eof(),
            "1234567890123456789012345678901234567890\n    123456789012345678901234567890123456"
        );
    }

    #[test]
    fn continuation_uses_the_actual_leading_break_width() {
        let mut p = printer();
        p.word("12345678901234567890123456789012345678");
        p.continuation_box(4, false);
        p.break_offset(3, 0);
        p.word("x");
        p.end();

        assert_eq!(p.eof(), "12345678901234567890123456789012345678\n    x");
    }

    #[test]
    fn continuation_uses_the_leading_break_line_geometry() {
        let mut p = Printer::new(80, None, 4);
        p.word("123456789012345678901234567890123456789012345678901234567890123456789012345678");
        p.continuation_box(4, true);
        p.scan_break(BreakToken {
            offset: 4,
            blank_space: 1,
            post_break: Some("=> "),
            ..BreakToken::default()
        });
        p.word("call(");
        p.cbox(4);
        p.zerobreak();
        p.word("123456789012345678901234567890123456789012345678901234567890abcde");
        p.zerobreak();
        p.offset(-4);
        p.end();
        p.word(")");
        p.end();

        assert_eq!(
            p.eof(),
            concat!(
                "123456789012345678901234567890123456789012345678901234567890123456789012345678\n",
                "        => call(\n",
                "        123456789012345678901234567890123456789012345678901234567890abcde\n",
                "    )"
            )
        );
    }

    #[test]
    fn continuation_head_can_break_before_nested_layout() {
        let mut p = printer();
        p.word("12345678901234567890123456789012345678");
        p.continuation_box(4, true);
        p.space();
        p.word("123456789012345678901234567890");
        p.space();
        p.word("123456789012345678901234567890");
        p.end();

        assert_eq!(
            p.eof(),
            concat!(
                "12345678901234567890123456789012345678\n",
                "    123456789012345678901234567890\n",
                "    123456789012345678901234567890"
            )
        );
    }

    #[test]
    fn continuation_delegates_long_contents_to_nested_groups() {
        let mut p = printer();
        p.word("declaration =");
        p.continuation_box(4, false);
        p.space();
        p.word("call(");
        p.cbox(4);
        p.zerobreak();
        p.word("an_argument_that_makes_the_call_too_wide,");
        p.space();
        p.word("other");
        p.zerobreak();
        p.offset(-4);
        p.end();
        p.word(")");
        p.end();

        assert_eq!(
            p.eof(),
            "declaration = call(\n    an_argument_that_makes_the_call_too_wide,\n    other\n)"
        );
    }

    #[test]
    fn nested_continuation_keeps_a_fitting_head_on_the_current_line() {
        let mut p = printer();
        p.word("123456789012345678901234567890");
        p.nested_continuation_box(4);
        p.space();
        p.word("call(");
        p.cbox(4);
        p.zerobreak();
        p.word("abcdefghijklmnopqrst");
        p.zerobreak();
        p.offset(-4);
        p.end();
        p.word(")");
        p.end();

        assert_eq!(p.eof(), "123456789012345678901234567890 call(\n    abcdefghijklmnopqrst\n)");
    }

    #[test]
    fn adaptive_continuation_prefers_the_next_line_only_when_contents_fit() {
        let mut fitting = printer();
        fitting.word("1234567890123456789012345678901234");
        fitting.adaptive_continuation_box(4);
        fitting.space();
        fitting.word("call(");
        fitting.cbox(4);
        fitting.zerobreak();
        fitting.word("short_argument");
        fitting.zerobreak();
        fitting.offset(-4);
        fitting.end();
        fitting.word(")");
        fitting.end();

        assert_eq!(fitting.eof(), "1234567890123456789012345678901234\n    call(short_argument)");

        let mut tight = printer();
        tight.word("123456789012345678901234567890");
        tight.adaptive_continuation_box(4);
        tight.space();
        tight.word("call(");
        tight.cbox(4);
        tight.zerobreak();
        tight.word("abcdefghijklmnopqrstuvwx");
        tight.zerobreak();
        tight.offset(-4);
        tight.end();
        tight.word(")");
        tight.end();

        assert_eq!(
            tight.eof(),
            "123456789012345678901234567890 call(\n    abcdefghijklmnopqrstuvwx\n)"
        );

        let mut overflowing = printer();
        overflowing.word("1234567890123456789012345678901234");
        overflowing.adaptive_continuation_box(4);
        overflowing.space();
        overflowing.word("call(");
        overflowing.cbox(4);
        overflowing.zerobreak();
        overflowing.word("an_argument_that_exceeds_the_continuation_line,");
        overflowing.space();
        overflowing.word("other");
        overflowing.zerobreak();
        overflowing.offset(-4);
        overflowing.end();
        overflowing.word(")");
        overflowing.end();

        assert_eq!(
            overflowing.eof(),
            concat!(
                "1234567890123456789012345678901234 call(\n",
                "    an_argument_that_exceeds_the_continuation_line,\n",
                "    other\n",
                ")"
            )
        );
    }

    #[test]
    fn transparent_group_preserves_the_enclosing_break_policy() {
        let mut p = printer();
        p.cbox(4);
        p.word("12345678901234567890123456789012345");
        let group = p.transparent_group(0);
        p.space();
        p.word("left");
        p.space();
        p.word("right");
        p.set_indent(group, 8);
        p.end();
        p.end();

        assert_eq!(
            p.eof(),
            "12345678901234567890123456789012345\n            left\n            right"
        );
    }

    #[test]
    fn nested_continuation_counts_its_separator_at_the_current_line_boundary() {
        let mut p = printer();
        p.word("123456789012345678901234567890");
        p.nested_continuation_box(4);
        p.space();
        p.word("1234567890");
        p.cbox(4);
        p.space();
        p.word("tail");
        p.end();
        p.end();

        assert_eq!(p.eof(), "123456789012345678901234567890\n    1234567890 tail");
    }

    #[test]
    fn isolated_group_stays_flat_when_its_parent_breaks() {
        let mut p = printer();
        p.cbox(0);
        p.word("a_parent_prefix_that_uses_the_entire_line");
        p.space();
        p.isolated_cbox(4);
        p.word("short");
        p.space();
        p.word("child");
        p.end();
        p.end();

        assert_eq!(p.eof(), "a_parent_prefix_that_uses_the_entire_line\nshort child");
    }

    #[test]
    fn isolated_group_does_not_mask_nested_break_parent() {
        let mut p = printer();
        p.isolated_cbox(0);
        p.cbox(4);
        p.word("left");
        p.space();
        p.break_parent();
        p.word("right");
        p.end();
        p.end();

        assert_eq!(p.eof(), "left\n    right");
    }

    #[test]
    fn isolated_group_does_not_mask_break_children() {
        let mut p = printer();
        let outer = p.isolated_cbox(0);
        p.cbox(4);
        p.word("left");
        p.space();
        p.word("right");
        p.end();
        p.break_children(outer);
        p.end();

        assert_eq!(p.eof(), "left\n    right");
    }

    #[test]
    fn forced_children_override_explicit_flattening() {
        let mut p = printer();
        let outer = p.cbox_with_id(0);
        p.cbox(4);
        p.word("left");
        p.space();
        p.word("right");
        p.end();
        p.break_children(outer);
        p.flatten_children(outer);
        p.end();

        assert_eq!(p.eof(), "left\n    right");
    }

    #[test]
    fn forced_outer_group_does_not_override_child_flattening() {
        let mut p = printer();
        p.cbox(0);
        let child = p.cbox_with_id(0);
        p.word("12345678901234567890123456789012345");
        p.space();
        p.word("right");
        p.end();
        p.break_parent();
        p.flatten_children(child);
        p.end();

        assert_eq!(p.eof(), "12345678901234567890123456789012345 right");
    }

    #[test]
    fn isolated_group_slack_allows_controlled_overflow() {
        let mut p = printer();
        p.isolated_cbox_with_slack(4, 4);
        p.word("1234567890123456789012345678901234567890");
        p.space();
        p.word("x");
        p.end();

        assert_eq!(p.eof(), "1234567890123456789012345678901234567890 x");
    }

    #[test]
    fn hardbreak_remains_forced_in_a_delegating_continuation() {
        let mut p = printer();
        p.continuation_box(4, false);
        p.word("left");
        p.hardbreak();
        p.word("right");
        p.end();

        assert_eq!(p.eof(), "left\nright");
    }

    #[test]
    fn continuation_layout_is_annotated_inside_line_suffixes() {
        let mut p = printer();
        p.word("12345678901234567890123456789012345678");
        let suffix = p.begin_line_suffix();
        p.continuation_box(4, false);
        p.space();
        p.word("short");
        p.end();
        p.end_line_suffix(suffix);

        assert_eq!(p.eof(), "12345678901234567890123456789012345678\n    short");
    }

    #[test]
    fn display_width_uses_terminal_columns() {
        assert_eq!(display_width("a界e\u{301}", 4), 4);
    }
}
