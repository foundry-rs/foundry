use super::Result;
use crate::{abi::HEVMCalls, fuzz::error::ASSUME_MAGIC_RETURN_CODE};
use ethers::types::Bytes;

pub fn apply(call: &HEVMCalls) -> Option<Result> {
    if let HEVMCalls::Assume(inner) = call {
        Some(if inner.0 { Ok(Bytes::new()) } else { Err(ASSUME_MAGIC_RETURN_CODE.into()) })
    } else {
        None
    }
}
