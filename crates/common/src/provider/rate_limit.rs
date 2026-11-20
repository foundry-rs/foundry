//! A client-side rate-limiting transport layer that proactively paces requests.
//!
//! Alloy's [`RetryBackoffLayer`](alloy_transport::layers::RetryBackoffLayer) only
//! spaces requests *after* an endpoint has already answered with a rate-limit
//! error: the compute-units-per-second budget feeds its post-error backoff, not
//! the first attempt. Against a heavily rate-limited endpoint the first attempts
//! still go out as fast as the caller makes them and immediately draw `429`s
//! (see <https://github.com/foundry-rs/foundry/issues/9821>).
//!
//! [`CupsRateLimitLayer`] sits in front of the retry layer and paces the first
//! attempt of every request to stay under the same budget. It reuses alloy's
//! cost model: a budget of `compute_units_per_second` with an assumed average
//! request cost of [`DEFAULT_AVG_COST`] CU allows
//! `compute_units_per_second / avg_cost` requests per second, so requests are
//! spaced by the reciprocal of that rate.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportFut};
use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Instant, sleep};
use tower::{Layer, Service};

/// The average cost of a request in Compute Units (CU).
///
/// Matches `alloy_transport`'s `RetryBackoffLayer` default so proactive pacing
/// and reactive backoff assume the same per-request cost.
const DEFAULT_AVG_COST: u64 = 20;

/// Returns the minimum spacing between request dispatches for a given budget.
///
/// Mirrors alloy's capacity calculation: `compute_units_per_second / avg_cost`
/// requests per second, clamped to at least one, inverted to a per-request
/// interval.
fn min_interval(compute_units_per_second: u64, avg_cost: u64) -> Duration {
    let requests_per_second = compute_units_per_second.saturating_div(avg_cost.max(1)).max(1);
    Duration::from_secs(1) / requests_per_second.min(u32::MAX as u64) as u32
}

/// A [`tower`] layer that paces requests to stay under a compute-units-per-second
/// budget before they reach the transport.
#[derive(Clone, Debug)]
pub struct CupsRateLimitLayer {
    min_interval: Duration,
}

impl CupsRateLimitLayer {
    /// Creates a layer that paces requests to `compute_units_per_second`, using
    /// alloy's default average request cost of [`DEFAULT_AVG_COST`] CU.
    pub fn new(compute_units_per_second: u64) -> Self {
        Self::with_avg_cost(compute_units_per_second, DEFAULT_AVG_COST)
    }

    /// Creates a layer with an explicit average request cost in Compute Units.
    pub fn with_avg_cost(compute_units_per_second: u64, avg_cost: u64) -> Self {
        Self { min_interval: min_interval(compute_units_per_second, avg_cost) }
    }
}

impl<S> Layer<S> for CupsRateLimitLayer {
    type Service = CupsRateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CupsRateLimitService {
            inner,
            min_interval: self.min_interval,
            next_allowed: Arc::new(Mutex::new(None)),
        }
    }
}

/// The [`tower::Service`] produced by [`CupsRateLimitLayer`].
#[derive(Clone, Debug)]
pub struct CupsRateLimitService<S> {
    inner: S,
    /// Minimum spacing between request dispatches.
    min_interval: Duration,
    /// The earliest instant at which the next request may be dispatched.
    next_allowed: Arc<Mutex<Option<Instant>>>,
}

impl<S> Service<RequestPacket> for CupsRateLimitService<S>
where
    S: Service<
            RequestPacket,
            Response = ResponsePacket,
            Error = TransportError,
            Future = TransportFut<'static>,
        > + Clone
        + Send
        + 'static,
{
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        // Move the ready inner service into the future, leaving a clone behind.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let min_interval = self.min_interval;
        let next_allowed = self.next_allowed.clone();
        Box::pin(async move {
            // Reserve this request's dispatch slot `min_interval` after the
            // previously reserved one. The lock is released before awaiting.
            let wait = {
                let mut next_allowed = next_allowed.lock().expect("rate limiter poisoned");
                let now = Instant::now();
                let slot = match *next_allowed {
                    Some(prev) if prev > now => prev,
                    _ => now,
                };
                *next_allowed = Some(slot + min_interval);
                slot.saturating_duration_since(now)
            };
            if !wait.is_zero() {
                sleep(wait).await;
            }
            inner.call(request).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{Id, Request};
    use alloy_transport::TransportErrorKind;

    #[test]
    fn min_interval_matches_alloy_capacity() {
        // 330 CU/s at 20 CU/request => 16 req/s.
        assert_eq!(min_interval(330, DEFAULT_AVG_COST), Duration::from_secs(1) / 16);
        // 100 CU/s => 5 req/s => 200ms spacing.
        assert_eq!(min_interval(100, DEFAULT_AVG_COST), Duration::from_millis(200));
        // A budget below one request's cost still allows one request per second.
        assert_eq!(min_interval(1, DEFAULT_AVG_COST), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn paces_sequential_requests() {
        let inner = tower::service_fn(|_req: RequestPacket| -> TransportFut<'static> {
            Box::pin(async { Err(TransportErrorKind::custom_str("noop")) })
        });
        let layer = CupsRateLimitLayer::new(400); // 20 req/s => 50ms spacing.
        let interval = layer.min_interval;
        let mut service = layer.layer(inner);
        let request = RequestPacket::Single(
            Request::new("eth_blockNumber", Id::Number(1), ()).serialize().unwrap(),
        );

        let start = Instant::now();
        for _ in 0..3 {
            let _ = service.call(request.clone()).await;
        }

        // The first request dispatches immediately; the next two each wait a full
        // interval, so the three calls span at least two intervals.
        assert!(start.elapsed() >= interval * 2);
    }
}
