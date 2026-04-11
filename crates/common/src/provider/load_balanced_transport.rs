//! Load-balanced transport that distributes requests across multiple [`RuntimeTransport`]
//! instances using atomic round-robin selection.
//!
//! The [`LoadBalancedTransport`] sits between the `RetryBackoffLayer` and multiple
//! [`RuntimeTransport`] instances. When a request fails and the retry layer retries, it calls
//! this transport again — which round-robins to the next backend, providing free failover.

use crate::provider::runtime_transport::RuntimeTransport;
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportFut};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tower::Service;

/// A [`tower::Service`] that wraps multiple [`RuntimeTransport`] instances and distributes
/// requests across them using atomic round-robin selection.
///
/// # Panics
///
/// The constructor panics if `backends` is empty.
#[derive(Clone, Debug)]
pub struct LoadBalancedTransport {
    /// The underlying transports.
    backends: Vec<RuntimeTransport>,
    /// Atomic counter used for round-robin backend selection.
    next: Arc<AtomicUsize>,
}

impl LoadBalancedTransport {
    /// Creates a new [`LoadBalancedTransport`] from the given backends.
    ///
    /// # Panics
    ///
    /// Panics if `backends` is empty.
    pub fn new(backends: Vec<RuntimeTransport>) -> Self {
        assert!(!backends.is_empty(), "LoadBalancedTransport requires at least one backend");
        Self { backends, next: Arc::new(AtomicUsize::new(0)) }
    }

    /// Returns the number of backends.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Returns `true` if there are no backends.
    ///
    /// Note: this is always `false` for a successfully constructed [`LoadBalancedTransport`]
    /// because the constructor panics on empty input.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// Selects the next backend using atomic round-robin and returns a reference to it.
    ///
    /// The counter will eventually wrap at `usize::MAX`, but `%` ensures the index
    /// stays within bounds regardless (on 64-bit this takes ~585 years at 1B req/s).
    pub fn next_backend(&self) -> &RuntimeTransport {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.backends.len();
        &self.backends[idx]
    }
}

impl fmt::Display for LoadBalancedTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LoadBalancedTransport({} backends)", self.backends.len())
    }
}

impl Service<RequestPacket> for LoadBalancedTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.next_backend().request(req)
    }
}

impl Service<RequestPacket> for &LoadBalancedTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.next_backend().request(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::runtime_transport::RuntimeTransportBuilder;
    use url::Url;

    fn make_transport(url: &str) -> RuntimeTransport {
        RuntimeTransportBuilder::new(Url::parse(url).unwrap()).build()
    }

    #[test]
    fn round_robin_selection() {
        let t = LoadBalancedTransport::new(vec![
            make_transport("http://localhost:8545"),
            make_transport("http://localhost:8546"),
            make_transport("http://localhost:8547"),
        ]);

        // The counter starts at 0; each call to next_backend increments it.
        // We verify the counter advances by checking that three successive calls
        // wrap back around to the first backend on the fourth call.
        assert_eq!(t.next.load(Ordering::Relaxed), 0);
        let _ = t.next_backend(); // idx 0, counter → 1
        assert_eq!(t.next.load(Ordering::Relaxed), 1);
        let _ = t.next_backend(); // idx 1, counter → 2
        assert_eq!(t.next.load(Ordering::Relaxed), 2);
        let _ = t.next_backend(); // idx 2, counter → 3
        assert_eq!(t.next.load(Ordering::Relaxed), 3);

        // Fourth call wraps: 3 % 3 == 0 → first backend again
        let _ = t.next_backend(); // idx 0, counter → 4
        assert_eq!(t.next.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn single_backend() {
        let t = LoadBalancedTransport::new(vec![make_transport("http://localhost:8545")]);
        assert_eq!(t.len(), 1);
        assert!(!t.is_empty());

        // Every call must select index 0 (counter % 1 == 0 always).
        for i in 1..=5u64 {
            let _ = t.next_backend();
            assert_eq!(t.next.load(Ordering::Relaxed) as u64, i);
        }
    }

    #[test]
    #[should_panic(expected = "LoadBalancedTransport requires at least one backend")]
    fn empty_backends_panics() {
        let _ = LoadBalancedTransport::new(vec![]);
    }
}
