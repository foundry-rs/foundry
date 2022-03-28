use crate::{
    error::Error,
    request::{Id, Version},
};
use serde::{Deserialize, Serialize};

/// Response of a _single_ rpc call
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcResponse {
    // JSON RPC version
    jsonrpc: Version,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    #[serde(flatten)]
    result: ResponseResult,
}

impl From<Error> for RpcResponse {
    fn from(e: Error) -> Self {
        Self { jsonrpc: Version::V2, id: None, result: ResponseResult::Error(e) }
    }
}

impl RpcResponse {
    pub fn new(id: Id, content: impl Into<ResponseResult>) -> Self {
        RpcResponse { jsonrpc: Version::V2, id: Some(id), result: content.into() }
    }

    pub fn invalid_request(id: Id) -> Self {
        Self::new(id, Error::invalid_request())
    }
}

/// Represents the result of a call either success or error
#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub enum ResponseResult {
    #[serde(rename = "result")]
    Success(serde_json::Value),
    #[serde(rename = "error")]
    Error(Error),
}

impl ResponseResult {
    pub fn success<S>(content: S) -> Self
    where
        S: Serialize + 'static,
    {
        ResponseResult::Success(serde_json::to_value(&content).unwrap())
    }

    pub fn error(error: Error) -> Self {
        ResponseResult::Error(error)
    }
}

impl From<Error> for ResponseResult {
    fn from(err: Error) -> Self {
        ResponseResult::error(err)
    }
}

/// Synchronous response
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Response {
    /// single json rpc response
    Single(RpcResponse),
    /// batch of several responses
    Batch(Vec<RpcResponse>),
}

impl Response {
    /// Creates new [Response] with the given [Error]
    pub fn error(error: Error) -> Self {
        RpcResponse::new(Id::Null, ResponseResult::Error(error)).into()
    }
}

impl From<Error> for Response {
    fn from(err: Error) -> Self {
        Response::error(err)
    }
}

impl From<RpcResponse> for Response {
    fn from(resp: RpcResponse) -> Self {
        Response::Single(resp)
    }
}
