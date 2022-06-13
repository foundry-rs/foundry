use super::Cheatcodes;
use crate::{
    abi::HEVMCalls,
    executor::{backend::DatabaseExt},
};
use bytes::Bytes;
use ethers::{abi::AbiEncode};
use revm::EVMData;

/// Handles fork related cheatcodes
pub fn apply<DB: DatabaseExt>(
    _state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Snapshot(_) => Ok(data.db.snapshot().encode().into()),
        HEVMCalls::RevertTo(snapshot) => Ok(data.db.revert(snapshot.0).encode().into()),
        _ => return None,
    })
}