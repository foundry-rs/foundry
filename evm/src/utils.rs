use bytes::Bytes;
use ethers::{
    abi::{self, Abi},
    prelude::{H256, U256},
};
use revm::{opcode, spec_opcode_gas, SpecId};
use std::collections::BTreeMap;

/// Small helper function to convert [U256] into [H256].
pub fn u256_to_h256_le(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_little_endian(h.as_mut());
    h
}

/// Small helper function to convert [U256] into [H256].
pub fn u256_to_h256_be(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_big_endian(h.as_mut());
    h
}

/// Small helper function to convert [H256] into [U256].
pub fn h256_to_u256_be(storage: H256) -> U256 {
    U256::from_big_endian(storage.as_bytes())
}

/// Small helper function to convert [H256] into [U256].
pub fn h256_to_u256_le(storage: H256) -> U256 {
    U256::from_little_endian(storage.as_bytes())
}

/// Builds the instruction counter map for the given bytecode.
// TODO: Some of the same logic is performed in REVM, but then later discarded. We should
// investigate if we can reuse it
pub fn build_ic_map(spec: SpecId, code: &Bytes) -> BTreeMap<usize, usize> {
    let opcode_infos = spec_opcode_gas(spec);
    let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();

    let mut i = 0;
    let mut cumulative_push_size = 0;
    while i < code.len() {
        let op = code[i];
        ic_map.insert(i, i - cumulative_push_size);
        if opcode_infos[op as usize].is_push {
            // Skip the push bytes.
            //
            // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
            i += (op - opcode::PUSH1 + 1) as usize;
            cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
        }
        i += 1;
    }

    ic_map
}

/// Given an ABI encoded error string with the function signature `Error(string)`, it decodes
/// it and returns the revert error message.
pub fn decode_revert(
    error: &[u8],
    maybe_abi: Option<&Abi>,
    status: Option<revm::Return>,
) -> eyre::Result<String> {
    if error.len() >= 4 {
        match error[0..4] {
            // keccak(Panic(uint256))
            [78, 72, 123, 113] => {
                // ref: https://soliditydeveloper.com/solidity-0.8
                match error[error.len() - 1] {
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
                        Ok("Calling a zero initialized variable of internal function type"
                            .to_string())
                    }
                    _ => Err(eyre::Error::msg("Unsupported solidity builtin panic")),
                }
            }
            // keccak(Error(string))
            [8, 195, 121, 160] => {
                if let Ok(decoded) = abi::decode(&[abi::ParamType::String], &error[4..]) {
                    Ok(decoded[0].to_string())
                } else {
                    Err(eyre::Error::msg("Bad string decode"))
                }
            }
            // keccak(expectRevert(bytes))
            [242, 141, 206, 179] => {
                let err_data = &error[4..];
                if err_data.len() > 64 {
                    let len = U256::from(&err_data[32..64]).as_usize();
                    if err_data.len() > 64 + len {
                        let actual_err = &err_data[64..64 + len];
                        if let Ok(decoded) = decode_revert(actual_err, maybe_abi, None) {
                            // check if its a builtin
                            return Ok(decoded)
                        } else if let Ok(as_str) = String::from_utf8(actual_err.to_vec()) {
                            // check if its a true string
                            return Ok(as_str)
                        }
                    }
                }
                Err(eyre::Error::msg("Non-native error and not string"))
            }
            // keccak(expectRevert(bytes4))
            [195, 30, 176, 224] => {
                let err_data = &error[4..];
                if err_data.len() == 32 {
                    let actual_err = &err_data[..4];
                    if let Ok(decoded) = decode_revert(actual_err, maybe_abi, None) {
                        // it's a known selector
                        return Ok(decoded)
                    }
                }
                Err(eyre::Error::msg("Unknown error selector"))
            }
            _ => {
                // try to decode a custom error if provided an abi
                if error.len() >= 4 {
                    if let Some(abi) = maybe_abi {
                        for abi_error in abi.errors() {
                            if abi_error.signature()[0..4] == error[0..4] {
                                // if we dont decode, dont return an error, try to decode as a
                                // string later
                                if let Ok(decoded) = abi_error.decode(&error[4..]) {
                                    let inputs = decoded
                                        .iter()
                                        .map(foundry_utils::format_token)
                                        .collect::<Vec<String>>()
                                        .join(", ");
                                    return Ok(format!("{}({})", abi_error.name, inputs))
                                }
                            }
                        }
                    }
                }
                // evm_error will sometimes not include the function selector for the error,
                // optimistically try to decode
                if let Ok(decoded) = abi::decode(&[abi::ParamType::String], error) {
                    Ok(decoded[0].to_string())
                } else {
                    Err(eyre::Error::msg("Non-native error and not string"))
                }
            }
        }
    } else {
        if let Some(status) = status {
            use revm::Return;
            if !matches!(status, revm::return_ok!()) {
                return Ok(format!("EvmError: {:?}", status))
            }
        }
        Err(eyre::Error::msg("Not enough error data to decode"))
    }
}
