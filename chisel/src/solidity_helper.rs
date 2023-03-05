//! SolidityHelper
//!
//! This module contains the `SolidityHelper`, a [rustyline::Helper] implementation for
//! usage in Chisel. It is ported from [soli](https://github.com/jpopesculian/soli/blob/master/src/main.rs).

use crate::prelude::{ChiselCommand, COMMAND_LEADER};
use rustyline::{
    completion::Completer,
    highlight::Highlighter,
    hint::Hinter,
    validate::{ValidationContext, ValidationResult, Validator},
    Helper,
};
use solang_parser::{
    lexer::{Lexer, LexicalError, Token},
    pt,
};
use std::{borrow::Cow, str::FromStr};
use yansi::{Color, Paint, Style};

/// The default pre-allocation for solang parsed comments
const DEFAULT_COMMENTS: usize = 5;

/// The maximum length of an ANSI prefix + suffix characters using [SolidityHelper].
///
/// * 5 - prefix:
///   * 2 - start: `\x1B[`
///   * 2 - fg: `3<fg_code>`
///   * 1 - end: `m`
/// * 4 - suffix: `\x1B[0m`
const MAX_ANSI_LEN: usize = 9;

/// `(start, style, end)`
pub type SpannedStyle = (usize, Style, usize);

/// A rustyline helper for Solidity code
pub struct SolidityHelper;

impl SolidityHelper {
    /// Get styles for a solidity source string
    pub fn get_styles(input: &str) -> Vec<SpannedStyle> {
        let mut comments = Vec::with_capacity(DEFAULT_COMMENTS);
        let mut errors = Vec::with_capacity(5);
        let mut out = Lexer::new(input, 0, &mut comments, &mut errors)
            .flatten()
            .map(|(start, token, end)| (start, token.style(), end))
            .collect::<Vec<_>>();

        // highlight comments too
        let comments_iter = comments.into_iter().map(|comment| {
            let loc = match comment {
                pt::Comment::Line(loc, _) |
                pt::Comment::Block(loc, _) |
                pt::Comment::DocLine(loc, _) |
                pt::Comment::DocBlock(loc, _) => loc,
            };
            (loc.start(), Style::default().dimmed(), loc.end())
        });
        out.extend(comments_iter);

        out
    }

    /// Get contiguous styles for a solidity source string
    pub fn get_contiguous_styles(input: &str) -> Vec<SpannedStyle> {
        let mut styles = Self::get_styles(input);
        styles.sort_unstable_by_key(|(start, _, _)| *start);

        let len = input.len();
        // len / 4 is just a random average of whitespaces in the input
        let mut out = Vec::with_capacity(styles.len() + len / 4 + 1);
        let mut index = 0;
        for (start, style, end) in styles {
            if index < start {
                out.push((index, Style::default(), start));
            }
            out.push((start, style, end));
            index = end;
        }
        if index < len {
            out.push((index, Style::default(), len));
        }
        out
    }

    /// Highlights a solidity source string
    pub fn highlight(input: &str) -> Cow<str> {
        if !Paint::is_enabled() || Self::skip_input(input) {
            return Cow::Borrowed(input)
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
            let style = Style::new(if cmd_res.is_ok() { Color::Green } else { Color::Red });
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
            let styles = Self::get_contiguous_styles(input);
            let len = styles.len();
            if len == 0 {
                Cow::Borrowed(input)
            } else {
                let mut out = String::with_capacity(input.len() + MAX_ANSI_LEN * len);
                for (start, style, end) in styles {
                    Self::paint_unchecked(&input[start..end], style, &mut out);
                }
                Cow::Owned(out)
            }
        }
    }

    /// Validate that a source snippet is closed (i.e., all braces and parenthesis are matched).
    fn validate_closed(input: &str) -> ValidationResult {
        if Self::skip_input(input) {
            let msg = Paint::red("\nInput must not start with `.<number>`");
            return ValidationResult::Invalid(Some(msg.to_string()))
        }

        let mut bracket_depth = 0usize;
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut comments = Vec::with_capacity(DEFAULT_COMMENTS);
        // returns on any encountered error, so allocate for just one
        let mut errors = Vec::with_capacity(1);
        for res in Lexer::new(input, 0, &mut comments, &mut errors) {
            match res {
                Err(err) => match err {
                    LexicalError::EndOfFileInComment(_) |
                    LexicalError::EndofFileInHex(_) |
                    LexicalError::EndOfFileInString(_) => return ValidationResult::Incomplete,
                    _ => return ValidationResult::Valid(None),
                },
                Ok((_, token, _)) => match token {
                    Token::OpenBracket => {
                        bracket_depth += 1;
                    }
                    Token::OpenCurlyBrace => {
                        brace_depth += 1;
                    }
                    Token::OpenParenthesis => {
                        paren_depth += 1;
                    }
                    Token::CloseBracket => {
                        bracket_depth = bracket_depth.saturating_sub(1);
                    }
                    Token::CloseCurlyBrace => {
                        brace_depth = brace_depth.saturating_sub(1);
                    }
                    Token::CloseParenthesis => {
                        paren_depth = paren_depth.saturating_sub(1);
                    }
                    _ => {}
                },
            }
        }
        if (bracket_depth | brace_depth | paren_depth) == 0 {
            ValidationResult::Valid(None)
        } else {
            ValidationResult::Incomplete
        }
    }

    /// Formats `input` with `style` into `out`, without checking `style.wrapping` or
    /// `Paint::is_enabled`
    #[inline]
    fn paint_unchecked(string: &str, style: Style, out: &mut String) {
        let _ = style.fmt_prefix(out);
        out.push_str(string);
        let _ = style.fmt_suffix(out);
    }

    /// Whether to skip parsing this input due to known errors or panics
    #[inline]
    fn skip_input(input: &str) -> bool {
        // input.startsWith(/\.[0-9]/)
        let mut chars = input.chars();
        chars.next() == Some('.') && chars.next().map(|c| c.is_ascii_digit()).unwrap_or_default()
    }
}

impl Highlighter for SolidityHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Self::highlight(line)
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        pos == line.len()
    }
}

impl Validator for SolidityHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        Ok(Self::validate_closed(ctx.input()))
    }
}

impl Completer for SolidityHelper {
    type Candidate = String;
}

impl Hinter for SolidityHelper {
    type Hint = String;
}

impl Helper for SolidityHelper {}

/// Trait that assigns a color to a Token kind
pub trait TokenStyle {
    /// Returns the style with which the token should be decorated with.
    fn style(&self) -> Style;
}

/// [TokenStyle] implementation for [Token]
impl<'a> TokenStyle for Token<'a> {
    fn style(&self) -> Style {
        use Token::*;
        match self {
            StringLiteral(_, _) => Style::new(Color::Green),
            AddressLiteral(_) |
            HexLiteral(_) |
            Number(_, _) |
            RationalNumber(_, _, _) |
            HexNumber(_) |
            True |
            False |
            This => Color::Yellow.style(),
            Memory | Storage | Calldata | Public | Private | Internal | External | Constant |
            Pure | View | Payable | Anonymous | Indexed | Abstract | Virtual | Override |
            Modifier | Immutable | Unchecked => Color::Cyan.style(),
            Contract | Library | Interface | Function | Pragma | Import | Struct | Event |
            Enum | Type | Constructor | As | Is | Using | New | Delete | Do | Continue |
            Break | Throw | Emit | Return | Returns | Revert | For | While | If | Else | Try |
            Catch | Assembly | Let | Leave | Switch | Case | Default | YulArrow | Arrow => {
                Color::Magenta.style()
            }
            Uint(_) | Int(_) | Bytes(_) | Byte | DynamicBytes | Bool | Address | String |
            Mapping => Color::Blue.style(),
            Identifier(_) => Style::default(),
            _ => Style::default(),
        }
    }
}
