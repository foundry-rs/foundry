//! Various utilities to decode test results.

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::JsonAbi;
use alloy_primitives::B256;
use alloy_sol_types::SolCall;
use ethers::{abi::RawLog, contract::EthLogDecode, types::Log};
use foundry_abi::console::ConsoleEvents::{self, *};
use foundry_cheatcodes_defs::Vm;
use foundry_common::SELECTOR_LEN;
use itertools::Itertools;
use revm::interpreter::{return_ok, InstructionResult};
use thiserror::Error;

/// Decode a set of logs, only returning logs from DSTest logging events and Hardhat's `console.log`
pub fn decode_console_logs(logs: &[Log]) -> Vec<String> {
    logs.iter().filter_map(decode_console_log).collect()
}

/// Decode a single log.
///
/// This function returns [None] if it is not a DSTest log or the result of a Hardhat
/// `console.log`.
pub fn decode_console_log(log: &Log) -> Option<String> {
    // NOTE: We need to do this conversion because ethers-rs does not
    // support passing `Log`s
    let raw_log = RawLog { topics: log.topics.clone(), data: log.data.to_vec() };
    let decoded = match ConsoleEvents::decode_log(&raw_log).ok()? {
        LogsFilter(inner) => format!("{}", inner.0),
        LogBytesFilter(inner) => format!("{}", inner.0),
        LogNamedAddressFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedBytes32Filter(inner) => {
            format!("{}: {}", inner.key, B256::new(inner.val))
        }
        LogNamedDecimalIntFilter(inner) => {
            let (sign, val) = inner.val.into_sign_and_abs();
            format!(
                "{}: {}{}",
                inner.key,
                sign,
                ethers::utils::format_units(val, inner.decimals.as_u32()).unwrap()
            )
        }
        LogNamedDecimalUintFilter(inner) => {
            format!(
                "{}: {}",
                inner.key,
                ethers::utils::format_units(inner.val, inner.decimals.as_u32()).unwrap()
            )
        }
        LogNamedIntFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedUintFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedBytesFilter(inner) => {
            format!("{}: {}", inner.key, inner.val)
        }
        LogNamedStringFilter(inner) => format!("{}: {}", inner.key, inner.val),
        LogNamedArray1Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedArray2Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedArray3Filter(inner) => format!("{}: {:?}", inner.key, inner.val),

        e => e.to_string(),
    };
    Some(decoded)
}

/// Possible errors when decoding a revert error string.
#[derive(Debug, Clone, Error)]
pub enum RevertDecodingError {
    #[error("Not enough data to decode")]
    InsufficientErrorData,
    #[error("Unsupported solidity builtin panic")]
    UnsupportedSolidityBuiltinPanic,
    #[error("Could not decode slice")]
    SliceDecodingError,
    #[error("Non-native error and not string")]
    NonNativeErrorAndNotString,
    #[error("Unknown Error Selector")]
    UnknownErrorSelector,
    #[error("Could not decode cheatcode string")]
    UnknownCheatcodeErrorString,
    #[error("Bad String decode")]
    BadStringDecode,
    #[error(transparent)]
    AlloyDecodingError(alloy_dyn_abi::Error),
}

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(
    err: &[u8],
    maybe_abi: Option<&JsonAbi>,
    status: Option<InstructionResult>,
) -> String {
    if err.len() < SELECTOR_LEN {
        if let Some(status) = status {
            if !matches!(status, return_ok!()) {
                return format!("EvmError: {status:?}")
            }
        }
    }

    // `expectRevert(bytes)`
    if let Ok(e) = Vm::expectRevert_2Call::abi_decode(err, true) {
        return decode_revert(&e.revertData[..], maybe_abi, status)
    }

    // `expectRevert(bytes4)`
    if let Ok(e) = Vm::expectRevert_1Call::abi_decode(err, true) {
        return decode_revert(&e.revertData[..], maybe_abi, status)
    }

    // try to decode a custom error if provided an abi
    if let Some(abi) = maybe_abi {
        for abi_error in abi.errors() {
            if abi_error.selector()[..SELECTOR_LEN] == err[..SELECTOR_LEN] {
                // if we don't decode, don't return an error, try to decode as a string later
                if let Ok(decoded) = abi_error.abi_decode_input(&err[SELECTOR_LEN..], false) {
                    let inputs = decoded
                        .iter()
                        .map(foundry_common::fmt::format_token)
                        .collect::<Vec<_>>()
                        .join(", ");
                    return format!("{}({inputs})", abi_error.name)
                }
            }
        }
    }

    // `string`
    if let Ok(s) = std::str::from_utf8(err) {
        return s.to_string()
    }

    // Generic custom error
    let (selector, err) = err.split_at(SELECTOR_LEN);
    format!(
        "Custom error {}:{}",
        hex::encode(selector),
        std::str::from_utf8(err).map_or_else(|_| trimmed_hex(err), String::from)
    )
}

fn trimmed_hex(s: &[u8]) -> String {
    let s = hex::encode(s);
    if s.len() <= 32 {
        s
    } else {
        format!("{}…{} ({} bytes)", &s[..16], &s[s.len() - 16..], s.len())
    }
}

/// Tries to optimistically decode a custom solc error, with at most 4 arguments
pub fn decode_custom_error(err: &[u8]) -> Option<DynSolValue> {
    decode_custom_error_args(err, 4)
}

/// Tries to optimistically decode a custom solc error with a maximal amount of arguments
///
/// This will brute force decoding of custom errors with up to `args` arguments
pub fn decode_custom_error_args(err: &[u8], args: usize) -> Option<DynSolValue> {
    if err.len() <= SELECTOR_LEN {
        return None
    }

    let err = &err[SELECTOR_LEN..];
    /// types we check against
    static TYPES: &[DynSolType] = &[
        DynSolType::Address,
        DynSolType::Bool,
        DynSolType::Uint(256),
        DynSolType::Int(256),
        DynSolType::Bytes,
        DynSolType::String,
    ];

    // check if single param, but only if it's a single word
    if err.len() == 32 {
        for ty in TYPES.iter() {
            if let Ok(decoded) = ty.abi_decode(err) {
                return Some(decoded)
            }
        }
        return None
    }

    // brute force decode all possible combinations
    for num in (2..=args).rev() {
        for candidate in TYPES.iter().cloned().combinations(num) {
            if let Ok(decoded) = DynSolType::abi_decode(&DynSolType::Tuple(candidate), err) {
                return Some(decoded)
            }
        }
    }

    // try as array
    for ty in TYPES.iter().cloned().map(|ty| DynSolType::Array(Box::new(ty))) {
        if let Ok(decoded) = ty.abi_decode(err) {
            return Some(decoded)
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::{sol, SolError};

    #[test]
    fn test_decode_custom_error_address() {
        sol! {
            error AddressErr(address addr);
        }
        let err = AddressErr { addr: Address::random() };

        let encoded = err.abi_encode();
        let decoded = decode_custom_error(&encoded).unwrap();
        assert_eq!(decoded, DynSolValue::Address(err.addr));
    }

    #[test]
    fn test_decode_custom_error_args3() {
        sol! {
            error MyError(address addr, bool b, uint256 val);
        }
        let err = MyError { addr: Address::random(), b: true, val: U256::from(100u64) };

        let encoded = err.clone().abi_encode();
        let decoded = decode_custom_error(&encoded).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::Tuple(vec![
                DynSolValue::Address(err.addr),
                DynSolValue::Bool(err.b),
                DynSolValue::Uint(U256::from(100u64), 256),
            ])
        );
    }
}
