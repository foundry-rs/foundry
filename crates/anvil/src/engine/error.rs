use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use serde::Serialize;


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

/// Helper trait to easily convert results to rpc results
pub(crate) trait ToRpcResponseResult {
    fn to_rpc_result(self) -> ResponseResult;
}

impl<T: Serialize> ToRpcResponseResult for Result<T> {
    fn to_rpc_result(self) -> ResponseResult {
        match self {
            Ok(val) => to_rpc_result(val),
            Err(err) => to_rpc_result(RpcError::internal_error()),
        }
    }
}

pub(crate) type Result<T> = std::result::Result<T, EngineError>;

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`")]
    InvalidFeeInput,
}