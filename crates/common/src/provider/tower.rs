//! Alloy-related tower middleware for retrying rate-limited requests
//! and applying backoff.
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut};
use tower::Service;

use super::{
    retry::{RateLimitRetryPolicy, RetryPolicy},
    runtime_transport::RuntimeTransport,
};

/// An Alloy Tower Layer that is responsible for retrying requests based on the
/// error type. See [TransportError].
#[derive(Debug, Clone)]
pub struct RetryBackoffLayer {
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The maximum number of retries for timeout errors
    max_timeout_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
    /// The number of compute units per second for this provider
    compute_units_per_second: u64,
}

impl RetryBackoffLayer {
    /// Creates a new [RetryWithPolicyLayer] with the given parameters
    pub fn new(
        max_rate_limit_retries: u32,
        max_timeout_retries: u32,
        initial_backoff: u64,
        compute_units_per_second: u64,
    ) -> Self {
        Self {
            max_rate_limit_retries,
            max_timeout_retries,
            initial_backoff,
            compute_units_per_second,
        }
    }
}

impl<S> tower::layer::Layer<S> for RetryBackoffLayer {
    type Service = RetryBackoffService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RetryBackoffService {
            inner,
            policy: RateLimitRetryPolicy,
            max_rate_limit_retries: self.max_rate_limit_retries,
            max_timeout_retries: self.max_timeout_retries,
            initial_backoff: self.initial_backoff,
            compute_units_per_second: self.compute_units_per_second,
            requests_enqueued: Arc::new(AtomicU32::new(0)),
        }
    }
}

/// An Alloy Tower Service that is responsible for retrying requests based on the
/// error type. See [TransportError] and [RetryWithPolicyLayer].
#[derive(Debug, Clone)]
pub struct RetryBackoffService<S> {
    /// The inner service
    inner: S,
    /// The retry policy
    policy: RateLimitRetryPolicy,
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The maximum number of retries for timeout errors
    max_timeout_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
    /// The number of compute units per second for this service
    compute_units_per_second: u64,
    /// The number of requests currently enqueued
    requests_enqueued: Arc<AtomicU32>,
}

// impl tower service
impl Service<RequestPacket> for RetryBackoffService<RuntimeTransport> {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Our middleware doesn't care about backpressure, so it's ready as long
        // as the inner service is ready.
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        let mut this = self.clone();
        Box::pin(async move {
            let ahead_in_queue = this.requests_enqueued.fetch_add(1, Ordering::SeqCst) as u64;
            let mut rate_limit_retry_number: u32 = 0;
            let mut timeout_retries: u32 = 0;
            loop {
                let err;
                let fut = this.inner.call(request.clone()).await;

                match fut {
                    Ok(res) => {
                        this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                        return Ok(res)
                    }
                    Err(e) => err = e,
                }

                let err = TransportError::from(err);
                let should_retry = this.policy.should_retry(&err);
                if should_retry {
                    rate_limit_retry_number += 1;
                    if rate_limit_retry_number > this.max_rate_limit_retries {
                        return Err(TransportErrorKind::custom_str("Max retries exceeded"))
                    }

                    let current_queued_reqs = this.requests_enqueued.load(Ordering::SeqCst) as u64;

                    // try to extract the requested backoff from the error or compute the next
                    // backoff based on retry count
                    let mut next_backoff = this
                        .policy
                        .backoff_hint(&err)
                        .unwrap_or_else(|| std::time::Duration::from_millis(this.initial_backoff));

                    // requests are usually weighted and can vary from 10 CU to several 100 CU,
                    // cheaper requests are more common some example alchemy
                    // weights:
                    // - `eth_getStorageAt`: 17
                    // - `eth_getBlockByNumber`: 16
                    // - `eth_newFilter`: 20
                    //
                    // (coming from forking mode) assuming here that storage request will be the
                    // driver for Rate limits we choose `17` as the average cost
                    // of any request
                    const AVG_COST: u64 = 17u64;
                    let seconds_to_wait_for_compute_budget = compute_unit_offset_in_secs(
                        AVG_COST,
                        this.compute_units_per_second,
                        current_queued_reqs,
                        ahead_in_queue,
                    );
                    next_backoff +=
                        std::time::Duration::from_secs(seconds_to_wait_for_compute_budget);

                    tokio::time::sleep(next_backoff).await;
                } else {
                    if timeout_retries < this.max_timeout_retries {
                        timeout_retries += 1;
                        continue;
                    }

                    this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                    return Err(TransportErrorKind::custom_str("Max retries exceeded"))
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
    let request_capacity_per_second = compute_units_per_second.saturating_div(avg_cost);
    if current_queued_requests > request_capacity_per_second {
        current_queued_requests.min(ahead_in_queue).saturating_div(request_capacity_per_second)
    } else {
        0
    }
}
