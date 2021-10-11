use std::borrow::Cow;

use axum::{body::Body, response::IntoResponse};
use http::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    jsonrpc: Version,
    id: Id,
    method: String,
    params: Option<Box<RawValue>>,
}

impl JsonRpcRequest {
    pub fn id(&self) -> Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Version {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Id {
    String(String),
    Number(i64),
    Null,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    jsonrpc: Version,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    #[serde(flatten)]
    content: ResponseContent,
}

impl From<Error> for JsonRpcResponse {
    fn from(e: Error) -> Self {
        Self { jsonrpc: Version::V2, id: None, content: ResponseContent::Error(e) }
    }
}

impl JsonRpcResponse {
    pub fn new(id: Id, content: ResponseContent) -> Self {
        JsonRpcResponse { jsonrpc: Version::V2, id: Some(id), content }
    }
}

impl IntoResponse for JsonRpcResponse {
    type Body = Body;
    type BodyError = <Self::Body as axum::body::HttpBody>::Error;

    fn into_response(self) -> http::Response<Self::Body> {
        let body = Body::from(serde_json::to_vec(&self).unwrap());
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap()
    }
}

#[derive(Serialize)]
pub enum ResponseContent {
    #[serde(rename = "result")]
    Success(Box<dyn erased_serde::Serialize>),
    #[serde(rename = "error")]
    Error(Error),
}

#[derive(Serialize)]
pub struct Error {
    code: i64,
    message: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Box<dyn erased_serde::Serialize>>,
}

impl Error {
    pub const INVALID_REQUEST: Error =
        Error { code: -32600, message: Cow::Borrowed("Invalid Request"), data: None };

    pub const METHOD_NOT_FOUND: Error =
        Error { code: -32601, message: Cow::Borrowed("Method not found"), data: None };

    pub const INVALID_PARAMS: Error =
        Error { code: -32602, message: Cow::Borrowed("Invalid params"), data: None };
}

impl ResponseContent {
    pub fn success<S>(content: S) -> Self
    where
        S: Serialize + 'static,
    {
        ResponseContent::Success(Box::new(content))
    }

    pub fn error(error: Error) -> Self {
        ResponseContent::Error(error)
    }
}
