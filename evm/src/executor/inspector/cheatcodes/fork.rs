use super::Cheatcodes;
use crate::{
    abi::HEVMCalls,
    executor::{backend::DatabaseExt, fork::CreateFork},
};
use bytes::Bytes;
use ethers::{abi::AbiEncode, types::BlockNumber};
use revm::EVMData;

/// Handles fork related cheatcodes
pub fn apply<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::CreateFork0(fork) => {
            create_fork(state, data, fork.0.clone(), BlockNumber::Latest)
        }
        HEVMCalls::CreateFork1(fork) => {
            create_fork(state, data, fork.0.clone(), fork.1.as_u64().into())
        }
        HEVMCalls::SelectFork(fork_id) => match data.db.select_fork(fork_id.0.clone()) {
            Ok(_) => Ok(Bytes::new()),
            Err(err) => Err(err.to_string().encode().into()),
        },
        HEVMCalls::RollFork0(fork) => match data.db.roll_fork(fork.0, None) {
            Ok(b) => Ok(b.encode().into()),
            Err(err) => Err(err.to_string().encode().into()),
        },
        HEVMCalls::RollFork1(fork) => {
            match data.db.roll_fork(fork.1, Some(fork.0.clone().into())) {
                Ok(b) => Ok(b.encode().into()),
                Err(err) => Err(err.to_string().encode().into()),
            }
        }
        _ => return None,
    })
}

/// Creates a new fork
fn create_fork<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    url_or_alias: String,
    block: BlockNumber,
) -> Result<Bytes, Bytes> {
    let url = state.config.get_rpc_url(url_or_alias)?;
    let fork = CreateFork {
        enable_caching: state.config.rpc_storage_caching.enable_for_endpoint(&url),
        url,
        block,
        chain_id: None,
        env: data.env.clone(),
    };
    match data.db.create_fork(fork) {
        Ok(id) => Ok(id.encode().into()),
        Err(err) => Err(err.to_string().encode().into()),
    }
}
