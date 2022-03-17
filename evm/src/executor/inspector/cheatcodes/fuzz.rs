use crate::{abi::HEVMCalls, fuzz::ASSUME_MAGIC_RETURN_CODE};
use bytes::Bytes;
use revm::{Database, EVMData};

pub fn apply<DB: Database>(
    _: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    if let HEVMCalls::Assume(inner) = call {
        Some(if inner.0 { Ok(Bytes::new()) } else { Err(ASSUME_MAGIC_RETURN_CODE.into()) })
    } else {
        None
    }
}
