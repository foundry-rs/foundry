//! TUI debugger builder.

use crate::{identifier::DebugTraceIdentifierBuilder, Debugger};
use foundry_common::evm::Breakpoints;
use foundry_evm_core::debug::{DebugArena, DebugNodeFlat};

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    debug_arena: Vec<DebugNodeFlat>,
    /// Builder for [DebugTraceIdentifier].
    identifier: DebugTraceIdentifierBuilder,
    /// Map of the debugger breakpoints.
    breakpoints: Breakpoints,
}

impl DebuggerBuilder {
    /// Creates a new debugger builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures the [DebugTraceIdentifier].
    #[inline]
    pub fn identifier(
        mut self,
        f: impl FnOnce(DebugTraceIdentifierBuilder) -> DebugTraceIdentifierBuilder,
    ) -> Self {
        self.identifier = f(self.identifier);
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn debug_arenas(mut self, arena: &[DebugArena]) -> Self {
        for arena in arena {
            self = self.debug_arena(arena);
        }
        self
    }

    /// Extends the debug arena.
    #[inline]
    pub fn debug_arena(mut self, arena: &DebugArena) -> Self {
        arena.flatten_to(0, &mut self.debug_arena);
        self
    }

    /// Sets the breakpoints for the debugger.
    #[inline]
    pub fn breakpoints(mut self, breakpoints: Breakpoints) -> Self {
        self.breakpoints = breakpoints;
        self
    }

    /// Builds the debugger.
    #[inline]
    pub fn build(self) -> Debugger {
        let Self { debug_arena, identifier, breakpoints } = self;
        Debugger::new(debug_arena, identifier.build(), breakpoints)
    }
}
