use crate::{
    error::RpcError,
    request::{Id, Version},
};
use serde::{Deserialize, Serialize};

/// Response of a _single_ rpc call
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcResponse {
    // JSON RPC version
    jsonrpc: Version,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    #[serde(flatten)]
    result: ResponseResult,
}

impl From<RpcError> for RpcResponse {
    fn from(e: RpcError) -> Self {
        Self { jsonrpc: Version::V2, id: None, result: ResponseResult::Error(e) }
    }
}

impl RpcResponse {
    pub fn new(id: Id, content: impl Into<ResponseResult>) -> Self {
        Self { jsonrpc: Version::V2, id: Some(id), result: content.into() }
    }

    pub fn invalid_request(id: Id) -> Self {
        Self::new(id, RpcError::invalid_request())
    }
}

/// Represents the result of a call either success or error
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ResponseResult {
    #[serde(rename = "result")]
    Success(serde_json::Value),
    #[serde(rename = "error")]
    Error(RpcError),
}

impl ResponseResult {
    pub fn success<S>(content: S) -> Self
    where
        S: Serialize + 'static,
    {
        Self::Success(serde_json::to_value(&content).unwrap())
    }

    pub fn error(error: RpcError) -> Self {
        Self::Error(error)
    }
}

impl From<RpcError> for ResponseResult {
    fn from(err: RpcError) -> Self {
        Self::error(err)
    }
}
/// Synchronous response
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Response {
    /// single json rpc response
    Single(RpcResponse),
    /// batch of several responses
    Batch(Vec<RpcResponse>),
}

impl Response {
    /// Creates new [`Response`] with the given [`RpcError`].
    pub fn error(error: RpcError) -> Self {
        RpcResponse::new(Id::Null, ResponseResult::Error(error)).into()
    }
}

impl From<RpcError> for Response {
    fn from(err: RpcError) -> Self {
        Self::error(err)
    }
}

impl From<RpcResponse> for Response {
    fn from(resp: RpcResponse) -> Self {
        Self::Single(resp)
    }
}
