use crate::substrate_node::mining_engine::MiningError;
use anvil_rpc::{error::RpcError, response::ResponseResult};
use polkadot_sdk::pallet_revive_eth_rpc::{EthRpcError, client::ClientError};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block mining failed: {0}")]
    Mining(#[from] MiningError),
    #[error("Rpc Endpoint not implemented")]
    RpcUnimplemented,
    #[error("Invalid params: {0}")]
    InvalidParams(String),
    #[error("Revive call failed: {0}")]
    ReviveRpc(#[from] EthRpcError),
    #[error("Internal error: {0}")]
    InternalError(String),
}
impl From<subxt::Error> for Error {
    fn from(err: subxt::Error) -> Self {
        Self::ReviveRpc(EthRpcError::ClientError(err.into()))
    }
}

impl From<ClientError> for Error {
    fn from(err: ClientError) -> Self {
        Self::ReviveRpc(EthRpcError::ClientError(err))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Helper trait to easily convert results to rpc results
pub(crate) trait ToRpcResponseResult {
    fn to_rpc_result(self) -> ResponseResult;
}

/// Converts a serializable value into a `ResponseResult`.
fn to_rpc_result<T: Serialize>(val: T) -> ResponseResult {
    match serde_json::to_value(val) {
        Ok(success) => ResponseResult::Success(success),
        Err(err) => {
            error!(%err, "Failed serialize rpc response");
            ResponseResult::error(RpcError::internal_error())
        }
    }
}

impl<T: Serialize> ToRpcResponseResult for Result<T> {
    fn to_rpc_result(self) -> ResponseResult {
        match self {
            Ok(val) => to_rpc_result(val),
            Err(err) => match err {
                Error::Mining(mining_error) => match mining_error {
                    MiningError::BlockProducing(error) => {
                        RpcError::internal_error_with(format!("Failed to produce a block: {error}"))
                            .into()
                    }
                    MiningError::MiningModeMismatch => {
                        RpcError::invalid_params("Current mining mode can not answer this query.")
                            .into()
                    }
                    MiningError::Timestamp => {
                        RpcError::invalid_params("Current timestamp is newer.").into()
                    }
                    MiningError::ClosedChannel => {
                        RpcError::internal_error_with("Communication channel was dropped.").into()
                    }
                },
                Error::RpcUnimplemented => RpcError::internal_error_with("Not implemented").into(),
                Error::InvalidParams(error_message) => {
                    RpcError::invalid_params(error_message).into()
                }
                Error::ReviveRpc(client_error) => {
                    RpcError::internal_error_with(format!("{client_error}")).into()
                }
                Error::InternalError(error_message) => {
                    RpcError::internal_error_with(error_message).into()
                }
            },
        }
    }
}
