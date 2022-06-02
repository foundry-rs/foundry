//! Support for running multiple fork backend
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use crate::executor::fork::{database::ForkedDatabase, BackendHandler};
use ethers::{
    providers::{Http, Provider},
    types::BlockId,
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::{Fuse, Stream},
    task::{Context, Poll},
    Future, FutureExt,
};
use std::{collections::HashMap, pin::Pin};
use tracing::trace;

// TODO move some types from avil fork to evm

/// The identifier for a specific fork, this could be the name of the network a custom descriptive
/// name.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ForkId(pub String);

/// A database type that maintains multiple forks
#[derive(Debug, Clone)]
pub struct MutltiFork {
    /// Channel to send `Request`s to the handler
    handler: Sender<Request>,
    /// All created databases for forks identified by their `ForkId`
    forks: HashMap<ForkId, ForkedDatabase>,
    /// The currently active Database
    active: ForkId,
}

// === impl MultiFork ===

impl MutltiFork {
    /// Creates a new pair of `MutltiFork` and its handler `MutltiForkHandler`
    pub fn new(id: ForkId, db: ForkedDatabase) -> (MutltiFork, MutltiForkHandler) {
        todo!()
    }

    /// Creates a new pair and spawns the `MutltiForkHandler` on a background thread
    pub fn spawn(id: ForkId, db: ForkedDatabase) -> MutltiFork {
        todo!()
    }

    /// Returns the identifier of the currently active fork
    pub fn active_id(&self) -> &ForkId {
        &self.active
    }

    /// Returns the currently active database
    pub fn active(&self) -> &ForkedDatabase {
        &self.forks[self.active_id()]
    }
}

/// Request that's send to the handler
#[derive(Debug)]
enum Request {
    Create { fork_id: ForkId, endpoint: String, chain_id: Option<u64>, block: Option<BlockId> },
}

type RequestFuture = Pin<Box<dyn Future<Output = ()> + 'static + Send>>;

/// The type that manages connections in the background
pub struct MutltiForkHandler {
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

impl MutltiForkHandler {
    fn on_request(&mut self, req: Request) {}
}

// Drives all handler to completion
// This future will finish once all underlying BackendHandler are completed
impl Future for MutltiForkHandler {
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
            let mut request = pin.requests.swap_remove(n);
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
