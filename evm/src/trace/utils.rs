//! utilities used within tracing

use crate::decode;
use ethers::{
    abi::{Abi, Address, Function, ParamType, Token},
    core::utils::to_checksum,
};
use foundry_utils::format_token;
use std::collections::HashMap;

/// Returns the label for the given `token`
///
/// If the `token` is an `Address` then we look abel the label map.
/// by default the token is formatted using standard formatting
pub fn label(token: &Token, labels: &HashMap<Address, String>) -> String {
    match token {
        Token::Address(addr) => {
            if let Some(label) = labels.get(addr) {
                format!("{}: [{}]", label, to_checksum(addr, None))
            } else {
                format_token(token)
            }
        }
        _ => format_token(token),
    }
}

pub(crate) fn decode_cheatcode_inputs(
    func: &Function,
    data: &[u8],
    errors: &Abi,
) -> Option<Vec<String>> {
    match func.name.as_str() {
        "expectRevert" => {
            decode::decode_revert(data, Some(errors), None).ok().map(|decoded| vec![decoded])
        }
        "sign" | "startBroadcast" | "broadcast" => {
            // sign and broadcast functions accept a private key as uint256, which should not be
            // converted to plain text
            if func.inputs.len() > 0 && matches!(&func.inputs[0].kind, ParamType::Uint(_)) {
                // redact private key input
                Some(vec!["<pk>".to_string()])
            } else {
                None
            }
        }

        _ => None,
    }
}
