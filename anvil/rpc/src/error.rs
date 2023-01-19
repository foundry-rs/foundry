//! JSON-RPC error bindings
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Cow, fmt};

/// Represents a JSON-RPC error
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub code: ErrorCode,
    /// error message
    pub message: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    /// New [Error] with the given [ErrorCode]
    pub const fn new(code: ErrorCode) -> Self {
        RpcError { message: Cow::Borrowed(code.message()), code, data: None }
    }

    /// Creates a new `ParseError`
    pub const fn parse_error() -> Self {
        Self::new(ErrorCode::ParseError)
    }

    /// Creates a new `MethodNotFound`
    pub const fn method_not_found() -> Self {
        Self::new(ErrorCode::MethodNotFound)
    }

    /// Creates a new `InvalidRequest`
    pub const fn invalid_request() -> Self {
        Self::new(ErrorCode::InvalidRequest)
    }

    /// Creates a new `InternalError`
    pub const fn internal_error() -> Self {
        Self::new(ErrorCode::InternalError)
    }

    /// Creates a new `InvalidParams`
    pub fn invalid_params<M>(message: M) -> Self
    where
        M: Into<String>,
    {
        RpcError { code: ErrorCode::InvalidParams, message: message.into().into(), data: None }
    }

    /// Creates a new `InternalError` with a message
    pub fn internal_error_with<M>(message: M) -> Self
    where
        M: Into<String>,
    {
        RpcError { code: ErrorCode::InternalError, message: message.into().into(), data: None }
    }

    /// Creates a new rpc error for when a transaction was rejected
    pub fn transaction_rejected<M>(message: M) -> Self
    where
        M: Into<String>,
    {
        RpcError {
            code: ErrorCode::TransactionRejected,
            message: message.into().into(),
            data: None,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.code.message(), self.message)
    }
}

/// List of JSON-RPC error codes
#[derive(Debug, Copy, PartialEq, Eq, Clone)]
pub enum ErrorCode {
    /// Server received Invalid JSON.
    /// server side error while parsing JSON
    ParseError,
    /// send invalid request object.
    InvalidRequest,
    /// method does not exist or valid
    MethodNotFound,
    /// invalid method parameter.
    InvalidParams,
    /// internal call error
    InternalError,
    /// Failed to send transaction, See also <https://github.com/MetaMask/eth-rpc-errors/blob/main/src/error-constants.ts>
    TransactionRejected,
    /// Custom geth error code, <https://github.com/vapory-legacy/wiki/blob/master/JSON-RPC-Error-Codes-Improvement-Proposal.md>
    ExecutionError,
    /// Used for server specific errors.
    ServerError(i64),
}

impl ErrorCode {
    /// Returns the error code as `i64`
    pub fn code(&self) -> i64 {
        match *self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::TransactionRejected => -32003,
            ErrorCode::ExecutionError => 3,
            ErrorCode::ServerError(c) => c,
        }
    }

    /// Returns the message associated with the error
    pub const fn message(&self) -> &'static str {
        match *self {
            ErrorCode::ParseError => "Parse error",
            ErrorCode::InvalidRequest => "Invalid request",
            ErrorCode::MethodNotFound => "Method not found",
            ErrorCode::InvalidParams => "Invalid params",
            ErrorCode::InternalError => "Internal error",
            ErrorCode::TransactionRejected => "Transaction rejected",
            ErrorCode::ServerError(_) => "Server error",
            ErrorCode::ExecutionError => "Execution error",
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(self.code())
    }
}

impl<'a> Deserialize<'a> for ErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<ErrorCode, D::Error>
    where
        D: Deserializer<'a>,
    {
        i64::deserialize(deserializer).map(Into::into)
    }
}

impl From<i64> for ErrorCode {
    fn from(code: i64) -> Self {
        match code {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            -32003 => ErrorCode::TransactionRejected,
            3 => ErrorCode::ExecutionError,
            _ => ErrorCode::ServerError(code),
        }
    }
}
