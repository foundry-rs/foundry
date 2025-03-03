//! SolidityHelper
//!
//! This module contains the `SolidityHelper`, a [rustyline::Helper] implementation for
//! usage in Chisel. It is ported from [soli](https://github.com/jpopesculian/soli/blob/master/src/main.rs).

use crate::{
    dispatcher::PROMPT_ARROW,
    prelude::{ChiselCommand, COMMAND_LEADER, PROMPT_ARROW_STR},
};
use rustyline::{
    completion::Completer,
    highlight::{CmdKind, Highlighter},
    hint::Hinter,
    validate::{ValidationContext, ValidationResult, Validator},
    Helper,
};
use solar_parse::{
    interface::{Session, SessionGlobals},
    token::{Token, TokenKind},
    Lexer,
};
use std::{borrow::Cow, ops::Range, str::FromStr};
use yansi::{Color, Style};

/// The maximum length of an ANSI prefix + suffix characters using [SolidityHelper].
///
/// * 5 - prefix:
///   * 2 - start: `\x1B[`
///   * 2 - fg: `3<fg_code>`
///   * 1 - end: `m`
/// * 4 - suffix: `\x1B[0m`
const MAX_ANSI_LEN: usize = 9;

/// A rustyline helper for Solidity code
pub struct SolidityHelper {
    errored: bool,

    do_paint: bool,
    sess: Session,
    globals: SessionGlobals,
}

impl Default for SolidityHelper {
    fn default() -> Self {
        Self::new()
    }
}

impl SolidityHelper {
    /// Create a new SolidityHelper.
    pub fn new() -> Self {
        Self {
            errored: false,
            do_paint: yansi::is_enabled(),
            sess: Session::builder().with_silent_emitter(None).build(),
            globals: SessionGlobals::new(),
        }
    }

    /// Returns whether the helper is in an errored state.
    pub fn errored(&self) -> bool {
        self.errored
    }

    /// Set the errored field.
    pub fn set_errored(&mut self, errored: bool) -> &mut Self {
        self.errored = errored;
        self
    }

    /// Highlights a Solidity source string.
    pub fn highlight<'a>(&self, input: &'a str) -> Cow<'a, str> {
        if !self.do_paint() {
            return Cow::Borrowed(input);
        }

        // Highlight commands separately
        if input.starts_with(COMMAND_LEADER) {
            let (cmd, rest) = match input.split_once(' ') {
                Some((cmd, rest)) => (cmd, Some(rest)),
                None => (input, None),
            };
            let cmd = cmd.strip_prefix(COMMAND_LEADER).unwrap_or(cmd);

            let mut out = String::with_capacity(input.len() + MAX_ANSI_LEN);

            // cmd
            out.push(COMMAND_LEADER);
            let cmd_res = ChiselCommand::from_str(cmd);
            let style = (if cmd_res.is_ok() { Color::Green } else { Color::Red }).foreground();
            Self::paint_unchecked(cmd, style, &mut out);

            // rest
            match rest {
                Some(rest) if !rest.is_empty() => {
                    out.push(' ');
                    out.push_str(rest);
                }
                _ => {}
            }

            Cow::Owned(out)
        } else {
            let mut out = String::with_capacity(input.len() * 2);
            self.with_contiguous_styles(input, |style, range| {
                Self::paint_unchecked(&input[range], style, &mut out);
            });
            Cow::Owned(out)
        }
    }

    /// Returns a list of styles and the ranges they should be applied to.
    ///
    /// Covers the entire source string, including any whitespace.
    fn with_contiguous_styles(&self, input: &str, mut f: impl FnMut(Style, Range<usize>)) {
        self.enter(|sess| {
            let len = input.len();
            let mut index = 0;
            for token in Lexer::new(sess, input) {
                let range = token.span.lo().to_usize()..token.span.hi().to_usize();
                let style = token_style(&token);
                if index < range.start {
                    f(Style::default(), index..range.start);
                }
                index = range.end;
                f(style, range);
            }
            if index < len {
                f(Style::default(), index..len);
            }
        });
    }

    /// Validate that a source snippet is closed (i.e., all braces and parenthesis are matched).
    fn validate_closed(&self, input: &str) -> ValidationResult {
        let mut depth = [0usize; 3];
        self.enter(|sess| {
            for token in Lexer::new(sess, input) {
                match token.kind {
                    TokenKind::OpenDelim(delim) => {
                        depth[delim as usize] += 1;
                    }
                    TokenKind::CloseDelim(delim) => {
                        depth[delim as usize] = depth[delim as usize].saturating_sub(1);
                    }
                    _ => {}
                }
            }
        });
        if depth == [0; 3] {
            ValidationResult::Valid(None)
        } else {
            ValidationResult::Incomplete
        }
    }

    /// Formats `input` with `style` into `out`, without checking `style.wrapping` or
    /// `self.do_paint`.
    fn paint_unchecked(string: &str, style: Style, out: &mut String) {
        if style == Style::default() {
            out.push_str(string);
        } else {
            let _ = style.fmt_prefix(out);
            out.push_str(string);
            let _ = style.fmt_suffix(out);
        }
    }

    fn paint_unchecked_owned(string: &str, style: Style) -> String {
        let mut out = String::with_capacity(MAX_ANSI_LEN + string.len());
        Self::paint_unchecked(string, style, &mut out);
        out
    }

    /// Returns whether to color the output.
    fn do_paint(&self) -> bool {
        self.do_paint
    }

    /// Enters the session.
    fn enter(&self, f: impl FnOnce(&Session)) {
        self.globals.set(|| self.sess.enter(|| f(&self.sess)));
    }
}

impl Highlighter for SolidityHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        self.highlight(line)
    }

    fn highlight_char(&self, line: &str, pos: usize, _kind: CmdKind) -> bool {
        pos == line.len()
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        if !self.do_paint() {
            return Cow::Borrowed(prompt);
        }

        let mut out = prompt.to_string();

        // `^(\(ID: .*?\) )? ➜ `
        if prompt.starts_with("(ID: ") {
            let id_end = prompt.find(')').unwrap();
            let id_span = 5..id_end;
            let id = &prompt[id_span.clone()];
            out.replace_range(
                id_span,
                &Self::paint_unchecked_owned(id, Color::Yellow.foreground()),
            );
            out.replace_range(1..=2, &Self::paint_unchecked_owned("ID", Color::Cyan.foreground()));
        }

        if let Some(i) = out.find(PROMPT_ARROW) {
            let style =
                if self.errored { Color::Red.foreground() } else { Color::Green.foreground() };
            out.replace_range(i..=i + 2, &Self::paint_unchecked_owned(PROMPT_ARROW_STR, style));
        }

        Cow::Owned(out)
    }
}

impl Validator for SolidityHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(self.validate_closed(ctx.input()))
    }
}

impl Completer for SolidityHelper {
    type Candidate = String;
}

impl Hinter for SolidityHelper {
    type Hint = String;
}

impl Helper for SolidityHelper {}

#[allow(non_upper_case_globals)]
#[deny(unreachable_patterns)]
fn token_style(token: &Token) -> Style {
    use solar_parse::{
        interface::kw::*,
        token::{TokenKind::*, TokenLitKind::*},
    };

    match token.kind {
        Literal(Str | HexStr | UnicodeStr, _) => Color::Green.foreground(),
        Literal(..) => Color::Yellow.foreground(),

        Ident(
            Memory | Storage | Calldata | Public | Private | Internal | External | Constant | Pure
            | View | Payable | Anonymous | Indexed | Abstract | Virtual | Override | Modifier
            | Immutable | Unchecked,
        ) => Color::Cyan.foreground(),

        Ident(s) if s.is_elementary_type() => Color::Blue.foreground(),
        Ident(Mapping) => Color::Blue.foreground(),

        Ident(s) if s.is_used_keyword() || s.is_yul_keyword() => Color::Magenta.foreground(),
        Arrow | FatArrow => Color::Magenta.foreground(),

        Comment(..) => Color::Primary.dim(),

        _ => Color::Primary.foreground(),
    }
}
