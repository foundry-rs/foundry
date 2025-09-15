use super::mining_engine::MiningError;
use anvil_rpc::{error::RpcError, response::ResponseResult};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block mining failed: {0:?}")]
    Mining(MiningError),
}

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

impl<T: Serialize> ToRpcResponseResult for Result<T, Error> {
    fn to_rpc_result(self) -> ResponseResult {
        match self {
            Ok(val) => to_rpc_result(val),
            Err(err) => match err {
                Error::Mining(mining_error) => {
                    RpcError::internal_error_with(format!("Block mining failed: {mining_error:?}"))
                }
            }
            .into(),
        }
    }
}
