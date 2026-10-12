//! Forge process execution context.

use crate::ForgeContext;
use std::sync::OnceLock;

/// Stores the forge execution context for the duration of the program.
pub static FORGE_CONTEXT: OnceLock<ForgeContext> = OnceLock::new();

/// Returns the current forge execution context, if it has been set.
pub fn current_execution_context() -> Option<ForgeContext> {
    FORGE_CONTEXT.get().copied()
}

/// Set `forge` command current execution context for the duration of the program.
/// Execution context is immutable, subsequent calls of this function won't change the context.
pub fn set_execution_context(context: ForgeContext) {
    let _ = FORGE_CONTEXT.set(context);
}
