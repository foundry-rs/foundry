//! utilities used within tracing

use ethers::abi::{Abi, Address, Function, Token};
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
            foundry_utils::decode_revert(data, Some(errors)).ok().map(|decoded| vec![decoded])
        }
        _ => None,
    }
}
