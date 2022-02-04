//! utility functions

use crate::sputnik::cheatcodes::HevmConsoleEvents;
use ethers::contract::EthLogDecode;
use ethers_core::abi::RawLog;
use sputnik::backend::Log;

/// Converts a log into a formatted string
pub(crate) fn convert_log(log: Log) -> Option<String> {
    use HevmConsoleEvents::*;
    let log = RawLog { topics: log.topics, data: log.data };
    let event = HevmConsoleEvents::decode_log(&log).ok()?;
    let ret = match event {
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

        e => e.to_string(),
    };
    Some(ret)
}
