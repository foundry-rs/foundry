//! utilities used within tracing

use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi as Abi};
use alloy_primitives::Address;
use foundry_common::{abi::format_token, SELECTOR_LEN};
use foundry_evm_core::decode;
use std::collections::HashMap;

/// Returns the label for the given [DynSolValue]
///
/// If the `token` is an `Address` then we look abel the label map.
/// by default the token is formatted using standard formatting
pub fn label(token: &DynSolValue, labels: &HashMap<Address, String>) -> String {
    match token {
        DynSolValue::Address(addr) => {
            if let Some(label) = labels.get(addr) {
                format!("{label}: [{}]", addr.to_checksum(None))
            } else {
                format_token(token)
            }
        }
        _ => format_token(token),
    }
}

/// Custom decoding of cheatcode calls
pub(crate) fn decode_cheatcode_inputs(
    func: &Function,
    data: &[u8],
    errors: &Abi,
    verbosity: u8,
) -> Option<Vec<String>> {
    match func.name.as_str() {
        "expectRevert" => {
            decode::decode_revert(data, Some(errors), None).ok().map(|decoded| vec![decoded])
        }
        "rememberKey" | "addr" | "startBroadcast" | "broadcast" => {
            // these functions accept a private key as uint256, which should not be
            // converted to plain text
            let _expected_type = DynSolType::Uint(256).to_string();
            if !func.inputs.is_empty() && matches!(&func.inputs[0].ty, _expected_type) {
                // redact private key input
                Some(vec!["<pk>".to_string()])
            } else {
                None
            }
        }
        "sign" => {
            // sign(uint256,bytes32)
            let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..], false).ok()?;
            let _expected_type = DynSolType::Uint(256).to_string();
            if !decoded.is_empty() && matches!(&func.inputs[0].ty, _expected_type) {
                decoded[0] = DynSolValue::String("<pk>".to_string());
            }
            Some(decoded.iter().map(format_token).collect())
        }
        "deriveKey" => Some(vec!["<pk>".to_string()]),
        "parseJson" |
        "parseJsonUint" |
        "parseJsonUintArray" |
        "parseJsonInt" |
        "parseJsonIntArray" |
        "parseJsonString" |
        "parseJsonStringArray" |
        "parseJsonAddress" |
        "parseJsonAddressArray" |
        "parseJsonBool" |
        "parseJsonBoolArray" |
        "parseJsonBytes" |
        "parseJsonBytesArray" |
        "parseJsonBytes32" |
        "parseJsonBytes32Array" |
        "writeJson" |
        "keyExists" |
        "serializeBool" |
        "serializeUint" |
        "serializeInt" |
        "serializeAddress" |
        "serializeBytes32" |
        "serializeString" |
        "serializeBytes" => {
            if verbosity >= 5 {
                None
            } else {
                let mut decoded = func.abi_decode_input(&data[SELECTOR_LEN..], false).ok()?;
                let token =
                    if func.name.as_str() == "parseJson" || func.name.as_str() == "keyExists" {
                        "<JSON file>"
                    } else {
                        "<stringified JSON>"
                    };
                decoded[0] = DynSolValue::String(token.to_string());
                Some(decoded.iter().map(format_token).collect())
            }
        }
        _ => None,
    }
}

/// Custom decoding of cheatcode return values
pub(crate) fn decode_cheatcode_outputs(
    func: &Function,
    _data: &[u8],
    verbosity: u8,
) -> Option<String> {
    if func.name.starts_with("env") {
        // redacts the value stored in the env var
        return Some("<env var value>".to_string())
    }
    if func.name == "deriveKey" {
        // redacts derived private key
        return Some("<pk>".to_string())
    }
    if func.name == "parseJson" && verbosity < 5 {
        return Some("<encoded JSON value>".to_string())
    }
    if func.name == "readFile" && verbosity < 5 {
        return Some("<file>".to_string())
    }
    None
}
