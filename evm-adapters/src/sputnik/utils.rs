//! utility functions

use crate::sputnik::cheatcodes::HevmConsoleEvents;
use ethers::contract::EthLogDecode;
use ethers_core::{
    abi::{RawLog, Token},
    types::{Address, H160},
};
use sputnik::{backend::Log, Capture, ExitReason, ExitRevert, ExitSucceed};
use std::convert::Infallible;

/// For certain cheatcodes, we may internally change the status of the call, i.e. in
/// `expectRevert`. Solidity will see a successful call and attempt to abi.decode for the called
/// function. Therefore, we need to populate the return with dummy bytes such that the decode
/// doesn't fail
pub const DUMMY_OUTPUT: [u8; 320] = [0u8; 320];

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

/// Wrapper around both return types for expectRevert in call or create
pub enum ExpectRevertReturn {
    Call(Capture<(ExitReason, Vec<u8>), Infallible>),
    Create(Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible>),
}

impl ExpectRevertReturn {
    pub fn into_call_inner(self) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        match self {
            ExpectRevertReturn::Call(inner) => inner,
            _ => panic!("tried to get call response inner from a create"),
        }
    }
    pub fn into_create_inner(self) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        match self {
            ExpectRevertReturn::Create(inner) => inner,
            _ => panic!("tried to get create response inner from a call"),
        }
    }

    pub fn is_call(&self) -> bool {
        matches!(self, ExpectRevertReturn::Call(..))
    }
}

// helper for creating an exit type
pub fn evm_error(retdata: &str) -> Capture<(ExitReason, Vec<u8>), Infallible> {
    Capture::Exit((
        ExitReason::Revert(ExitRevert::Reverted),
        ethers::abi::encode(&[Token::String(retdata.to_string())]),
    ))
}

// helper for creating the Expected Revert return type, based on if there was a call or a create,
// and if there was any decoded retdata that matched the expected revert value.
pub fn revert_return_evm<T: ToString>(
    call: bool,
    result: Option<(&[u8], &[u8])>,
    err: impl FnOnce() -> T,
) -> ExpectRevertReturn {
    let success =
        result.map(|(retdata, expected_revert)| retdata == expected_revert).unwrap_or(false);

    match (success, call) {
        // Success case for CALLs needs to return a dummy output value which
        // can be decoded
        (true, true) => ExpectRevertReturn::Call(Capture::Exit((
            ExitReason::Succeed(ExitSucceed::Returned),
            DUMMY_OUTPUT.to_vec(),
        ))),
        // Success case for CREATE doesn't need to return any value but must return a
        // dummy address
        (true, false) => ExpectRevertReturn::Create(Capture::Exit((
            ExitReason::Succeed(ExitSucceed::Returned),
            Some(Address::from_str("0000000000000000000000000000000000000001").unwrap()),
            Vec::new(),
        ))),
        // Failure cases just return the abi encoded error
        (false, true) => ExpectRevertReturn::Call(Capture::Exit((
            ExitReason::Revert(ExitRevert::Reverted),
            ethers::abi::encode(&[Token::String(err().to_string())]),
        ))),
        (false, false) => ExpectRevertReturn::Create(Capture::Exit((
            ExitReason::Revert(ExitRevert::Reverted),
            None,
            ethers::abi::encode(&[Token::String(err().to_string())]),
        ))),
    }
}
