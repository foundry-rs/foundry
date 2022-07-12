use super::Cheatcodes;
use crate::{abi::HEVMCalls, executor::backend::DatabaseExt};
use bytes::Bytes;
use ethers::abi::AbiEncode;
use revm::EVMData;

/// Handles fork related cheatcodes
pub fn apply<DB: DatabaseExt>(
    _state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::Snapshot(_) => Ok(data.db.snapshot(&data.subroutine, data.env).encode().into()),
        HEVMCalls::RevertTo(snapshot) => {
            let res =
                if let Some(subroutine) = data.db.revert(snapshot.0, &data.subroutine, data.env) {
                    // we reset the evm's subroutine to the state of the snapshot previous state
                    data.subroutine = subroutine;
                    true
                } else {
                    false
                };
            Ok(res.encode().into())
        }
        _ => return None,
    })
}
