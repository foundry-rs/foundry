//! Support for running multiple fork backend
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use crate::executor::{
    fork::{
        database::{ForkDbSnapshot, ForkedDatabase},
        BackendHandler, CreateFork, SharedBackend,
    },
    snapshot::Snapshots,
};
use ethers::{
    providers::{Http, Provider},
    types::BlockId,
};
use futures::{
    channel::mpsc::{Receiver, Sender},
    stream::{Fuse, Stream},
    task::{Context, Poll},
    Future, FutureExt,
};
use std::{collections::HashMap, pin::Pin};
use tracing::trace;

/// The identifier for a specific fork, this could be the name of the network a custom descriptive
/// name.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ForkId(pub String);

/// A database type that can maintain multiple forks
#[derive(Debug, Clone)]
pub struct MultiFork {
    /// Channel to send `Request`s to the handler
    handler: Sender<Request>,
    /// All created databases for forks identified by their `ForkId`
    forks: HashMap<ForkId, ForkedDatabase>,
}

// === impl MultiForkBackend ===

impl MultiFork {
    /// Creates a new pair of `MutltiFork` and its handler `MultiForkHandler`
    pub fn new(_id: ForkId, _db: ForkedDatabase) -> (MultiFork, MultiForkHandler) {
        todo!()
    }

    /// Creates a new pair and spawns the `MultiForkHandler` on a background thread
    pub fn spawn(_id: ForkId, _db: ForkedDatabase) -> MultiFork {
        todo!()
    }

    pub fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<ForkId> {
        todo!()
    }
}

/// Request that's send to the handler
#[derive(Debug)]
enum Request {
    Create(CreateFork),
}

type RequestFuture = Pin<Box<dyn Future<Output = ()> + 'static + Send>>;

/// The type that manages connections in the background
pub struct MultiForkHandler {
    /// Incoming requests from the `MultiFork`.
    incoming: Fuse<Receiver<Request>>,
    /// All active handlers
    ///
    /// It's expected that this list will be rather small (<10)
    handlers: Vec<(ForkId, BackendHandler<Provider<Http>>)>,
    // requests currently in progress
    requests: Vec<RequestFuture>,
}

// === impl MultiForkHandler ===

impl MultiForkHandler {
    fn on_request(&mut self, _req: Request) {}
}

// Drives all handler to completion
// This future will finish once all underlying BackendHandler are completed
impl Future for MultiForkHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // receive new requests
        loop {
            match Pin::new(&mut pin.incoming).poll_next(cx) {
                Poll::Ready(Some(req)) => {
                    pin.on_request(req);
                }
                Poll::Ready(None) => {
                    // channel closed, but we still need to drive the fork handlers to completion
                    trace!(target: "fork::multi", "request channel closed");
                    break
                }
                Poll::Pending => break,
            }
        }

        // advance all jobs
        for n in (0..pin.requests.len()).rev() {
            let _request = pin.requests.swap_remove(n);
            // TODO poll future
        }

        // advance all handlers
        for n in (0..pin.handlers.len()).rev() {
            let (id, mut handler) = pin.handlers.swap_remove(n);
            match handler.poll_unpin(cx) {
                Poll::Ready(_) => {
                    trace!(target: "fork::multi", "fork {:?} completed", id);
                }
                Poll::Pending => {
                    pin.handlers.push((id, handler));
                }
            }
        }

        if pin.handlers.is_empty() && pin.incoming.is_done() {
            trace!(target: "fork::multi", "completed");
            return Poll::Ready(())
        }

        Poll::Pending
    }
}
