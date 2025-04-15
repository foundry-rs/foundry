//! Adapted from [`rustc_ast_pretty`](https://github.com/rust-lang/rust/blob/07d3fd1d9b9c1f07475b96a9d168564bf528db68/compiler/rustc_ast_pretty/src/pp.rs)
//! and [`prettyplease`](https://github.com/dtolnay/prettyplease/blob/8eb8c14649aea32e810732bd4d64fe519e6b752a/src/algorithm.rs).

use ring::RingBuffer;
use std::{borrow::Cow, cmp, collections::VecDeque, iter};

mod convenience;
mod helpers;
mod ring;

const DEBUG: bool = false;
const DEBUG_INDENT: bool = false;

// TODO(dani): config

// Target line width.
const MARGIN: isize = 89;

// Number of spaces increment at each level of block indentation.
const INDENT: isize = 4;

// Every line is allowed at least this much space, even if highly indented.
const MIN_SPACE: isize = 60;

/// How to break. Described in more detail in the module docs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Breaks {
    Consistent,
    Inconsistent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum IndentStyle {
    /// Vertically aligned under whatever column this block begins at.
    ///
    ///     fn demo(arg1: usize,
    ///             arg2: usize) {}
    Visual,
    /// Indented relative to the indentation level of the previous line.
    ///
    ///     fn demo(
    ///         arg1: usize,
    ///         arg2: usize,
    ///     ) {}
    Block { offset: isize },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BreakToken {
    offset: isize,
    blank_space: isize,
    pre_break: Option<char>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BeginToken {
    indent: IndentStyle,
    breaks: Breaks,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Token {
    // In practice a string token contains either a `&'static str` or a
    // `String`. `Cow` is overkill for this because we never modify the data,
    // but it's more convenient than rolling our own more specialized type.
    String(Cow<'static, str>),
    Break(BreakToken),
    Begin(BeginToken),
    End,
}

#[derive(Debug, Copy, Clone)]
enum PrintFrame {
    Fits(Breaks),
    Broken(usize, Breaks),
}

const SIZE_INFINITY: isize = 0xffff;

#[derive(Debug)]
pub struct Printer {
    out: String,
    /// Number of spaces left on line
    space: isize,
    /// Ring-buffer of tokens and calculated sizes
    buf: RingBuffer<BufEntry>,
    /// Running size of stream "...left"
    left_total: isize,
    /// Running size of stream "...right"
    right_total: isize,
    /// Pseudo-stack, really a ring too. Holds the
    /// primary-ring-buffers index of the Begin that started the
    /// current block, possibly with the most recent Break after that
    /// Begin (if there is any) on top of it. Stuff is flushed off the
    /// bottom as it becomes irrelevant due to the primary ring-buffer
    /// advancing.
    scan_stack: VecDeque<usize>,
    /// Stack of blocks-in-progress being flushed by print
    print_stack: Vec<PrintFrame>,
    /// Level of indentation of current line
    indent: usize,
    /// Buffered indentation to avoid writing trailing whitespace
    pending_indentation: usize,
    /// The token most recently popped from the left boundary of the
    /// ring-buffer for printing
    last_printed: Option<Token>,
}

#[derive(Debug)]
struct BufEntry {
    token: Token,
    size: isize,
}

impl Printer {
    pub fn new() -> Self {
        Printer {
            out: String::new(),
            space: MARGIN,
            buf: RingBuffer::new(),
            left_total: 0,
            right_total: 0,
            scan_stack: VecDeque::new(),
            print_stack: Vec::new(),
            indent: 0,
            pending_indentation: 0,
            last_printed: None,
        }
    }

    pub(crate) fn last_token(&self) -> Option<&Token> {
        self.last_token_still_buffered().or_else(|| self.last_printed.as_ref())
    }

    pub(crate) fn last_token_still_buffered(&self) -> Option<&Token> {
        self.buf.last().map(|last| &last.token)
    }

    /// Be very careful with this!
    pub(crate) fn replace_last_token_still_buffered(&mut self, token: Token) {
        self.buf.last_mut().unwrap().token = token;
    }

    fn scan_eof(&mut self) {
        if !self.scan_stack.is_empty() {
            self.check_stack(0);
            self.advance_left();
        }
    }

    fn scan_begin(&mut self, token: BeginToken) {
        if self.scan_stack.is_empty() {
            self.left_total = 1;
            self.right_total = 1;
            self.buf.clear();
        }
        let right = self.buf.push(BufEntry { token: Token::Begin(token), size: -self.right_total });
        self.scan_stack.push_back(right);
    }

    fn scan_end(&mut self) {
        if self.scan_stack.is_empty() {
            self.print_end();
        } else {
            let right = self.buf.push(BufEntry { token: Token::End, size: -1 });
            self.scan_stack.push_back(right);
        }
    }

    fn scan_break(&mut self, token: BreakToken) {
        if self.scan_stack.is_empty() {
            self.left_total = 1;
            self.right_total = 1;
            self.buf.clear();
        } else {
            self.check_stack(0);
        }
        let right = self.buf.push(BufEntry { token: Token::Break(token), size: -self.right_total });
        self.scan_stack.push_back(right);
        self.right_total += token.blank_space;
    }

    fn scan_string(&mut self, string: Cow<'static, str>) {
        if self.scan_stack.is_empty() {
            self.print_string(&string);
        } else {
            let len = string.len() as isize;
            self.buf.push(BufEntry { token: Token::String(string), size: len });
            self.right_total += len;
            self.check_stream();
        }
    }

    pub(crate) fn offset(&mut self, offset: isize) {
        if let Some(BufEntry { token: Token::Break(token), .. }) = self.buf.last_mut() {
            token.offset += offset;
        }
    }

    fn check_stream(&mut self) {
        while self.right_total - self.left_total > self.space {
            if *self.scan_stack.front().unwrap() == self.buf.index_of_first() {
                self.scan_stack.pop_front().unwrap();
                self.buf.first_mut().unwrap().size = SIZE_INFINITY;
            }
            self.advance_left();
            if self.buf.is_empty() {
                break;
            }
        }
    }

    fn advance_left(&mut self) {
        while self.buf.first().unwrap().size >= 0 {
            let left = self.buf.pop_first().unwrap();

            match &left.token {
                Token::String(string) => {
                    self.left_total += string.len() as isize;
                    self.print_string(string);
                }
                Token::Break(token) => {
                    self.left_total += token.blank_space;
                    self.print_break(*token, left.size);
                }
                Token::Begin(token) => self.print_begin(*token, left.size),
                Token::End => self.print_end(),
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
            // TODO(dani)
            /*
            if DEBUG_INDENT {
                self.out.extend(token.offset.to_string().chars().map(|ch| match ch {
                    '0'..='9' => ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉']
                        [(ch as u8 - b'0') as usize],
                    '-' => '₋',
                    _ => unreachable!(),
                }));
            }
            */
        }

        if size > self.space {
            self.print_stack.push(PrintFrame::Broken(self.indent, token.breaks));
            self.indent = match token.indent {
                IndentStyle::Block { offset } => {
                    usize::try_from(self.indent as isize + offset).unwrap()
                }
                IndentStyle::Visual => (MARGIN - self.space) as usize,
            };
        } else {
            self.print_stack.push(PrintFrame::Fits(token.breaks));
        }
    }

    fn print_end(&mut self) {
        let breaks = match self.print_stack.pop().unwrap() {
            PrintFrame::Broken(indent, breaks) => {
                self.indent = indent;
                breaks
            }
            PrintFrame::Fits(breaks) => breaks,
        };
        if DEBUG {
            self.out.push(match breaks {
                Breaks::Consistent => '»',
                Breaks::Inconsistent => '›',
            });
        }
    }

    fn print_break(&mut self, token: BreakToken, size: isize) {
        let fits = /* token.never_break || */
            match self.get_top() {
                PrintFrame::Fits(..) => true,
                PrintFrame::Broken(.., Breaks::Consistent) => false,
                PrintFrame::Broken(.., Breaks::Inconsistent) => size <= self.space,
            };
        if fits {
            self.pending_indentation += usize::try_from(token.blank_space).unwrap();
            self.space -= token.blank_space as isize;
            // TODO(dani)
            /*
            if let Some(no_break) = token.no_break {
                self.out.push(no_break);
                self.space -= no_break.len_utf8() as isize;
            }
            */
            if DEBUG {
                self.out.push('·');
            }
        } else {
            if let Some(pre_break) = token.pre_break {
                self.print_indent();
                self.out.push(pre_break);
            }
            if DEBUG {
                self.out.push('·');
            }
            self.out.push('\n');
            let indent = self.indent as isize + token.offset;
            self.pending_indentation = usize::try_from(indent).unwrap();
            // TODO(dani): config
            self.space = cmp::max(MARGIN - indent, MIN_SPACE);
            // TODO(dani)
            /*
            if !token.post_break.is_empty() {
                self.print_indent();
                self.out.push_str(token.post_break);
                self.space -= token.post_break.len() as isize;
            }
            */
        }
    }

    fn print_string(&mut self, string: &str) {
        self.print_indent();
        self.out.push_str(string);
        self.space -= string.len() as isize;
    }

    fn print_indent(&mut self) {
        self.out.reserve(self.pending_indentation);
        self.out.extend(iter::repeat(' ').take(self.pending_indentation));
        self.pending_indentation = 0;
    }
}
