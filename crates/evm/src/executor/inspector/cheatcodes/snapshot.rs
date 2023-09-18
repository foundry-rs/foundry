use super::Result;
use crate::{
    abi::HEVMCalls,
    executor::backend::DatabaseExt,
    utils::{ru256_to_u256, u256_to_ru256},
};
use ethers::abi::AbiEncode;
use revm::EVMData;

/// Handles fork related cheatcodes
#[instrument(level = "error", name = "snapshot", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: DatabaseExt>(data: &mut EVMData<'_, DB>, call: &HEVMCalls) -> Option<Result> {
    Some(match call {
        HEVMCalls::Snapshot(_) => {
            Ok(ru256_to_u256(data.db.snapshot(&data.journaled_state, data.env)).encode().into())
        }
        HEVMCalls::RevertTo(snapshot) => {
            let res = if let Some(journaled_state) =
                data.db.revert(u256_to_ru256(snapshot.0), &data.journaled_state, data.env)
            {
                // we reset the evm's journaled_state to the state of the snapshot previous state
                data.journaled_state = journaled_state;
                true
            } else {
                false
            };
            Ok(res.encode().into())
        }
        _ => return None,
    })
}
