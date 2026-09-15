//! External trace identification scheduled as tests finish, ahead of rendering.

use crate::result::TestResult;
use alloy_primitives::map::AddressSet;
use foundry_common::ContractsByArtifact;
use foundry_evm::traces::{
    CallTraceDecoder,
    identifier::{ExternalPrefetcher, TraceIdentifier, TraceIdentifiers, unidentified_nodes},
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
        identifies_traces: IdentifiesTraces,
    ) -> Self {
        Self { external, decoder, known_contracts, identifies_traces }
    }

    /// Schedules lookups for the addresses in `result`'s traces that local identification
    /// cannot resolve.
    pub fn prefetch(&self, result: &TestResult) {
        if !(self.identifies_traces)(result) {
            return;
        }
        let mut local = TraceIdentifiers::new().with_local(&self.known_contracts);
        let mut addresses = AddressSet::default();
        for (_, arena) in &result.traces {
            let nodes = self.decoder.unidentified_nodes(&arena.arena);
            let identities = local.identify_addresses(&nodes);
            addresses.extend(
                unidentified_nodes(&nodes, &identities).iter().map(|node| node.trace.address),
            );
        }
        self.external.prefetch(addresses);
    }
}
