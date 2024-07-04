//! User facing Logger

use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{subscriber::Interest, Metadata};
use tracing_subscriber::{layer::Context, Layer};

/// The target that identifies the events intended to be logged to stdout
pub(crate) const NODE_USER_LOG_TARGET: &str = "node::user";

/// The target that identifies the events coming from the `console.log` invocations.
pub(crate) const EVM_CONSOLE_LOG_TARGET: &str = "node::console";

/// A logger that listens for node related events and displays them.
///
/// This layer is intended to be used as filter for `NODE_USER_LOG_TARGET` events that will
/// eventually be logged to stdout
#[derive(Clone, Debug, Default)]
pub struct NodeLogLayer {
    state: LoggingManager,
}

impl NodeLogLayer {
    /// Returns a new instance of this layer
    pub fn new(state: LoggingManager) -> Self {
        Self { state }
    }
}

// use `Layer`'s filter function to globally enable/disable `NODE_USER_LOG_TARGET` events
impl<S> Layer<S> for NodeLogLayer
where
    S: tracing::Subscriber,
{
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.target() == NODE_USER_LOG_TARGET || metadata.target() == EVM_CONSOLE_LOG_TARGET
        {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        self.state.is_enabled() &&
            (metadata.target() == NODE_USER_LOG_TARGET ||
                metadata.target() == EVM_CONSOLE_LOG_TARGET)
    }
}

/// Contains the configuration of the logger
#[derive(Clone, Debug)]
pub struct LoggingManager {
    /// Whether the logger is currently enabled
    pub enabled: Arc<RwLock<bool>>,
}

impl LoggingManager {
    /// Returns true if logging is currently enabled
    pub fn is_enabled(&self) -> bool {
        *self.enabled.read()
    }

    /// Updates the `enabled` state
    pub fn set_enabled(&self, enabled: bool) {
        let mut current = self.enabled.write();
        *current = enabled;
    }
}

impl Default for LoggingManager {
    fn default() -> Self {
        Self { enabled: Arc::new(RwLock::new(true)) }
    }
}
