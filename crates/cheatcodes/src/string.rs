//! Implementations of [`String`](spec::Group::String) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{hex, U256};
use alloy_sol_types::SolValue;

// address
impl Cheatcode for toString_0Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

// bytes
impl Cheatcode for toString_1Call { 
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

// bytes32
impl Cheatcode for toString_2Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

// bool
impl Cheatcode for toString_3Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

// uint256
impl Cheatcode for toString_4Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

// int256
impl Cheatcode for toString_5Call {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { value } = self;
        Ok(value.to_string())
    }
}

impl Cheatcode for parseBytesCall {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Bytes)
    }
}

impl Cheatcode for parseAddressCall {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Address)
    }
}

impl Cheatcode for parseUintCall {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Uint(256))
    }
}

impl Cheatcode for parseIntCall {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Int(256))
    }
}

impl Cheatcode for parseBytes32Call {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::FixedBytes(32))
    }
}

impl Cheatcode for parseBoolCall {
    type Return = Vec<u8>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { stringifiedValue } = self;
        parse(stringifiedValue, &DynSolType::Bool)
    }
}

impl Cheatcode for toLowercaseCall {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input } = self;
        Ok(input.to_lowercase())
    }
}

impl Cheatcode for toUppercaseCall {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input } = self;
        Ok(input.to_uppercase())
    }
}

impl Cheatcode for trimCall {
    type Return = String; // TODO: fix

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input } = self;
        Ok(input.trim())
    }
}

impl Cheatcode for replaceCall {
    type Return = String;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input, from, to } = self;
        Ok(input.replace(from, to))
    }
}

impl Cheatcode for splitCall {
    type Return = Vec<String>;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input, delimiter } = self;
        let parts: Vec<&str> = input.split(delimiter).collect();
        Ok(parts.into_iter().map(|s| s.to_string()).collect())
    }
}

impl Cheatcode for indexOfCall {
    type Return = U256;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { input, key } = self;
        Ok(input.find(key).map(U256::from).unwrap_or(U256::MAX))
    }
}

impl Cheatcode for containsCall {
    type Return = bool;

    fn apply(&self, _state: &mut Cheatcodes) -> Result<<Self as Cheatcode>::Return> {
        let Self { subject, search } = self;
        Ok(subject.contains(search))
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
pub(super) fn parse_value(s: &str, ty: &DynSolType) -> Result<DynSolValue> {
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
            if !s.starts_with("0x") && hex::check_raw(s) {
                return Some(Err("missing hex prefix (\"0x\") for hex string"));
            }
        }
        _ => {}
    }
    None
}
