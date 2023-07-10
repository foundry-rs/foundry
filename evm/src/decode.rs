//! Various utilities to decode test results
use crate::{
    abi::ConsoleEvents::{self, *},
    executor::inspector::cheatcodes::util::MAGIC_SKIP_BYTES,
};
use ethers::{
    abi::{decode, AbiDecode, Contract as Abi, ParamType, RawLog, Token},
    contract::EthLogDecode,
    prelude::U256,
    types::Log,
};
use foundry_common::{abi::format_token, SELECTOR_LEN};
use foundry_utils::error::ERROR_PREFIX;
use itertools::Itertools;
use once_cell::sync::Lazy;
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

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(
    err: &[u8],
    maybe_abi: Option<&Abi>,
    status: Option<InstructionResult>,
) -> eyre::Result<String> {
    if err.len() < SELECTOR_LEN {
        if let Some(status) = status {
            if !matches!(status, return_ok!()) {
                return Ok(format!("EvmError: {status:?}"))
            }
        }
        eyre::bail!("Not enough error data to decode")
    }
    match err[..SELECTOR_LEN] {
        // keccak(Panic(uint256))
        [78, 72, 123, 113] => {
            // ref: https://soliditydeveloper.com/solidity-0.8
            match err[err.len() - 1] {
                1 => {
                    // assert
                    Ok("Assertion violated".to_string())
                }
                17 => {
                    // safemath over/underflow
                    Ok("Arithmetic over/underflow".to_string())
                }
                18 => {
                    // divide by 0
                    Ok("Division or modulo by 0".to_string())
                }
                33 => {
                    // conversion into non-existent enum type
                    Ok("Conversion into non-existent enum type".to_string())
                }
                34 => {
                    // incorrectly encoded storage byte array
                    Ok("Incorrectly encoded storage byte array".to_string())
                }
                49 => {
                    // pop() on empty array
                    Ok("`pop()` on empty array".to_string())
                }
                50 => {
                    // index out of bounds
                    Ok("Index out of bounds".to_string())
                }
                65 => {
                    // allocating too much memory or creating too large array
                    Ok("Memory allocation overflow".to_string())
                }
                81 => {
                    // calling a zero initialized variable of internal function type
                    Ok("Calling a zero initialized variable of internal function type".to_string())
                }
                _ => {
                    eyre::bail!("Unsupported solidity builtin panic")
                }
            }
        }
        // keccak(Error(string))
        [8, 195, 121, 160] => {
            String::decode(&err[SELECTOR_LEN..]).map_err(|_| eyre::eyre!("Bad string decode"))
        }
        // keccak(expectRevert(bytes))
        [242, 141, 206, 179] => {
            let err_data = &err[SELECTOR_LEN..];
            if err_data.len() > 64 {
                let len = U256::from(&err_data[32..64]).as_usize();
                if err_data.len() > 64 + len {
                    let actual_err = &err_data[64..64 + len];
                    if let Ok(decoded) = decode_revert(actual_err, maybe_abi, None) {
                        // check if it's a builtin
                        return Ok(decoded)
                    } else if let Ok(as_str) = String::from_utf8(actual_err.to_vec()) {
                        // check if it's a true string
                        return Ok(as_str)
                    }
                }
            }
            eyre::bail!("Non-native error and not string")
        }
        // keccak(expectRevert(bytes4))
        [195, 30, 176, 224] => {
            let err_data = &err[SELECTOR_LEN..];
            if err_data.len() == 32 {
                let actual_err = &err_data[..SELECTOR_LEN];
                if let Ok(decoded) = decode_revert(actual_err, maybe_abi, None) {
                    // it's a known selector
                    return Ok(decoded)
                }
            }
            eyre::bail!("Unknown error selector")
        }
        _ => {
            // See if the revert is caused by a skip() call.
            if err == MAGIC_SKIP_BYTES {
                return Ok("SKIPPED".to_string())
            }
            // try to decode a custom error if provided an abi
            if let Some(abi) = maybe_abi {
                for abi_error in abi.errors() {
                    if abi_error.signature()[..SELECTOR_LEN] == err[..SELECTOR_LEN] {
                        // if we don't decode, don't return an error, try to decode as a
                        // string later
                        if let Ok(decoded) = abi_error.decode(&err[SELECTOR_LEN..]) {
                            let inputs = decoded
                                .iter()
                                .map(foundry_common::abi::format_token)
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Ok(format!("{}({inputs})", abi_error.name))
                        }
                    }
                }
            }
            // optimistically try to decode as string, unknown selector or `CheatcodeError`
            String::decode(err)
                .ok()
                .or_else(|| {
                    // try decoding as cheatcode error
                    if err.starts_with(ERROR_PREFIX.as_slice()) {
                        String::decode(&err[ERROR_PREFIX.len()..]).ok()
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    // try decoding as unknown err
                    String::decode(&err[SELECTOR_LEN..])
                        .map(|err_str| format!("{}:{err_str}", hex::encode(&err[..SELECTOR_LEN])))
                        .ok()
                })
                .or_else(|| {
                    // try to decode possible variations of custom error types
                    decode_custom_error(err).map(|token| {
                        let s = format!("Custom Error {}:", hex::encode(&err[..SELECTOR_LEN]));

                        let err_str = format_token(&token);
                        if err_str.starts_with('(') {
                            format!("{s}{err_str}")
                        } else {
                            format!("{s}({err_str})")
                        }
                    })
                })
                .ok_or_else(|| eyre::eyre!("Non-native error and not string"))
        }
    }
}

/// Tries to optimistically decode a custom solc error, with at most 4 arguments
pub fn decode_custom_error(err: &[u8]) -> Option<Token> {
    decode_custom_error_args(err, 4)
}

/// Tries to optimistically decode a custom solc error with a maximal amount of arguments
///
/// This will brute force decoding of custom errors with up to `args` arguments
pub fn decode_custom_error_args(err: &[u8], args: usize) -> Option<Token> {
    if err.len() <= SELECTOR_LEN {
        return None
    }

    let err = &err[SELECTOR_LEN..];
    /// types we check against
    static TYPES: Lazy<Vec<ParamType>> = Lazy::new(|| {
        vec![
            ParamType::Address,
            ParamType::Bool,
            ParamType::Uint(256),
            ParamType::Int(256),
            ParamType::Bytes,
            ParamType::String,
        ]
    });

    macro_rules! try_decode {
        ($ty:ident) => {
            if let Ok(mut decoded) = decode(&[$ty], err) {
                return Some(decoded.remove(0))
            }
        };
    }

    // check if single param, but only if it's a single word
    if err.len() == 32 {
        for ty in TYPES.iter().cloned() {
            try_decode!(ty);
        }
        return None
    }

    // brute force decode all possible combinations
    for num in (2..=args).rev() {
        for candidate in TYPES.iter().cloned().combinations(num) {
            if let Ok(decoded) = decode(&candidate, err) {
                return Some(Token::Tuple(decoded))
            }
        }
    }

    // try as array
    for ty in TYPES.iter().cloned().map(|ty| ParamType::Array(Box::new(ty))) {
        try_decode!(ty);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::{
        abi::{AbiEncode, Address},
        contract::EthError,
    };

    #[test]
    fn test_decode_custom_error_address() {
        #[derive(Debug, Clone, EthError)]
        struct AddressErr(Address);
        let err = AddressErr(Address::random());

        let encoded = err.clone().encode();
        let decoded = decode_custom_error(&encoded).unwrap();
        assert_eq!(decoded, Token::Address(err.0));
    }

    #[test]
    fn test_decode_custom_error_args3() {
        #[derive(Debug, Clone, EthError)]
        struct MyError(Address, bool, U256);
        let err = MyError(Address::random(), true, 100u64.into());

        let encoded = err.clone().encode();
        let decoded = decode_custom_error(&encoded).unwrap();
        assert_eq!(
            decoded,
            Token::Tuple(vec![
                Token::Address(err.0),
                Token::Bool(err.1),
                Token::Uint(100u64.into()),
            ])
        );
    }
}
