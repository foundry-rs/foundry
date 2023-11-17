//! Various utilities to decode test results.

use crate::abi::ConsoleEvents;
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::JsonAbi;
use alloy_primitives::B256;
use alloy_sol_types::{SolCall, SolError, SolInterface, SolValue};
use ethers_contract::EthLogDecode;
use ethers_core::{abi::RawLog, types::Log, utils::format_units};
use foundry_cheatcodes_spec::Vm;
use foundry_common::SELECTOR_LEN;
use itertools::Itertools;
use revm::interpreter::InstructionResult;

/// Decode a set of logs, only returning logs from DSTest logging events and Hardhat's `console.log`
pub fn decode_console_logs(logs: &[Log]) -> Vec<String> {
    logs.iter().filter_map(decode_console_log).collect()
}

/// Decode a single log.
///
/// This function returns [None] if it is not a DSTest log or the result of a Hardhat
/// `console.log`.
pub fn decode_console_log(log: &Log) -> Option<String> {
    use ConsoleEvents as CE;

    // NOTE: We need to do this conversion because ethers-rs does not
    // support passing `Log`s
    let raw_log = RawLog { topics: log.topics.clone(), data: log.data.to_vec() };
    let decoded = match ConsoleEvents::decode_log(&raw_log).ok()? {
        CE::LogsFilter(inner) => format!("{}", inner.0),
        CE::LogBytesFilter(inner) => format!("{}", inner.0),
        CE::LogNamedAddressFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        CE::LogNamedBytes32Filter(inner) => {
            format!("{}: {}", inner.key, B256::new(inner.val))
        }
        CE::LogNamedDecimalIntFilter(inner) => {
            let (sign, val) = inner.val.into_sign_and_abs();
            format!(
                "{}: {}{}",
                inner.key,
                sign,
                format_units(val, inner.decimals.as_u32()).unwrap()
            )
        }
        CE::LogNamedDecimalUintFilter(inner) => {
            format!("{}: {}", inner.key, format_units(inner.val, inner.decimals.as_u32()).unwrap())
        }
        CE::LogNamedIntFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        CE::LogNamedUintFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
        CE::LogNamedBytesFilter(inner) => {
            format!("{}: {}", inner.key, inner.val)
        }
        CE::LogNamedStringFilter(inner) => format!("{}: {}", inner.key, inner.val),
        CE::LogNamedArray1Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        CE::LogNamedArray2Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        CE::LogNamedArray3Filter(inner) => format!("{}: {:?}", inner.key, inner.val),

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
    maybe_decode_revert(err, maybe_abi, status).unwrap_or_else(|| {
        if err.is_empty() {
            "<no data>".to_string()
        } else {
            trimmed_hex(err)
        }
    })
}

pub fn maybe_decode_revert(
    err: &[u8],
    maybe_abi: Option<&JsonAbi>,
    status: Option<InstructionResult>,
) -> Option<String> {
    if err.len() < SELECTOR_LEN {
        if let Some(status) = status {
            if !status.is_ok() {
                return Some(format!("EvmError: {status:?}"));
            }
        }
        return if err.is_empty() {
            None
        } else {
            Some(format!("custom error bytes {}", hex::encode_prefixed(err)))
        }
    }

    if err == crate::constants::MAGIC_SKIP {
        // Also used in forge fuzz runner
        return Some("SKIPPED".to_string());
    }

    // Solidity's `Error(string)` or `Panic(uint256)`
    if let Ok(e) = alloy_sol_types::GenericContractError::abi_decode(err, false) {
        return Some(e.to_string());
    }

    let (selector, data) = err.split_at(SELECTOR_LEN);
    let selector: &[u8; 4] = selector.try_into().unwrap();

    match *selector {
        // `CheatcodeError(string)`
        Vm::CheatcodeError::SELECTOR => {
            let e = Vm::CheatcodeError::abi_decode_raw(data, false).ok()?;
            return Some(e.message);
        }
        // `expectRevert(bytes)`
        Vm::expectRevert_2Call::SELECTOR => {
            let e = Vm::expectRevert_2Call::abi_decode_raw(data, false).ok()?;
            return maybe_decode_revert(&e.revertData[..], maybe_abi, status);
        }
        // `expectRevert(bytes4)`
        Vm::expectRevert_1Call::SELECTOR => {
            let e = Vm::expectRevert_1Call::abi_decode_raw(data, false).ok()?;
            return maybe_decode_revert(&e.revertData[..], maybe_abi, status);
        }
        _ => {}
    }

    // Custom error from the given ABI
    if let Some(abi) = maybe_abi {
        if let Some(abi_error) = abi.errors().find(|e| selector == e.selector()) {
            // if we don't decode, don't return an error, try to decode as a string later
            if let Ok(decoded) = abi_error.abi_decode_input(data, false) {
                return Some(format!(
                    "{}({})",
                    abi_error.name,
                    decoded.iter().map(foundry_common::fmt::format_token).format(", ")
                ));
            }
        }
    }

    // ABI-encoded `string`
    if let Ok(s) = String::abi_decode(err, false) {
        return Some(s);
    }

    // UTF-8-encoded string
    if let Ok(s) = std::str::from_utf8(err) {
        return Some(s.to_string());
    }

    // Generic custom error
    Some(format!(
        "custom error {}:{}",
        hex::encode(selector),
        std::str::from_utf8(data).map_or_else(|_| trimmed_hex(data), String::from)
    ))
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
