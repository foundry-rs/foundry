//! SolidityHelper
//!
//! This module contains the `SolidityHelper`, a [rustyline::Helper] implementation for
//! usage in Chisel. It is ported from [soli](https://github.com/jpopesculian/soli/blob/master/src/main.rs).

use crate::prelude::ChiselCommand;
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

/// A rustyline helper for Solidity code
pub struct SolidityHelper;

/// Highlighter implementation for `SolHighlighter`
impl Highlighter for SolidityHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Owned(SolidityHelper::highlight(line))
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        pos == line.len()
    }
}

impl SolidityHelper {
    /// Get styles for a solidity source string
    pub fn get_styles(input: &str) -> Vec<(usize, Style, usize)> {
        let mut comments = Vec::new();
        let mut out = Lexer::new(input, 0, &mut comments)
            .flatten()
            .map(|(start, token, end)| (start, token.style(), end))
            .collect::<Vec<_>>();
        for comment in comments {
            let loc = match comment {
                pt::Comment::Line(loc, _) |
                pt::Comment::Block(loc, _) |
                pt::Comment::DocLine(loc, _) |
                pt::Comment::DocBlock(loc, _) => loc,
            };
            out.push((loc.start(), Style::default().dimmed(), loc.end()))
        }
        out
    }

    /// Get contiguous styles for a solidity source string
    pub fn get_contiguous_styles(input: &str) -> Vec<(usize, Style, usize)> {
        let mut styles = Self::get_styles(input);
        styles.sort_by_key(|(start, _, _)| *start);
        let mut out = vec![];
        let mut index = 0;
        for (start, style, end) in styles {
            if index < start {
                out.push((index, Style::default(), start));
            }
            out.push((start, style, end));
            index = end;
        }
        if index < input.len() {
            out.push((index, Style::default(), input.len()));
        }
        out
    }

    /// Highlights a solidity source string
    pub fn highlight(input: &str) -> String {
        let mut out = String::new();
        // Highlight commands separately
        if input.starts_with('!') {
            let split: Vec<&str> = input.split(' ').collect();
            let cmd = ChiselCommand::from_str(&split[0][1..]);
            out = format!(
                "!{} {}",
                if cmd.is_ok() { Paint::green(&split[0][1..]) } else { Paint::red(&split[0][1..]) },
                split[1..].join(" ")
            );
        } else {
            for (start, style, end) in Self::get_contiguous_styles(input) {
                out.push_str(&format!("{}", style.paint(&input[start..end])))
            }
        }
        out
    }

    /// Validate that a source snippet is closed (i.e., all braces and parenthesis are matched).
    fn validate_closed(input: &str) -> ValidationResult {
        let mut bracket_depth = 0usize;
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut comments = Vec::new();
        for res in Lexer::new(input, 0, &mut comments) {
            match res {
                Err(err) => match err {
                    LexicalError::EndOfFileInComment(_) |
                    LexicalError::EndofFileInHex(_) |
                    LexicalError::EndOfFileInString(_) => return ValidationResult::Incomplete,
                    _ => return ValidationResult::Valid(None),
                },
                Ok((_, token, _)) => match token {
                    Token::OpenBracket => {
                        bracket_depth = bracket_depth.saturating_add(1);
                    }
                    Token::OpenCurlyBrace => {
                        brace_depth = brace_depth.saturating_add(1);
                    }
                    Token::OpenParenthesis => {
                        paren_depth = paren_depth.saturating_add(1);
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
            Seconds |
            Minutes |
            Hours |
            Days |
            Weeks |
            Gwei |
            Wei |
            Ether |
            This => Color::Yellow.style(),
            Memory | Storage | Calldata | Public | Private | Internal | External | Constant |
            Pure | View | Payable | Anonymous | Indexed | Abstract | Virtual | Override |
            Modifier | Immutable | Unchecked => Color::Cyan.style(),
            Contract | Library | Interface | Function | Pragma | Import | Struct | Event |
            Error | Enum | Type | Constructor | As | Is | Using | New | Delete | Do |
            Continue | Break | Throw | Emit | Return | Returns | Revert | For | While | If |
            Else | Try | Catch | Assembly | Let | Leave | Switch | Case | Default | YulArrow |
            Arrow => Color::Magenta.style(),
            Uint(_) | Int(_) | Bytes(_) | Byte | DynamicBytes | Bool | Address | String |
            Mapping => Color::Blue.style(),
            Identifier(_) => Style::default(),
            _ => Style::default(),
        }
    }
}
