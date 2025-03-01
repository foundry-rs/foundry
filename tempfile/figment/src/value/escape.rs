// This file is a partial, reduced, extended, and modified reproduction of
// `toml-rs`, Copyright (c) 2014 Alex Crichton, The MIT License, reproduced and
// relicensed, under the rights granted by The MIT License, under The MIT
// License or Apache License, Version 2.0, January 2004, at the user's
// discretion, Copyright (c) 2020 Sergio Benitez.
//
// See README.md, LICENSE-MIT, LICENSE-APACHE.

use core::fmt;
use std::borrow::Cow;

#[derive(Eq, PartialEq, Debug)]
pub enum Error {
    InvalidCharInString(usize, char),
    InvalidEscape(usize, char),
    InvalidHexEscape(usize, char),
    InvalidEscapeValue(usize, u32),
    UnterminatedString(usize),
}

pub fn escape(string: &str) -> Result<Cow<'_, str>, Error> {
    let mut chars = string.chars().enumerate();
    let mut output = Cow::from(string);
    while let Some((i, ch)) = chars.next() {
        match ch {
            '\\' => {
                if let Cow::Borrowed(_) = output {
                    output = Cow::Owned(string[..i].into());
                }

                let val = output.to_mut();
                match chars.next() {
                    Some((_, '"')) => val.push('"'),
                    Some((_, '\\')) => val.push('\\'),
                    Some((_, 'b')) => val.push('\u{8}'),
                    Some((_, 'f')) => val.push('\u{c}'),
                    Some((_, 'n')) => val.push('\n'),
                    Some((_, 'r')) => val.push('\r'),
                    Some((_, 't')) => val.push('\t'),
                    Some((i, c @ 'u')) | Some((i, c @ 'U')) => {
                        let len = if c == 'u' { 4 } else { 8 };
                        val.push(hex(&mut chars, i, len)?);
                    }
                    Some((i, c)) => return Err(Error::InvalidEscape(i, c)),
                    None => return Err(Error::UnterminatedString(0)),
                }
            },
            ch if ch == '\u{09}' || ('\u{20}' <= ch && ch <= '\u{10ffff}' && ch != '\u{7f}') => {
                // if we haven't allocated, the string contains the value
                if let Cow::Owned(ref mut val) = output {
                    val.push(ch);
                }
            },
            _ => return Err(Error::InvalidCharInString(i, ch)),
        }
    }

    Ok(output)
}

fn hex<I>(mut chars: I, i: usize, len: usize) -> Result<char, Error>
    where I: Iterator<Item = (usize, char)>
{
    let mut buf = String::with_capacity(len);
    for _ in 0..len {
        match chars.next() {
            Some((_, ch)) if ch as u32 <= 0x7F && ch.is_digit(16) => buf.push(ch),
            Some((i, ch)) => return Err(Error::InvalidHexEscape(i, ch)),
            None => return Err(Error::UnterminatedString(0)),
        }
    }

    let val = u32::from_str_radix(&buf, 16).unwrap();
    match std::char::from_u32(val) {
        Some(ch) => Ok(ch),
        None => Err(Error::InvalidEscapeValue(i, val)),
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidCharInString(_, ch) => write!(f, "invalid char `{:?}`", ch),
            Error::InvalidEscape(_, ch) => write!(f, "invalid escape `\\{:?}`", ch),
            Error::InvalidHexEscape(_, ch) => write!(f, "invalid hex escape `{:?}`", ch),
            Error::InvalidEscapeValue(_, ch) => write!(f, "invalid escaped value `{:?}`", ch),
            Error::UnterminatedString(_) => write!(f, "unterminated"),
        }
    }
}
