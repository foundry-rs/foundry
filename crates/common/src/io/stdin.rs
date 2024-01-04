//! Utility functions for reading from [`stdin`](std::io::stdin).

use eyre::Result;
use std::{
    error::Error as StdError,
    io::{self, BufRead, Read},
    str::FromStr,
};

/// Unwraps the given `Option<T>` or [reads stdin into a String](read) and parses it as `T`.
pub fn unwrap<T>(value: Option<T>, read_line: bool) -> Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    match value {
        Some(value) => Ok(value),
        None => parse(read_line),
    }
}

/// Shortcut for `(unwrap(a), unwrap(b))`.
#[inline]
pub fn unwrap2<A, B>(a: Option<A>, b: Option<B>) -> Result<(A, B)>
where
    A: FromStr,
    B: FromStr,
    A::Err: StdError + Send + Sync + 'static,
    B::Err: StdError + Send + Sync + 'static,
{
    match (a, b) {
        (Some(a), Some(b)) => Ok((a, b)),
        (a, b) => Ok((unwrap(a, true)?, unwrap(b, true)?)),
    }
}

/// [Reads stdin into a String](read) and parses it as `Vec<T>` using whitespaces as delimiters if
/// the given `Vec<T>` is empty.
pub fn unwrap_vec<T>(mut value: Vec<T>) -> Result<Vec<T>>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    if value.is_empty() {
        let s = read(false)?;
        value = s.split_whitespace().map(FromStr::from_str).collect::<Result<Vec<T>, _>>()?;
    }

    Ok(value)
}

/// Short-hand for `unwrap(value, true)`.
#[inline]
pub fn unwrap_line<T>(value: Option<T>) -> Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    unwrap(value, true)
}

/// Reads bytes from [`stdin`][io::stdin] into a String.
///
/// If `read_line` is true, stop at the first newline (the `0xA` byte).
#[inline]
pub fn parse<T>(read_line: bool) -> Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    read(read_line).and_then(|s| s.parse().map_err(Into::into))
}

/// Short-hand for `parse(true)`.
#[inline]
pub fn parse_line<T>() -> Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    parse(true)
}

/// Reads bytes from [`stdin`][io::stdin] into a String.
///
/// If `read_line` is true, stop at the first newline (the `0xA` byte).
#[inline]
pub fn read(read_line: bool) -> Result<String> {
    let bytes = read_bytes(read_line)?;

    if read_line {
        // SAFETY: [BufRead::read_line] appends into a String
        Ok(unsafe { String::from_utf8_unchecked(bytes) })
    } else {
        String::from_utf8(bytes).map_err(Into::into)
    }
}

/// Reads bytes from [`stdin`][io::stdin].
///
/// If `read_line` is true, read up to the first newline excluded (the `0xA` byte).
pub fn read_bytes(read_line: bool) -> Result<Vec<u8>> {
    let mut stdin = io::stdin().lock();

    if read_line {
        let mut buf = String::new();
        stdin.read_line(&mut buf)?;
        // remove the trailing newline
        if let Some(b'\n') = buf.as_bytes().last() {
            buf.pop();
        }
        Ok(buf.into_bytes())
    } else {
        let mut buf = Vec::new();
        stdin.read_to_end(&mut buf)?;
        Ok(buf)
    }
}
