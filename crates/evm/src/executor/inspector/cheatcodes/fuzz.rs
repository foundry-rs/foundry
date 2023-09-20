use super::{Error, Result};
use crate::{abi::HEVMCalls, fuzz::error::ASSUME_MAGIC_RETURN_CODE};
use alloy_primitives::Bytes;

#[instrument(level = "error", name = "fuzz", target = "evm::cheatcodes", skip_all)]
pub fn apply(call: &HEVMCalls) -> Option<Result> {
    if let HEVMCalls::Assume(inner) = call {
        let bytes = if inner.0 {
            Ok(Bytes::new())
        } else {
            // `custom_bytes` will not encode with the error prefix.
            Err(Error::custom_bytes(ASSUME_MAGIC_RETURN_CODE))
        };
        Some(bytes)
    } else {
        None
    }
}
