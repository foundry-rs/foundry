//! Implementations of [`String`](crate::Group::String) cheatcodes.

use super::{Cheatcode, Result};
use crate::{Cheatcodes, Vm::*};
use alloy_primitives::{Address, Bytes, B256, I256, U256};
use alloy_sol_types::{SolType, SolValue};

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
        parse::<Bytes>(stringifiedValue)
    }
}

impl Cheatcode for parseAddressCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse::<Address>(stringifiedValue)
    }
}

impl Cheatcode for parseUintCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse::<U256>(stringifiedValue)
    }
}

impl Cheatcode for parseIntCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse::<I256>(stringifiedValue)
    }
}

impl Cheatcode for parseBytes32Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse::<B256>(stringifiedValue)
    }
}

impl Cheatcode for parseBoolCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { stringifiedValue } = self;
        parse::<bool>(stringifiedValue)
    }
}

pub(super) fn parse<T>(s: &str) -> Result
where
    T: SolValue + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    parse_t::<T>(s).map(|v| v.abi_encode())
}

pub(super) fn parse_array<I, S, T>(values: I) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    T: SolValue + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let mut values = values.into_iter();
    match values.next() {
        Some(first) if !first.as_ref().is_empty() => std::iter::once(first)
            .chain(values)
            .map(|s| parse_t::<T>(s.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .map(|vec| vec.abi_encode()),
        // return the empty encoded Bytes when values is empty or the first element is empty
        _ => Ok("".abi_encode()),
    }
}

fn parse_t<T>(s: &str) -> Result<T>
where
    T: SolValue + std::str::FromStr,
    T::Err: std::fmt::Display,
{
    s.parse::<T>().map_err(|e| {
        fmt_err!("failed parsing {s:?} as type `{}`: {e}", T::SolType::sol_type_name())
    })
}
