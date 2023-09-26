use super::Result;
use crate::{abi::HEVMCalls, executor::backend::DatabaseExt};
use alloy_dyn_abi::DynSolValue;
use foundry_utils::types::ToAlloy;
use revm::EVMData;

/// Handles fork related cheatcodes
#[instrument(level = "error", name = "snapshot", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: DatabaseExt>(data: &mut EVMData<'_, DB>, call: &HEVMCalls) -> Option<Result> {
    Some(match call {
        HEVMCalls::Snapshot(_) => {
            Ok(DynSolValue::Uint(data.db.snapshot(&data.journaled_state, data.env), 32)
                .encode()
                .into())
        }
        HEVMCalls::RevertTo(snapshot) => {
            let res = if let Some(journaled_state) =
                data.db.revert(snapshot.0.to_alloy(), &data.journaled_state, data.env)
            {
                // we reset the evm's journaled_state to the state of the snapshot previous state
                data.journaled_state = journaled_state;
                true
            } else {
                false
            };
            Ok(DynSolValue::Bool(res).encode().into())
        }
        _ => return None,
    })
}
