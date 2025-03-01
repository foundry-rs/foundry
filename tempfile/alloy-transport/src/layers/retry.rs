use crate::{
    error::{RpcErrorExt, TransportError, TransportErrorKind},
    TransportFut,
};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tower::{Layer, Service};
use tracing::trace;

#[cfg(target_arch = "wasm32")]
use wasmtimer::tokio::sleep;

#[cfg(not(target_arch = "wasm32"))]
use tokio::time::sleep;

/// The default average cost of a request in compute units (CU).
const DEFAULT_AVG_COST: u64 = 17u64;

/// A Transport Layer that is responsible for retrying requests based on the
/// error type. See [`TransportError`].
///
/// TransportError: crate::error::TransportError
#[derive(Debug, Clone)]
pub struct RetryBackoffLayer<P: RetryPolicy = RateLimitRetryPolicy> {
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
    /// The number of compute units per second for this provider
    compute_units_per_second: u64,
    /// The average cost of a request. Defaults to [DEFAULT_AVG_COST]
    avg_cost: u64,
    /// The [RetryPolicy] to use. Defaults to [RateLimitRetryPolicy]
    policy: P,
}

impl RetryBackoffLayer {
    /// Creates a new retry layer with the given parameters and the default [RateLimitRetryPolicy].
    pub const fn new(
        max_rate_limit_retries: u32,
        initial_backoff: u64,
        compute_units_per_second: u64,
    ) -> Self {
        Self {
            max_rate_limit_retries,
            initial_backoff,
            compute_units_per_second,
            avg_cost: DEFAULT_AVG_COST,
            policy: RateLimitRetryPolicy,
        }
    }

    /// Set the average cost of a request. Defaults to `17` CU
    /// The cost of requests are usually weighted and can vary from 10 CU to several 100 CU,
    /// cheaper requests are more common some example alchemy
    /// weights:
    /// - `eth_getStorageAt`: 17
    /// - `eth_getBlockByNumber`: 16
    /// - `eth_newFilter`: 20
    ///
    /// (coming from forking mode) assuming here that storage request will be the
    /// driver for Rate limits we choose `17` as the average cost
    /// of any request
    pub fn with_avg_unit_cost(mut self, avg_cost: u64) {
        self.avg_cost = avg_cost;
    }
}

impl<P: RetryPolicy> RetryBackoffLayer<P> {
    /// Creates a new retry layer with the given parameters and [RetryPolicy].
    pub const fn new_with_policy(
        max_rate_limit_retries: u32,
        initial_backoff: u64,
        compute_units_per_second: u64,
        policy: P,
    ) -> Self {
        Self {
            max_rate_limit_retries,
            initial_backoff,
            compute_units_per_second,
            policy,
            avg_cost: DEFAULT_AVG_COST,
        }
    }
}

/// [RateLimitRetryPolicy] implements [RetryPolicy] to determine whether to retry depending on the
/// err.
#[derive(Debug, Copy, Clone, Default)]
#[non_exhaustive]
pub struct RateLimitRetryPolicy;

/// [RetryPolicy] defines logic for which [TransportError] instances should
/// the client retry the request and try to recover from.
pub trait RetryPolicy: Send + Sync + std::fmt::Debug {
    /// Whether to retry the request based on the given `error`
    fn should_retry(&self, error: &TransportError) -> bool;

    /// Providers may include the `backoff` in the error response directly
    fn backoff_hint(&self, error: &TransportError) -> Option<std::time::Duration>;
}

impl RetryPolicy for RateLimitRetryPolicy {
    fn should_retry(&self, error: &TransportError) -> bool {
        error.is_retryable()
    }

    /// Provides a backoff hint if the error response contains it
    fn backoff_hint(&self, error: &TransportError) -> Option<std::time::Duration> {
        error.backoff_hint()
    }
}

impl<S, P: RetryPolicy + Clone> Layer<S> for RetryBackoffLayer<P> {
    type Service = RetryBackoffService<S, P>;

    fn layer(&self, inner: S) -> Self::Service {
        RetryBackoffService {
            inner,
            policy: self.policy.clone(),
            max_rate_limit_retries: self.max_rate_limit_retries,
            initial_backoff: self.initial_backoff,
            compute_units_per_second: self.compute_units_per_second,
            requests_enqueued: Arc::new(AtomicU32::new(0)),
            avg_cost: self.avg_cost,
        }
    }
}

/// A Tower Service used by the RetryBackoffLayer that is responsible for retrying requests based
/// on the error type. See [TransportError] and [RateLimitRetryPolicy].
#[derive(Debug, Clone)]
pub struct RetryBackoffService<S, P: RetryPolicy = RateLimitRetryPolicy> {
    /// The inner service
    inner: S,
    /// The [RetryPolicy] to use.
    policy: P,
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
    /// The number of compute units per second for this service
    compute_units_per_second: u64,
    /// The number of requests currently enqueued
    requests_enqueued: Arc<AtomicU32>,
    /// The average cost of a request.
    avg_cost: u64,
}

impl<S, P: RetryPolicy> RetryBackoffService<S, P> {
    const fn initial_backoff(&self) -> Duration {
        Duration::from_millis(self.initial_backoff)
    }
}

impl<S, P> Service<RequestPacket> for RetryBackoffService<S, P>
where
    S: Service<RequestPacket, Future = TransportFut<'static>, Error = TransportError>
        + Send
        + 'static
        + Clone,
    P: RetryPolicy + Clone + 'static,
{
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Our middleware doesn't care about backpressure, so it's ready as long
        // as the inner service is ready.
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        let inner = self.inner.clone();
        let this = self.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);
        Box::pin(async move {
            let ahead_in_queue = this.requests_enqueued.fetch_add(1, Ordering::SeqCst) as u64;
            let mut rate_limit_retry_number: u32 = 0;
            loop {
                let err;
                let res = inner.call(request.clone()).await;

                match res {
                    Ok(res) => {
                        if let Some(e) = res.as_error() {
                            err = TransportError::ErrorResp(e.clone())
                        } else {
                            this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                            return Ok(res);
                        }
                    }
                    Err(e) => err = e,
                }

                let should_retry = this.policy.should_retry(&err);
                if should_retry {
                    rate_limit_retry_number += 1;
                    if rate_limit_retry_number > this.max_rate_limit_retries {
                        return Err(TransportErrorKind::custom_str(&format!(
                            "Max retries exceeded {}",
                            err
                        )));
                    }
                    trace!(%err, "retrying request");

                    let current_queued_reqs = this.requests_enqueued.load(Ordering::SeqCst) as u64;

                    // try to extract the requested backoff from the error or compute the next
                    // backoff based on retry count
                    let backoff_hint = this.policy.backoff_hint(&err);
                    let next_backoff = backoff_hint.unwrap_or_else(|| this.initial_backoff());

                    let seconds_to_wait_for_compute_budget = compute_unit_offset_in_secs(
                        this.avg_cost,
                        this.compute_units_per_second,
                        current_queued_reqs,
                        ahead_in_queue,
                    );
                    let total_backoff = next_backoff
                        + std::time::Duration::from_secs(seconds_to_wait_for_compute_budget);

                    trace!(
                        total_backoff_millis = total_backoff.as_millis(),
                        budget_backoff_millis = seconds_to_wait_for_compute_budget * 1000,
                        default_backoff_millis = next_backoff.as_millis(),
                        backoff_hint_millis = backoff_hint.map(|d| d.as_millis()),
                        "(all in ms) backing off due to rate limit"
                    );

                    sleep(total_backoff).await;
                } else {
                    this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                    return Err(err);
                }
            }
        })
    }
}

/// Calculates an offset in seconds by taking into account the number of currently queued requests,
/// number of requests that were ahead in the queue when the request was first issued, the average
/// cost a weighted request (heuristic), and the number of available compute units per seconds.
///
/// Returns the number of seconds (the unit the remote endpoint measures compute budget) a request
/// is supposed to wait to not get rate limited. The budget per second is
/// `compute_units_per_second`, assuming an average cost of `avg_cost` this allows (in theory)
/// `compute_units_per_second / avg_cost` requests per seconds without getting rate limited.
/// By taking into account the number of concurrent request and the position in queue when the
/// request was first issued and determine the number of seconds a request is supposed to wait, if
/// at all
fn compute_unit_offset_in_secs(
    avg_cost: u64,
    compute_units_per_second: u64,
    current_queued_requests: u64,
    ahead_in_queue: u64,
) -> u64 {
    let request_capacity_per_second = compute_units_per_second.saturating_div(avg_cost).max(1);
    if current_queued_requests > request_capacity_per_second {
        current_queued_requests.min(ahead_in_queue).saturating_div(request_capacity_per_second)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_units_per_second() {
        let offset = compute_unit_offset_in_secs(17, 10, 0, 0);
        assert_eq!(offset, 0);
        let offset = compute_unit_offset_in_secs(17, 10, 2, 2);
        assert_eq!(offset, 2);
    }
}
