//! Various utilities to decode test results
use crate::{
    abi::ConsoleEvents::{self, *},
    error::DecodedError,
};
use ethers::{
    abi::{Abi, AbiDecode, RawLog},
    contract::EthLogDecode,
    types::Log,
};
use foundry_common::SELECTOR_LEN;
use revm::Return;

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
            format!("{}: 0x{}", inner.key, hex::encode(inner.val))
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
            format!("{}: 0x{}", inner.key, hex::encode(inner.val))
        }
        LogNamedStringFilter(inner) => format!("{}: {}", inner.key, inner.val),
        LogNamedArray1Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedArray2Filter(inner) => format!("{}: {:?}", inner.key, inner.val),
        LogNamedArray3Filter(inner) => format!("{}: {:?}", inner.key, inner.val),

        e => e.to_string(),
    };
    Some(decoded)
}

/// Decodes a revert given some error data, the final status of the VM, and optionally an ABI.
pub fn decode_revert(
    data: &[u8],
    abi: Option<&Abi>,
    status: Option<Return>,
) -> eyre::Result<DecodedError> {
    if data.len() < SELECTOR_LEN {
        if status.map_or(false, |status| !matches!(status, revm::return_ok!())) {
            return Ok(DecodedError {
                message: format!("EvmError: {:?}", status.unwrap()),
                hints: Vec::new(),
            })
        }

        eyre::bail!("Not enough data to decode error.")
    }

    (if let Some(abi) = abi {
        DecodedError::decode_with_abi(data, abi)
    } else {
        DecodedError::decode(data)
    })
    .map_err(|_| eyre::eyre!("Could not decode error."))
}
