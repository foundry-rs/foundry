//! utilities used within tracing

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::Address;
use foundry_common::fmt::format_token;
use std::collections::HashMap;

/// Returns the label for the given [DynSolValue]
///
/// If the `token` is an `Address` then we look abel the label map.
/// by default the token is formatted using standard formatting
pub fn label(token: &DynSolValue, labels: &HashMap<Address, String>) -> String {
    match token {
        DynSolValue::Address(addr) => {
            if let Some(label) = labels.get(addr) {
                format!("{label}: [{addr}]")
            } else {
                format_token(token)
            }
        }
        _ => format_token(token),
    }
}
