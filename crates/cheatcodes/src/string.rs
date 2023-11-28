//! Implementations of [`String`](crate::Group::String) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_sol_types::SolValue;

// address
impl Cheatcode for toString_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(value.to_string().abi_encode())
    }
}

// bytes
impl Cheatcode for toString_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(hex::encode_prefixed(value).abi_encode())
    }
}

// bytes32
impl Cheatcode for toString_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(value.to_string().abi_encode())
    }
}

// bool
impl Cheatcode for toString_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(value.to_string().abi_encode())
    }
}

// uint256
impl Cheatcode for toString_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(value.to_string().abi_encode())
    }
}

// int256
impl Cheatcode for toString_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { value } = self;
        Ok(value.to_string().abi_encode())
    }
}

impl Cheatcode for parseBytesCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Address)
    }
}

impl Cheatcode for parseUintCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseIntCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseBytes32Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseBoolCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Bool)
    }
}

pub(super) fn parse(s: &str, ty: &DynSolType) -> Result {
    parse_value(s, ty).map(|v| v.abi_encode())
}

pub(super) fn parse_array<I, S>(values: I, ty: &DynSolType) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values = values.into_iter();
    match values.next() {
        Some(first) if !first.as_ref().is_empty() => std::iter::once(first)
            .chain(values)
            .map(|s| parse_value(s.as_ref(), ty))
            .collect::<Result<Vec<_>, _>>()
            .map(|vec| DynSolValue::Array(vec).abi_encode()),
        // return the empty encoded Bytes when values is empty or the first element is empty
        _ => Ok("".abi_encode()),
    }
}

#[instrument(target = "cheatcodes", level = "debug", skip(ty), fields(%ty), ret)]
fn parse_value(s: &str, ty: &DynSolType) -> Result<DynSolValue> {
    match ty.coerce_str(s) {
        Ok(value) => Ok(value),
        Err(e) => match parse_value_fallback(s, ty) {
            Some(Ok(value)) => Ok(value),
            Some(Err(e2)) => Err(fmt_err!("failed parsing {s:?} as type `{ty}`: {e2}")),
            None => Err(fmt_err!("failed parsing {s:?} as type `{ty}`: {e}")),
        },
    }
}

// More lenient parsers than `coerce_str`.
fn parse_value_fallback(s: &str, ty: &DynSolType) -> Option<Result<DynSolValue, &'static str>> {
    match ty {
        DynSolType::Bool => {
            let b = match s {
                "1" => true,
                "0" => false,
                s if s.eq_ignore_ascii_case("true") => true,
                s if s.eq_ignore_ascii_case("false") => false,
                _ => return None,
            };
            return Some(Ok(DynSolValue::Bool(b)));
        }
        DynSolType::Int(_) |
        DynSolType::Uint(_) |
        DynSolType::FixedBytes(_) |
        DynSolType::Bytes => {
            if !s.starts_with("0x") && s.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(Err("missing hex prefix (\"0x\") for hex string"))
            }
        }
        _ => {}
    }
    None
}
