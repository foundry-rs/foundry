//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![warn(unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]

#[macro_use]
extern crate tracing;

use alloy_primitives::{Address, Bytes, Log as RawLog, B256, U256};
use ethers_core::types::{DefaultFrame, GethDebugTracingOptions, StructLog};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, debug::Instruction, utils::CallKind};
use foundry_utils::types::ToEthers;
use hashbrown::HashMap;
use itertools::Itertools;
use revm::interpreter::{opcode, CallContext, InstructionResult, SharedMemory, Stack};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};
use yansi::{Color, Paint};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::LocalTraceIdentifier;

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

pub use reth_revm_inspectors::tracing::{
    types::{CallTrace, CallTraceNode},
    CallTraceArena, StackSnapshotType, TracingInspector, TracingInspectorConfig,
};

pub mod utils;

pub type Traces = Vec<(TraceKind, CallTraceArena)>;

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

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

/// Chooses the color of the trace depending on the destination address and status of the call.
fn trace_color(trace: &CallTrace) -> Color {
    if trace.address == CHEATCODE_ADDRESS {
        Color::Blue
    } else if trace.success {
        Color::Green
    } else {
        Color::Red
    }
}

/// Given a list of traces and artifacts, it returns a map connecting address to abi
pub fn load_contracts(
    traces: Traces,
    known_contracts: Option<&ContractsByArtifact>,
) -> ContractsByAddress {
    let Some(contracts) = known_contracts else { return BTreeMap::new() };
    let mut local_identifier = LocalTraceIdentifier::new(contracts);
    let mut decoder = CallTraceDecoderBuilder::new().build();
    for (_, trace) in &traces {
        decoder.identify(trace, &mut local_identifier);
    }

    decoder
        .contracts
        .iter()
        .filter_map(|(addr, name)| {
            if let Ok(Some((_, (abi, _)))) = contracts.find_by_name_or_identifier(name) {
                return Some((*addr, (name.clone(), abi.clone())))
            }
            None
        })
        .collect()
}

/// creates the memory data in 32byte chunks
/// see <https://github.com/ethereum/go-ethereum/blob/366d2169fbc0e0f803b68c042b77b6b480836dbc/eth/tracers/logger/logger.go#L450-L452>
fn convert_memory(data: &[u8]) -> Vec<String> {
    let mut memory = Vec::with_capacity((data.len() + 31) / 32);
    for idx in (0..data.len()).step_by(32) {
        let len = std::cmp::min(idx + 32, data.len());
        memory.push(hex::encode(&data[idx..len]));
    }
    memory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_convert_memory() {
        let mut data = vec![0u8; 32];
        assert_eq!(
            convert_memory(&data),
            vec!["0000000000000000000000000000000000000000000000000000000000000000".to_string()]
        );
        data.extend(data.clone());
        assert_eq!(
            convert_memory(&data),
            vec![
                "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                "0000000000000000000000000000000000000000000000000000000000000000".to_string()
            ]
        );
    }
}
