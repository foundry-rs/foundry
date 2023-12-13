//! An utility trait for retrying requests based on the error type. See [TransportError].
use alloy_json_rpc::ErrorPayload;
use alloy_transport::TransportError;
use serde::Deserialize;

/// [RetryPolicy] defines logic for which [JsonRpcClient::Error] instances should
/// the client retry the request and try to recover from.
pub trait RetryPolicy: Send + Sync + std::fmt::Debug {
    /// Whether to retry the request based on the given `error`
    fn should_retry(&self, error: &TransportError) -> bool;

    /// Providers may include the `backoff` in the error response directly
    fn backoff_hint(&self, error: &TransportError) -> Option<std::time::Duration>;
}

/// Implements [RetryPolicy] that will retry requests that errored with
/// status code 429 i.e. TOO_MANY_REQUESTS
///
/// Infura often fails with a `"header not found"` rpc error which is apparently linked to load
/// balancing, which are retried as well.
#[derive(Clone, Debug, Default)]
pub struct RateLimitRetryPolicy;

impl RetryPolicy for RateLimitRetryPolicy {
    fn backoff_hint(&self, error: &TransportError) -> Option<std::time::Duration> {
        if let TransportError::ErrorResp(resp) = error {
            println!("resp: {:?}", resp);
            let data = resp.try_data_as::<serde_json::Value>();
            if let Some(Ok(data)) = data {
                // if daily rate limit exceeded, infura returns the requested backoff in the error
                // response
                let backoff_seconds = &data["rate"]["backoff_seconds"];
                // infura rate limit error
                if let Some(seconds) = backoff_seconds.as_u64() {
                    return Some(std::time::Duration::from_secs(seconds))
                }
                if let Some(seconds) = backoff_seconds.as_f64() {
                    return Some(std::time::Duration::from_secs(seconds as u64 + 1))
                }
            }
        }
        None
    }

    fn should_retry(&self, error: &TransportError) -> bool {
        match error {
            TransportError::Transport(_) => true,
            // The transport could not serialize the error itself. The request was malformed from
            // the start.
            TransportError::SerError(_) => false,
            TransportError::DeserError { text, .. } => {
                // some providers send invalid JSON RPC in the error case (no `id:u64`), but the
                // text should be a `JsonRpcError`
                #[derive(Deserialize)]
                struct Resp {
                    error: ErrorPayload,
                }

                if let Ok(resp) = serde_json::from_str::<Resp>(text) {
                    return should_retry_json_rpc_error(&resp.error)
                }
                false
            }
            TransportError::ErrorResp(err) => should_retry_json_rpc_error(err),
        }
    }
}

/// Analyzes the [ErrorPayload] and decides if the request should be retried based on the
/// error code or the message.
fn should_retry_json_rpc_error(error: &ErrorPayload) -> bool {
    let ErrorPayload { code, message, .. } = error;
    // alchemy throws it this way
    if *code == 429 {
        return true
    }

    // This is an infura error code for `exceeded project rate limit`
    if *code == -32005 {
        return true
    }

    // alternative alchemy error for specific IPs
    if *code == -32016 && message.contains("rate limit") {
        return true
    }

    match message.as_str() {
        // this is commonly thrown by infura and is apparently a load balancer issue, see also <https://github.com/MetaMask/metamask-extension/issues/7234>
        "header not found" => true,
        // also thrown by infura if out of budget for the day and ratelimited
        "daily request count exceeded, request rate limited" => true,
        _ => false,
    }
}