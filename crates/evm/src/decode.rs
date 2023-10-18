//! Various utilities to decode test results
use crate::{
    abi::ConsoleEvents::{self, *},
    executor::inspector::cheatcodes::util::MAGIC_SKIP_BYTES,
};
use alloy_dyn_abi::{JsonAbiExt, DynSolValue, DynSolType};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{B256, Bytes};
use alloy_primitives::{Log as AlloyLog};
use alloy_sol_types::{sol, SolEvent};
use ethers::{
    abi::{decode, AbiDecode, Contract as Abi, ParamType, RawLog, Token},
    contract::EthLogDecode,
    prelude::U256,
    types::Log,
};
use foundry_common::{abi::format_token, SELECTOR_LEN};
use foundry_utils::{error::ERROR_PREFIX, types::ToAlloy};
use itertools::Itertools;
use once_cell::sync::Lazy;
use revm::interpreter::{return_ok, InstructionResult};

sol! {
    event log(string);
    event logs                   (bytes);
    event log_address            (address);
    event log_bytes32            (bytes32);
    event log_int                (int);
    event log_uint               (uint);
    event log_bytes              (bytes);
    event log_string             (string);
    event log_array              (uint256[] val);
    event log_array              (int256[] val);
    event log_array              (address[] val);
    event log_named_address      (string key, address val);
    event log_named_bytes32      (string key, bytes32 val);
    event log_named_decimal_int  (string key, int val, uint decimals);
    event log_named_decimal_uint (string key, uint val, uint decimals);
    event log_named_int          (string key, int val);
    event log_named_uint         (string key, uint val);
    event log_named_bytes        (string key, bytes val);
    event log_named_string       (string key, string val);
    event log_named_array        (string key, uint256[] val);
    event log_named_array        (string key, int256[] val);
    event log_named_array        (string key, address[] val);
}

/// Decode a set of logs, only returning logs from DSTest logging events and Hardhat's `console.log`
pub fn decode_console_logs(logs: &[Log]) -> Vec<String> {
    logs.iter().filter_map(decode_console_log).collect()
}

/// Decode a single log.
///
/// This function returns [None] if it is not a DSTest log or the result of a Hardhat
/// `console.log`.
pub fn decode_console_log(log: &Log) -> Option<String> {
    let raw_log = AlloyLog::new_unchecked(log.topics.into_iter().map(|h| h.to_alloy()).collect_vec(), log.data.0.into());
    let data = log_address::abi_decode_data(&raw_log.data, false);
    if let Ok(inner) = log::abi_decode_data(&raw_log.data, false) {
        return Some(inner.0)
    } else if let Ok(inner) = logs::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", Bytes::from(inner.0)))
    } else if let Ok(inner) = log_address::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", inner.0))
    } else if let Ok(inner) = log_bytes32::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", inner.0))
    } else if let Ok(inner) = log_int::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", inner.0))
    } else if let Ok(inner) = log_uint::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", inner.0))
    } else if let Ok(inner) = log_bytes::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", Bytes::from(inner.0)))
    } else if let Ok(inner) = log_string::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}", inner.0))
    } else if let Ok(inner) = log_array_0::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{:?}", inner.0))
    } else if let Ok(inner) = log_array_1::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{:?}", inner.0))
    } else if let Ok(inner) = log_array_2::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{:?}", inner.0))
    } else if let Ok(inner) = log_named_address::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_bytes32::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_decimal_int::abi_decode_data(&raw_log.data, false) {
        let (sign, val) = inner.1.into_sign_and_abs();
        // TODO: Format units
        return Some(format!("{}: {}{}", inner.0, sign, val))
    } else if let Ok(inner) = log_named_decimal_uint::abi_decode_data(&raw_log.data, false) {
        // TODO: Format units
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_int::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_uint::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_bytes::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, Bytes::from(inner.1)))
    } else if let Ok(inner) = log_named_string::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_array_0::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {:?}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_array_1::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {:?}", inner.0, inner.1))
    } else if let Ok(inner) = log_named_array_2::abi_decode_data(&raw_log.data, false) {
        return Some(format!("{}: {:?}", inner.0, inner.1))
    }

    return None
}

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(
    err: &[u8],
    maybe_abi: Option<&JsonAbi>,
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
                    if abi_error.signature()[..SELECTOR_LEN].as_bytes() == &err[..SELECTOR_LEN] {
                        // if we don't decode, don't return an error, try to decode as a
                        // string later
                        if let Ok(decoded) = abi_error.abi_decode_input(&err[SELECTOR_LEN..], false) {
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
    static TYPES: Lazy<Vec<DynSolType>> = Lazy::new(|| {
        vec![
            DynSolType::Address,
            DynSolType::Bool,
            DynSolType::Uint(256),
            DynSolType::Int(256),
            DynSolType::Bytes,
            DynSolType::String,
        ]
    });

    macro_rules! try_decode {
        ($ty:ident) => {
            if let Ok(mut decoded) = DynSolType::abi_decode(&[$ty], err) {
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
            if let Ok(decoded) = DynSolType::abi_decode_sequence( &DynSolType::Tuple(candidate), err) {
                return Some(decoded)
            }
        }
    }

    // try as array
    for ty in TYPES.iter().cloned().map(|ty| DynSolType::Array(Box::new(ty))) {
        try_decode!(ty);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256};

    #[test]
    fn test_decode_custom_error_address() {
        #[derive(Debug, Clone, EthError)]
        struct AddressErr(Address);
        let err = AddressErr(Address::random());

        let encoded = err.clone().encode();
        let decoded = decode_custom_error(&encoded).unwrap();
        assert_eq!(decoded, DynSolType::Address(err.0));
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
            DynSolValue::Tuple(vec![
                DynSolValue::Address(err.0),
                DynSolValue::Bool(err.1),
                DynSolValue::Uint(U256::from(100u64)),
            ])
        );
    }
}
