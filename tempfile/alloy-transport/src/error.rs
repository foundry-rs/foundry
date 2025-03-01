use alloy_json_rpc::{ErrorPayload, Id, RpcError, RpcResult};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::{error::Error as StdError, fmt::Debug};
use thiserror::Error;

/// A transport error is an [`RpcError`] containing a [`TransportErrorKind`].
pub type TransportError<ErrResp = Box<RawValue>> = RpcError<TransportErrorKind, ErrResp>;

/// A transport result is a [`Result`] containing a [`TransportError`].
pub type TransportResult<T, ErrResp = Box<RawValue>> = RpcResult<T, TransportErrorKind, ErrResp>;

/// Transport error.
///
/// All transport errors are wrapped in this enum.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TransportErrorKind {
    /// Missing batch response.
    ///
    /// This error is returned when a batch request is sent and the response
    /// does not contain a response for a request. For convenience the ID is
    /// specified.
    #[error("missing response for request with ID {0}")]
    MissingBatchResponse(Id),

    /// Backend connection task has stopped.
    #[error("backend connection task has stopped")]
    BackendGone,

    /// Pubsub service is not available for the current provider.
    #[error("subscriptions are not available on this provider")]
    PubsubUnavailable,

    /// HTTP Error with code and body
    #[error("{0}")]
    HttpError(#[from] HttpError),

    /// Custom error.
    #[error("{0}")]
    Custom(#[source] Box<dyn StdError + Send + Sync + 'static>),
}

impl TransportErrorKind {
    /// Returns `true` if the error is potentially recoverable.
    /// This is a naive heuristic and should be used with caution.
    pub const fn recoverable(&self) -> bool {
        matches!(self, Self::MissingBatchResponse(_))
    }

    /// Instantiate a new `TransportError` from a custom error.
    pub fn custom_str(err: &str) -> TransportError {
        RpcError::Transport(Self::Custom(err.into()))
    }

    /// Instantiate a new `TransportError` from a custom error.
    pub fn custom(err: impl StdError + Send + Sync + 'static) -> TransportError {
        RpcError::Transport(Self::Custom(Box::new(err)))
    }

    /// Instantiate a new `TransportError` from a missing ID.
    pub const fn missing_batch_response(id: Id) -> TransportError {
        RpcError::Transport(Self::MissingBatchResponse(id))
    }

    /// Instantiate a new `TransportError::BackendGone`.
    pub const fn backend_gone() -> TransportError {
        RpcError::Transport(Self::BackendGone)
    }

    /// Instantiate a new `TransportError::PubsubUnavailable`.
    pub const fn pubsub_unavailable() -> TransportError {
        RpcError::Transport(Self::PubsubUnavailable)
    }

    /// Instantiate a new `TransportError::HttpError`.
    pub const fn http_error(status: u16, body: String) -> TransportError {
        RpcError::Transport(Self::HttpError(HttpError { status, body }))
    }

    /// Analyzes the [TransportErrorKind] and decides if the request should be retried based on the
    /// variant.
    pub fn is_retry_err(&self) -> bool {
        match self {
            // Missing batch response errors can be retried.
            Self::MissingBatchResponse(_) => true,
            Self::HttpError(http_err) => {
                http_err.is_rate_limit_err() || http_err.is_temporarily_unavailable()
            }
            Self::Custom(err) => {
                let msg = err.to_string();
                msg.contains("429 Too Many Requests")
            }
            _ => false,
        }
    }
}

/// Type for holding HTTP errors such as 429 rate limit error.
#[derive(Debug, thiserror::Error)]
#[error(
    "HTTP error {status} with {}",
    if body.is_empty() { "empty body".to_string() } else { format!("body: {body}") }
)]
pub struct HttpError {
    /// The HTTP status code.
    pub status: u16,
    /// The HTTP response body.
    pub body: String,
}

impl HttpError {
    /// Checks the `status` to determine whether the request should be retried.
    pub const fn is_rate_limit_err(&self) -> bool {
        self.status == 429
    }

    /// Checks the `status` to determine whether the service was temporarily unavailable and should
    /// be retried.
    pub const fn is_temporarily_unavailable(&self) -> bool {
        self.status == 503
    }
}

/// Extension trait to implement methods for [`RpcError<TransportErrorKind, E>`].
pub(crate) trait RpcErrorExt {
    /// Analyzes whether to retry the request depending on the error.
    fn is_retryable(&self) -> bool;

    /// Fetches the backoff hint from the error message if present
    fn backoff_hint(&self) -> Option<std::time::Duration>;
}

impl RpcErrorExt for RpcError<TransportErrorKind> {
    fn is_retryable(&self) -> bool {
        match self {
            // There was a transport-level error. This is either a non-retryable error,
            // or a server error that should be retried.
            Self::Transport(err) => err.is_retry_err(),
            // The transport could not serialize the error itself. The request was malformed from
            // the start.
            Self::SerError(_) => false,
            Self::DeserError { text, .. } => {
                if let Ok(resp) = serde_json::from_str::<ErrorPayload>(text) {
                    return resp.is_retry_err();
                }

                // some providers send invalid JSON RPC in the error case (no `id:u64`), but the
                // text should be a `JsonRpcError`
                #[derive(Deserialize)]
                struct Resp {
                    error: ErrorPayload,
                }

                if let Ok(resp) = serde_json::from_str::<Resp>(text) {
                    return resp.error.is_retry_err();
                }

                false
            }
            Self::ErrorResp(err) => err.is_retry_err(),
            Self::NullResp => true,
            _ => false,
        }
    }

    fn backoff_hint(&self) -> Option<std::time::Duration> {
        if let Self::ErrorResp(resp) = self {
            let data = resp.try_data_as::<serde_json::Value>();
            if let Some(Ok(data)) = data {
                // if daily rate limit exceeded, infura returns the requested backoff in the error
                // response
                let backoff_seconds = &data["rate"]["backoff_seconds"];
                // infura rate limit error
                if let Some(seconds) = backoff_seconds.as_u64() {
                    return Some(std::time::Duration::from_secs(seconds));
                }
                if let Some(seconds) = backoff_seconds.as_f64() {
                    return Some(std::time::Duration::from_secs(seconds as u64 + 1));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_error() {
        let err = "{\"code\":-32007,\"message\":\"100/second request limit reached - reduce calls per second or upgrade your account at quicknode.com\"}";
        let err = serde_json::from_str::<ErrorPayload>(err).unwrap();
        assert!(TransportError::ErrorResp(err).is_retryable());
    }
}
