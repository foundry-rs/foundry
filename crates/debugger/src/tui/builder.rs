//! TUI debugger builder.

use crate::Debugger;
use alloy_primitives::Address;
use foundry_common::{compile::ContractSources, evm::Breakpoints, get_contract_name};
use foundry_evm_core::debug::{DebugArena, DebugNodeFlat};
use foundry_evm_traces::CallTraceDecoder;
use std::collections::HashMap;

/// Debugger builder.
#[derive(Debug, Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct DebuggerBuilder {
    /// Debug traces returned from the EVM execution.
    debug_arena: Vec<DebugNodeFlat>,
    /// Identified contracts.
    identified_contracts: HashMap<Address, String>,
    /// Map of source files.
    sources: ContractSources,
    /// Map of the debugger breakpoints.
    breakpoints: Breakpoints,
}

impl DebuggerBuilder {
    /// Creates a new debugger builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
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

    /// Extends the identified contracts from multiple decoders.
    #[inline]
    pub fn decoders(mut self, decoders: &[CallTraceDecoder]) -> Self {
        for decoder in decoders {
            self = self.decoder(decoder);
        }
        self
    }

    /// Extends the identified contracts from a decoder.
    #[inline]
    pub fn decoder(self, decoder: &CallTraceDecoder) -> Self {
        let c = decoder.contracts.iter().map(|(k, v)| (*k, get_contract_name(v).to_string()));
        self.identified_contracts(c)
    }

    /// Extends the identified contracts.
    #[inline]
    pub fn identified_contracts(
        mut self,
        identified_contracts: impl IntoIterator<Item = (Address, String)>,
    ) -> Self {
        self.identified_contracts.extend(identified_contracts);
        self
    }

    /// Sets the sources for the debugger.
    #[inline]
    pub fn sources(mut self, sources: ContractSources) -> Self {
        self.sources = sources;
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
        let Self { debug_arena, identified_contracts, sources, breakpoints } = self;
        Debugger::new(debug_arena, identified_contracts, sources, breakpoints)
    }
}
