use super::{util, Cheatcodes};
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
    let resp = match call {
        HEVMCalls::CreateFork0(fork) => {
            create_fork(state, data, fork.0.clone(), BlockNumber::Latest)
        }
        HEVMCalls::CreateFork1(fork) => {
            create_fork(state, data, fork.0.clone(), fork.1.as_u64().into())
        }
        HEVMCalls::SelectFork(fork_id) => data
            .db
            .select_fork(fork_id.0, data.env)
            .map(|_| Default::default())
            .map_err(util::encode_error),
        HEVMCalls::RollFork0(fork) => {
            let block_number = fork.0;
            let resp = data.db.roll_fork(block_number, None).map(|_| Default::default()).map_err(util::encode_error);
            if resp.is_ok() {
                data.env.block.number = block_number;
            }
            resp
        }
        HEVMCalls::RollFork1(fork) => {
            let block_number = fork.1;
            let resp = data.db
            .roll_fork(block_number, Some(fork.0))
            .map(|_| Default::default())
            .map_err(util::encode_error);
            if resp.is_ok() {
                data.env.block.number = block_number;
            }
            resp
        },
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
        _ => return None,
    };

    Some(resp)
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
        evm_opts: state.config.evm_opts.clone(),
    };
    data.db.create_fork(fork).map_err(util::encode_error).map(|id| id.encode().into())
}
