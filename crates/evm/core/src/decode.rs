//! Various utilities to decode test results.

use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::JsonAbi;
use alloy_primitives::B256;
use alloy_sol_types::{SolCall, SolError, SolInterface, SolValue};
use ethers::{abi::RawLog, contract::EthLogDecode, types::Log};
use foundry_abi::console::ConsoleEvents::{self, *};
use foundry_cheatcodes_defs::Vm;
use foundry_common::SELECTOR_LEN;
use itertools::Itertools;
use revm::interpreter::{return_ok, InstructionResult};

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

/// Tries to decode an error message from the given revert bytes.
///
/// Note that this is just a best-effort guess, and should not be relied upon for anything other
/// than user output.
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
        return format!("custom error bytes {}", hex::encode_prefixed(err))
    }

    if err == crate::constants::MAGIC_SKIP {
        // Also used in forge fuzz runner
        return "SKIPPED".to_string()
    }

    // Solidity's `Error(string)` or `Panic(uint256)`
    if let Ok(e) = alloy_sol_types::GenericContractError::abi_decode(err, false) {
        return e.to_string()
    }

    let (selector, data) = err.split_at(SELECTOR_LEN);
    let selector: &[u8; 4] = selector.try_into().unwrap();

    match *selector {
        // `CheatcodeError(string)`
        Vm::CheatcodeError::SELECTOR => {
            if let Ok(e) = Vm::CheatcodeError::abi_decode_raw(data, false) {
                return e.message
            }
        }
        // `expectRevert(bytes)`
        Vm::expectRevert_2Call::SELECTOR => {
            if let Ok(e) = Vm::expectRevert_2Call::abi_decode_raw(data, false) {
                return decode_revert(&e.revertData[..], maybe_abi, status)
            }
        }
        // `expectRevert(bytes4)`
        Vm::expectRevert_1Call::SELECTOR => {
            if let Ok(e) = Vm::expectRevert_1Call::abi_decode_raw(data, false) {
                return decode_revert(&e.revertData[..], maybe_abi, status)
            }
        }
        _ => {}
    }

    // Custom error from the given ABI
    if let Some(abi) = maybe_abi {
        if let Some(abi_error) = abi.errors().find(|e| selector == e.selector()) {
            // if we don't decode, don't return an error, try to decode as a string later
            if let Ok(decoded) = abi_error.abi_decode_input(data, false) {
                return format!(
                    "{}({})",
                    abi_error.name,
                    decoded.iter().map(foundry_common::fmt::format_token).format(", ")
                )
            }
        }
    }

    // ABI-encoded `string`
    if let Ok(s) = String::abi_decode(err, false) {
        return s
    }

    // UTF-8-encoded string
    if let Ok(s) = std::str::from_utf8(err) {
        return s.to_string()
    }

    // Generic custom error
    format!(
        "custom error {}:{}",
        hex::encode(selector),
        std::str::from_utf8(data).map_or_else(|_| trimmed_hex(data), String::from)
    )
}

fn trimmed_hex(s: &[u8]) -> String {
    let s = hex::encode(s);
    let n = 32 * 2;
    if s.len() <= n {
        s
    } else {
        format!("{}…{} ({} bytes)", &s[..n / 2], &s[s.len() - n / 2..], s.len())
    }
}
