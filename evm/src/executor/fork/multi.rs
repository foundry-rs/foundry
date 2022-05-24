//! Support for running multiple fork backend
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use crate::executor::fork::{database::ForkedDatabase, BackendHandler};
use ethers::providers::{Http, Provider};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::Stream,
    task::{Context, Poll},
    Future, FutureExt,
};
use std::{collections::HashMap, pin::Pin};

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

/// The type that manages connections in the background
#[derive(Debug)]
pub struct MutltiForkHandler {
    /// Incoming requests from the `MultiFork`.
    incoming: Receiver<Request>,
    /// All active handlers
    ///
    /// It's expected that this list will be rather small
    handlers: Vec<(ForkId, BackendHandler<Provider<Http>>)>,
}

// Drives all handler to completion
// This future will finish once all underlying BackendHandler are completed
impl Future for MutltiForkHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// Request that's send to the handler
#[derive(Debug)]
enum Request {}
