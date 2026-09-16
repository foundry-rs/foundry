//! External trace identification scheduled as tests finish, ahead of rendering.

use crate::result::TestResult;
use alloy_primitives::map::AddressSet;
use foundry_common::ContractsByArtifact;
use foundry_evm::traces::{
    CallTraceDecoder,
    identifier::{ExternalPrefetcher, TraceIdentifiers},
};

/// Decides whether a finished test's traces get identified when its suite is rendered.
pub type IdentifiesTraces = Box<dyn Fn(&TestResult) -> bool + Send + Sync>;

/// Starts external identification for a test's traces as soon as the test finishes, so the
/// lookups overlap with the rest of the run instead of blocking trace rendering.
///
/// Only addresses the renderer would ask the external identifier about are scheduled: the
/// decoder's node filter and local identification run here first, exactly as they do at render
/// time.
pub struct TracePrefetcher {
    external: ExternalPrefetcher,
    /// Applies the same node filter as the rendering decoder.
    decoder: CallTraceDecoder,
    known_contracts: ContractsByArtifact,
    identify_from_bytecodes: bool,
    identifies_traces: IdentifiesTraces,
}

impl std::fmt::Debug for TracePrefetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracePrefetcher").field("external", &self.external).finish_non_exhaustive()
    }
}

impl TracePrefetcher {
    /// Creates a prefetcher that schedules lookups on `external` for tests accepted by
    /// `identifies_traces`.
    pub fn new(
        external: ExternalPrefetcher,
        decoder: CallTraceDecoder,
        known_contracts: ContractsByArtifact,
        identify_from_bytecodes: bool,
        identifies_traces: IdentifiesTraces,
    ) -> Self {
        Self { external, decoder, known_contracts, identify_from_bytecodes, identifies_traces }
    }

    /// Schedules lookups for the addresses in `result`'s traces that local identification
    /// cannot resolve.
    pub fn prefetch(&self, result: &TestResult) {
        if !(self.identifies_traces)(result) {
            return;
        }
        let mut decoder = self.decoder.clone();
        decoder
            .labels
            .extend(result.labels.iter().map(|(address, label)| (*address, label.clone())));
        let mut local = if !self.identify_from_bytecodes || result.debug_bytecodes.is_empty() {
            TraceIdentifiers::new().with_local(&self.known_contracts)
        } else {
            TraceIdentifiers::new()
                .with_local_and_bytecodes(&self.known_contracts, &result.debug_bytecodes)
        };
        let mut addresses = AddressSet::default();
        for (_, arena) in &result.traces {
            decoder.identify(&arena.arena, &mut local);
            addresses.extend(
                decoder.unidentified_nodes(&arena.arena).iter().map(|node| node.trace.address),
            );
        }
        self.external.prefetch(addresses);
    }
}
