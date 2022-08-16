//! Task management support

use crate::shutdown::Shutdown;
use std::future::Future;
use tokio::{runtime::Handle, task::JoinHandle};

/// A helper struct for managing additional tokio tasks.
pub struct TaskManager {
    /// Tokio runtime handle that's used to spawn futures, See [tokio::runtime::Handle].
    tokio_handle: Handle,
    /// A receiver for the shutdown signal
    on_shutdown: Shutdown,
}

// === impl TaskManager ===

impl TaskManager {
    /// Creates a new instance of the task manager
    pub fn new(tokio_handle: Handle, on_shutdown: Shutdown) -> Self {
        Self { tokio_handle, on_shutdown }
    }

    /// Returns a receiver for the shutdown event
    pub fn on_shutdown(&self) -> Shutdown {
        self.on_shutdown.clone()
    }

    /// Spawns the given task.
    pub fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
        self.tokio_handle.spawn(async move { task.await })
    }

    /// Spawns the blocking task.
    pub fn spawn_blocking(&self, task: impl Future<Output = ()> + Send + 'static) {
        let handle = self.tokio_handle.clone();
        self.tokio_handle.spawn_blocking(move || {
            handle.block_on(task);
        });
    }
}
