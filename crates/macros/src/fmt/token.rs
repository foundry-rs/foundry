//! Formatting helpers for [`DynSolValue`]s.

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::hex;
use std::{fmt, fmt::Write};

/// Wrapper that pretty formats a [DynSolValue]
pub struct TokenDisplay<'a>(pub &'a DynSolValue);

impl fmt::Display for TokenDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt_token(f, self.0)
    }
}

/// Recursively formats an ABI token.
fn fmt_token(f: &mut fmt::Formatter, item: &DynSolValue) -> fmt::Result {
    match item {
        DynSolValue::Address(inner) => {
            write!(f, "{}", inner.to_checksum(None))
        }
        // add 0x
        DynSolValue::Bytes(inner) => f.write_str(&hex::encode_prefixed(inner)),
        DynSolValue::FixedBytes(inner, _) => f.write_str(&hex::encode_prefixed(inner)),
        // print as decimal
        DynSolValue::Uint(inner, _) => write!(f, "{inner}"),
        DynSolValue::Int(inner, _) => write!(f, "{}", *inner),
        DynSolValue::Array(tokens) | DynSolValue::FixedArray(tokens) => {
            f.write_char('[')?;
            let mut tokens = tokens.iter().peekable();
            while let Some(token) = tokens.next() {
                fmt_token(f, token)?;
                if tokens.peek().is_some() {
                    f.write_char(',')?
                }
            }
            f.write_char(']')
        }
        DynSolValue::Tuple(tokens) => {
            f.write_char('(')?;
            let mut tokens = tokens.iter().peekable();
            while let Some(token) = tokens.next() {
                fmt_token(f, token)?;
                if tokens.peek().is_some() {
                    f.write_char(',')?
                }
            }
            f.write_char(')')
        }
        DynSolValue::String(inner) => write!(f, "{:?}", inner),
        DynSolValue::Bool(inner) => write!(f, "{}", inner),
        _ => write!(f, "{item:?}"),
    }
}
