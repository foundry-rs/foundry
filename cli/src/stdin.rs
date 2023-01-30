//! Utility functions for reading from [`stdin`](std::io::stdin).

use ethers::types::{BlockId, BlockNumber};
use eyre::Result;
use std::{
    error::Error as StdError,
    io::{self, BufRead, Read},
    str::FromStr,
};

use crate::opts::cast::parse_block_id;

/// Unwraps the given `Option<T>` or [reads stdin into a String](read) and parses it as `T`.
pub fn unwrap<T>(value: Option<T>, read_line: bool) -> eyre::Result<T>
where
    T: FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    match value {
        Some(value) => Ok(value),
        None => read(read_line)?.parse().map_err(Into::into),
    }
}

/// [Reads stdin into a String](read) and parses it as `Vec<T>` using whitespaces as delimiters if
/// the given `Vec<T>` is empty.
pub fn unwrap_vec<T>(mut value: Vec<T>) -> eyre::Result<Vec<T>>
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

// TODO: Use [BlockId::from_str]
pub fn unwrap_block_id(block: Option<BlockId>) -> Result<BlockId> {
    match block {
        Some(block) => Ok(block),
        None => {
            let s = read(true)?;
            let block = parse_block_id(&s).unwrap_or(BlockId::Number(BlockNumber::Latest));
            Ok(block)
        }
    }
}

/// Reads bytes from [`stdin`][io::stdin] into a String.
///
/// If `read_line` is true, stop at the first newline (the `0xA` byte).
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
        buf.pop();
        Ok(buf.into_bytes())
    } else {
        let mut buf = Vec::new();
        stdin.read_to_end(&mut buf)?;
        Ok(buf)
    }
}
