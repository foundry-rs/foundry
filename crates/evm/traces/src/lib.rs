//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use serde::{Deserialize, Serialize};

pub use revm_inspectors::tracing::{
    types::{
        CallKind, CallLog, CallTrace, CallTraceNode, DecodedCallData, DecodedCallLog,
        DecodedCallTrace,
    },
    CallTraceArena, FourByteInspector, GethTraceBuilder, ParityTraceBuilder, StackSnapshotType,
    TraceWriter, TracingInspector, TracingInspectorConfig,
};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::{LocalTraceIdentifier, TraceIdentifier};

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

pub type Traces = Vec<(TraceKind, CallTraceArena)>;

/// Decode a collection of call traces.
///
/// The traces will be decoded using the given decoder, if possible.
pub async fn decode_trace_arena(
    arena: &mut CallTraceArena,
    decoder: &CallTraceDecoder,
) -> Result<(), std::fmt::Error> {
    decoder.prefetch_signatures(arena.nodes()).await;
    decoder.populate_traces(arena.nodes_mut()).await;

    Ok(())
}

/// Render a collection of call traces to a string.
pub fn render_trace_arena(arena: &CallTraceArena) -> String {
    let mut w = TraceWriter::new(Vec::<u8>::new());
    w.write_arena(arena).expect("Failed to write traces");
    String::from_utf8(w.into_writer()).expect("trace writer wrote invalid UTF-8")
}

/// Specifies the kind of trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    Deployment,
    Setup,
    Execution,
}

impl TraceKind {
    /// Returns `true` if the trace kind is [`Deployment`].
    ///
    /// [`Deployment`]: TraceKind::Deployment
    #[must_use]
    pub fn is_deployment(self) -> bool {
        matches!(self, Self::Deployment)
    }

    /// Returns `true` if the trace kind is [`Setup`].
    ///
    /// [`Setup`]: TraceKind::Setup
    #[must_use]
    pub fn is_setup(self) -> bool {
        matches!(self, Self::Setup)
    }

    /// Returns `true` if the trace kind is [`Execution`].
    ///
    /// [`Execution`]: TraceKind::Execution
    #[must_use]
    pub fn is_execution(self) -> bool {
        matches!(self, Self::Execution)
    }
}

/// Given a list of traces and artifacts, it returns a map connecting address to abi
pub fn load_contracts<'a>(
    traces: impl IntoIterator<Item = &'a CallTraceArena>,
    known_contracts: &ContractsByArtifact,
) -> ContractsByAddress {
    let mut local_identifier = LocalTraceIdentifier::new(known_contracts);
    let decoder = CallTraceDecoder::new();
    let mut contracts = ContractsByAddress::new();
    for trace in traces {
        for address in local_identifier.identify_addresses(decoder.trace_addresses(trace)) {
            if let (Some(contract), Some(abi)) = (address.contract, address.abi) {
                contracts.insert(address.address, (contract, abi.into_owned()));
            }
        }
    }
    contracts
}
