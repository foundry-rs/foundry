use super::{error::CheatcodesError, Cheatcodes};
use crate::{
    abi::HEVMCalls,
    error,
    error::SolError,
    executor::{backend::DatabaseExt, fork::CreateFork},
};
use bytes::Bytes;
use ethers::{abi::AbiEncode, prelude::U256, types::H256};
use revm::EVMData;

/// Handles fork related cheatcodes
pub fn apply<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    let resp = match call {
        HEVMCalls::CreateFork0(fork) => {
            create_fork(state, data, fork.0.clone(), None).map(|id| id.encode().into())
        }
        HEVMCalls::CreateFork1(fork) => {
            create_fork(state, data, fork.0.clone(), Some(fork.1.as_u64()))
                .map(|id| id.encode().into())
        }
        HEVMCalls::CreateFork2(fork) => {
            create_fork_at_transaction(state, data, fork.0.clone(), fork.1.into())
                .map(|id| id.encode().into())
        }
        HEVMCalls::CreateSelectFork0(fork) => {
            create_select_fork(state, data, fork.0.clone(), None).map(|id| id.encode().into())
        }
        HEVMCalls::CreateSelectFork1(fork) => {
            create_select_fork(state, data, fork.0.clone(), Some(fork.1.as_u64()))
                .map(|id| id.encode().into())
        }
        HEVMCalls::CreateSelectFork2(fork) => {
            create_select_fork_at_transaction(state, data, fork.0.clone(), fork.1.into())
                .map(|id| id.encode().into())
        }
        HEVMCalls::SelectFork(fork_id) => select_fork(state, data, fork_id.0),
        HEVMCalls::MakePersistent0(acc) => {
            data.db.add_persistent_account(acc.0);
            Ok(Default::default())
        }
        HEVMCalls::MakePersistent1(acc) => {
            data.db.extend_persistent_accounts(acc.0.clone());
            Ok(Default::default())
        }
        HEVMCalls::MakePersistent2(acc) => {
            data.db.add_persistent_account(acc.0);
            data.db.add_persistent_account(acc.1);
            Ok(Default::default())
        }
        HEVMCalls::MakePersistent3(acc) => {
            data.db.add_persistent_account(acc.0);
            data.db.add_persistent_account(acc.1);
            data.db.add_persistent_account(acc.2);
            Ok(Default::default())
        }
        HEVMCalls::IsPersistent(acc) => Ok(data.db.is_persistent(&acc.0).encode().into()),
        HEVMCalls::RevokePersistent0(acc) => {
            data.db.remove_persistent_account(&acc.0);
            Ok(Default::default())
        }
        HEVMCalls::RevokePersistent1(acc) => {
            data.db.remove_persistent_accounts(acc.0.clone());
            Ok(Default::default())
        }
        HEVMCalls::ActiveFork(_) => data
            .db
            .active_fork_id()
            .map(|id| id.encode().into())
            .ok_or_else(|| error::encode_error("No active fork")),
        HEVMCalls::RollFork0(fork) => {
            let block_number = fork.0;
            data.db
                .roll_fork(None, block_number, data.env, &mut data.journaled_state)
                .map(|_| Default::default())
                .map_err(error::encode_error)
        }
        HEVMCalls::RollFork1(fork) => data
            .db
            .roll_fork_to_transaction(None, fork.0.into(), data.env, &mut data.journaled_state)
            .map(|_| Default::default())
            .map_err(error::encode_error),
        HEVMCalls::RollFork2(fork) => {
            let block_number = fork.1;
            data.db
                .roll_fork(Some(fork.0), block_number, data.env, &mut data.journaled_state)
                .map(|_| Default::default())
                .map_err(error::encode_error)
        }
        HEVMCalls::RollFork3(fork) => data
            .db
            .roll_fork_to_transaction(
                Some(fork.0),
                fork.1.into(),
                data.env,
                &mut data.journaled_state,
            )
            .map(|_| Default::default())
            .map_err(error::encode_error),
        HEVMCalls::RpcUrl(rpc) => state.config.get_rpc_url(&rpc.0).map(|url| url.encode().into()),
        HEVMCalls::RpcUrls(_) => {
            let mut urls = Vec::with_capacity(state.config.rpc_endpoints.len());
            for alias in state.config.rpc_endpoints.keys().cloned() {
                match state.config.get_rpc_url(&alias) {
                    Ok(url) => {
                        urls.push([alias, url]);
                    }
                    Err(err) => return Some(Err(err)),
                }
            }
            Ok(urls.encode().into())
        }
        HEVMCalls::RpcUrlStructs(_) => {
            let mut urls = Vec::with_capacity(state.config.rpc_endpoints.len());
            for alias in state.config.rpc_endpoints.keys().cloned() {
                match state.config.get_rpc_url(&alias) {
                    Ok(url) => {
                        urls.push([alias, url]);
                    }
                    Err(err) => return Some(Err(err)),
                }
            }
            Ok(urls.encode().into())
        }
        HEVMCalls::AllowCheatcodes(addr) => {
            data.db.allow_cheatcode_access(addr.0);
            Ok(Default::default())
        }
        HEVMCalls::Transact0(inner) => data
            .db
            .transact(None, inner.0.into(), data.env, &mut data.journaled_state, Some(state))
            .map(|_| Default::default())
            .map_err(error::encode_error),
        HEVMCalls::Transact1(inner) => data
            .db
            .transact(
                Some(inner.0),
                inner.1.into(),
                data.env,
                &mut data.journaled_state,
                Some(state),
            )
            .map(|_| Default::default())
            .map_err(error::encode_error),
        _ => return None,
    };

    Some(resp)
}

/// Selects the given fork id
fn select_fork<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<DB>,
    fork_id: U256,
) -> Result<Bytes, Bytes> {
    if state.broadcast.is_some() {
        return Err(CheatcodesError::SelectForkDuringBroadcast.encode_string())
    }

    // No need to correct since the sender's nonce does not get incremented when selecting a fork.
    state.corrected_nonce = true;

    data.db
        .select_fork(fork_id, data.env, &mut data.journaled_state)
        .map(|_| Default::default())
        .map_err(error::encode_error)
}

/// Creates and then also selects the new fork
fn create_select_fork<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    url_or_alias: String,
    block: Option<u64>,
) -> Result<U256, Bytes> {
    if state.broadcast.is_some() {
        return Err(CheatcodesError::SelectForkDuringBroadcast.encode_string())
    }

    // No need to correct since the sender's nonce does not get incremented when selecting a fork.
    state.corrected_nonce = true;

    let fork = create_fork_request(state, url_or_alias, block, data)?;
    data.db
        .create_select_fork(fork, data.env, &mut data.journaled_state)
        .map_err(error::encode_error)
}

/// Creates a new fork
fn create_fork<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    url_or_alias: String,
    block: Option<u64>,
) -> Result<U256, Bytes> {
    let fork = create_fork_request(state, url_or_alias, block, data)?;
    data.db.create_fork(fork).map_err(error::encode_error)
}
/// Creates and then also selects the new fork at the given transaction
fn create_select_fork_at_transaction<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    url_or_alias: String,
    transaction: H256,
) -> Result<U256, Bytes> {
    if state.broadcast.is_some() {
        return Err(CheatcodesError::SelectForkDuringBroadcast.encode_string())
    }

    // No need to correct since the sender's nonce does not get incremented when selecting a fork.
    state.corrected_nonce = true;

    let fork = create_fork_request(state, url_or_alias, None, data)?;
    data.db
        .create_select_fork_at_transaction(fork, data.env, &mut data.journaled_state, transaction)
        .map_err(error::encode_error)
}

/// Creates a new fork at the given transaction
fn create_fork_at_transaction<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    url_or_alias: String,
    transaction: H256,
) -> Result<U256, Bytes> {
    let fork = create_fork_request(state, url_or_alias, None, data)?;
    data.db.create_fork_at_transaction(fork, transaction).map_err(error::encode_error)
}

/// Creates the request object for a new fork request
fn create_fork_request<DB: DatabaseExt>(
    state: &Cheatcodes,
    url_or_alias: String,
    block: Option<u64>,
    data: &EVMData<DB>,
) -> Result<CreateFork, Bytes> {
    let url = state.config.get_rpc_url(url_or_alias)?;
    let mut evm_opts = state.config.evm_opts.clone();
    evm_opts.fork_block_number = block;
    let fork = CreateFork {
        enable_caching: state.config.rpc_storage_caching.enable_for_endpoint(&url),
        url,
        env: data.env.clone(),
        evm_opts,
    };
    Ok(fork)
}
