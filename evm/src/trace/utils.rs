//! utilities used within tracing

use crate::decode;
use ethers::{
    abi::{Abi, Address, Function, Token},
    prelude::U256,
};
use foundry_common::SELECTOR_LEN;
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
                format!("{}: [{:?}]", label, addr)
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
            let err_data = &data[SELECTOR_LEN..];
            match data[..SELECTOR_LEN] {
                // `expectRevert(bytes)`
                [242, 141, 206, 179] if err_data.len() > 64 => {
                    let len = U256::from(&err_data[32..64]).as_usize();
                    if err_data.len() > 64 + len {
                        let actual_err = &err_data[64..64 + len];

                        // check if it's a builtin
                        return decode::decode_revert(actual_err, Some(errors), None)
                            .ok()
                            .map(|decoded| decoded.message)
                            // check if it's a string
                            .or_else(|| String::from_utf8(actual_err.to_vec()).ok())
                            .map(|decoded| vec![decoded])
                    }
                }
                // `expectRevert(bytes4)`
                [195, 30, 176, 224] if err_data.len() == 32 => {
                    let actual_err = &data[..SELECTOR_LEN];

                    return decode::decode_revert(actual_err, Some(errors), None)
                        .ok()
                        .map(|decoded| vec![decoded.message])
                }
                _ => (),
            }

            None
        }
        _ => None,
    }
}
