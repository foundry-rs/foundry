use crate::substrate_node::mining_engine::MiningError;
use anvil_rpc::{error::RpcError, response::ResponseResult};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block mining failed: {0}")]
    Mining(#[from] MiningError),
    #[error("Rpc Endpoint not implemented")]
    RpcUnimplemented,
    #[error("Invalid params: {0}")]
    InvalidParams(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) trait ToRpcResponseResult {
    fn to_rpc_result(self) -> ResponseResult;
}

/// Converts a serializable value into a `ResponseResult`
pub fn to_rpc_result<T: Serialize>(val: T) -> ResponseResult {
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
            Err(Error::InvalidParams(msg)) => RpcError::invalid_params(msg).into(),
            Err(err) => RpcError::internal_error_with(err.to_string()).into(),
        }
    }
}
