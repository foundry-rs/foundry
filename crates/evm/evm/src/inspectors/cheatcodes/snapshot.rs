use super::{fmt_err, Result};
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::Address;
use ethers::utils::GenesisAccount;
use foundry_evm_core::{abi::HEVMCalls, backend::DatabaseExt};
use foundry_utils::types::ToAlloy;
use revm::EVMData;
use std::{collections::HashMap, fs::File};

/// Handles fork related cheatcodes
#[instrument(level = "error", name = "snapshot", target = "evm::cheatcodes", skip_all)]
pub fn apply<DB: DatabaseExt>(data: &mut EVMData<'_, DB>, call: &HEVMCalls) -> Option<Result> {
    Some(match call {
        HEVMCalls::Snapshot(_) => {
            Ok(DynSolValue::Uint(data.db.snapshot(&data.journaled_state, data.env), 256)
                .abi_encode()
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
            Ok(DynSolValue::Bool(res).abi_encode().into())
        }
        HEVMCalls::LoadAllocs(path) => {
            // First, load the allocs file from the provided path.
            let Ok(file) = File::open(&path.0) else {
                return Some(Err(fmt_err!("Failed to open allocs JSON at path \"{}\"", &path.0)));
            };
            let Ok(allocs): Result<HashMap<Address, GenesisAccount>, _> =
                serde_json::from_reader(file)
            else {
                return Some(Err(fmt_err!("Failed to parse allocs JSON at path \"{}\"", &path.0)));
            };

            // Loop through all of the allocs defined in the map and commit them to the journal.
            data.db
                .load_allocs(&allocs, &mut data.journaled_state)
                .map(|_| alloy_primitives::Bytes::default())
                .map_err(|e| fmt_err!("Failed to load allocs: {}", e))
        }
        _ => return None,
    })
}
