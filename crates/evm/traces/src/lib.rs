//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![warn(unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]

#[macro_use]
extern crate tracing;

use alloy_primitives::{Address, Bytes, Log as RawLog, B256, U256};
use ethers::types::{DefaultFrame, GethDebugTracingOptions, StructLog};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::{constants::CHEATCODE_ADDRESS, debug::Instruction, utils::CallKind};
use foundry_utils::types::ToEthers;
use hashbrown::HashMap;
use itertools::Itertools;
use node::CallTraceNode;
use revm::interpreter::{opcode, CallContext, InstructionResult, Memory, Stack};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::{self, Write},
};
use yansi::{Color, Paint};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::LocalTraceIdentifier;

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

mod inspector;
pub use inspector::Tracer;

pub mod node;
pub mod utils;

pub type Traces = Vec<(TraceKind, CallTraceArena)>;

/// An arena of [CallTraceNode]s
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallTraceArena {
    /// The arena of nodes
    pub arena: Vec<CallTraceNode>,
}

impl Default for CallTraceArena {
    fn default() -> Self {
        CallTraceArena { arena: vec![Default::default()] }
    }
}

impl CallTraceArena {
    /// Pushes a new trace into the arena, returning the trace ID
    pub fn push_trace(&mut self, entry: usize, new_trace: CallTrace) -> usize {
        match new_trace.depth {
            // The entry node, just update it
            0 => {
                self.arena[0].trace = new_trace;
                0
            }
            // We found the parent node, add the new trace as a child
            _ if self.arena[entry].trace.depth == new_trace.depth - 1 => {
                let id = self.arena.len();

                let trace_location = self.arena[entry].children.len();
                self.arena[entry].ordering.push(LogCallOrder::Call(trace_location));
                let node = CallTraceNode {
                    parent: Some(entry),
                    trace: new_trace,
                    idx: id,
                    ..Default::default()
                };
                self.arena.push(node);
                self.arena[entry].children.push(id);

                id
            }
            // We haven't found the parent node, go deeper
            _ => self.push_trace(
                *self.arena[entry].children.last().expect("Disconnected trace"),
                new_trace,
            ),
        }
    }

    pub fn addresses(&self) -> HashSet<(&Address, Option<&[u8]>)> {
        self.arena
            .iter()
            .map(|node| {
                if node.trace.created() {
                    if let TraceRetData::Raw(bytes) = &node.trace.output {
                        return (&node.trace.address, Some(bytes.as_ref()))
                    }
                }

                (&node.trace.address, None)
            })
            .collect()
    }

    // Recursively fill in the geth trace by going through the traces
    fn add_to_geth_trace(
        &self,
        storage: &mut HashMap<Address, BTreeMap<B256, B256>>,
        trace_node: &CallTraceNode,
        struct_logs: &mut Vec<StructLog>,
        opts: &GethDebugTracingOptions,
    ) {
        let mut child_id = 0;
        // Iterate over the steps inside the given trace
        for step in trace_node.trace.steps.iter() {
            let mut log: StructLog = step.into();

            // Fill in memory and storage depending on the options
            if !opts.disable_storage.unwrap_or_default() {
                let contract_storage = storage.entry(step.contract).or_default();
                if let Some((key, value)) = step.state_diff {
                    contract_storage.insert(B256::from(key), B256::from(value));
                    log.storage = Some(
                        contract_storage
                            .iter_mut()
                            .map(|t| (t.0.to_ethers(), t.1.to_ethers()))
                            .collect(),
                    );
                }
            }
            if opts.disable_stack.unwrap_or_default() {
                log.stack = None;
            }
            if !opts.enable_memory.unwrap_or_default() {
                log.memory = None;
            }

            // Add step to geth trace
            struct_logs.push(log);

            // Descend into a child trace if the step was a call
            match step.op {
                Instruction::OpCode(
                    opcode::CREATE |
                    opcode::CREATE2 |
                    opcode::DELEGATECALL |
                    opcode::CALL |
                    opcode::STATICCALL |
                    opcode::CALLCODE,
                ) => {
                    self.add_to_geth_trace(
                        storage,
                        &self.arena[trace_node.children[child_id]],
                        struct_logs,
                        opts,
                    );
                    child_id += 1;
                }
                _ => {}
            }
        }
    }

    /// Generate a geth-style trace e.g. for debug_traceTransaction
    pub fn geth_trace(
        &self,
        receipt_gas_used: U256,
        opts: GethDebugTracingOptions,
    ) -> DefaultFrame {
        if self.arena.is_empty() {
            return Default::default()
        }

        let mut storage = HashMap::new();
        // Fetch top-level trace
        let main_trace_node = &self.arena[0];
        let main_trace = &main_trace_node.trace;
        // Start geth trace
        let mut acc = DefaultFrame {
            // If the top-level trace succeeded, then it was a success
            failed: !main_trace.success,
            gas: receipt_gas_used.to_ethers(),
            return_value: main_trace.output.to_bytes().0.into(),
            ..Default::default()
        };

        self.add_to_geth_trace(&mut storage, main_trace_node, &mut acc.struct_logs, &opts);

        acc
    }
}

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

impl fmt::Display for CallTraceArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn inner(
            arena: &CallTraceArena,
            writer: &mut (impl Write + ?Sized),
            idx: usize,
            left: &str,
            child: &str,
            verbose: bool,
        ) -> fmt::Result {
            let node = &arena.arena[idx];

            // Display trace header
            if !verbose {
                writeln!(writer, "{left}{}", node.trace)?;
            } else {
                writeln!(writer, "{left}{:#}", node.trace)?;
            }

            // Display logs and subcalls
            let left_prefix = format!("{child}{BRANCH}");
            let right_prefix = format!("{child}{PIPE}");
            for child in &node.ordering {
                match child {
                    LogCallOrder::Log(index) => {
                        let mut log = String::new();
                        write!(log, "{}", node.logs[*index])?;

                        // Prepend our tree structure symbols to each line of the displayed log
                        log.lines().enumerate().try_for_each(|(i, line)| {
                            writeln!(
                                writer,
                                "{}{}",
                                if i == 0 { &left_prefix } else { &right_prefix },
                                line
                            )
                        })?;
                    }
                    LogCallOrder::Call(index) => {
                        inner(
                            arena,
                            writer,
                            node.children[*index],
                            &left_prefix,
                            &right_prefix,
                            verbose,
                        )?;
                    }
                }
            }

            // Display trace return data
            let color = trace_color(&node.trace);
            write!(writer, "{child}{EDGE}{}", color.paint(RETURN))?;
            if node.trace.created() {
                match &node.trace.output {
                    TraceRetData::Raw(bytes) => {
                        writeln!(writer, "{} bytes of code", bytes.len())?;
                    }
                    TraceRetData::Decoded(val) => {
                        writeln!(writer, "{val}")?;
                    }
                }
            } else {
                writeln!(writer, "{}", node.trace.output)?;
            }

            Ok(())
        }

        inner(self, f, 0, "  ", "  ", f.alternate())
    }
}

/// A raw or decoded log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawOrDecodedLog {
    /// A raw log
    Raw(RawLog),
    /// A decoded log.
    ///
    /// The first member of the tuple is the event name, and the second is a vector of decoded
    /// parameters.
    Decoded(String, Vec<(String, String)>),
}

impl fmt::Display for RawOrDecodedLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RawOrDecodedLog::Raw(log) => {
                for (i, topic) in log.topics().iter().enumerate() {
                    writeln!(
                        f,
                        "{:>13}: {}",
                        if i == 0 { "emit topic 0".to_string() } else { format!("topic {i}") },
                        Paint::cyan(format!("{topic:?}"))
                    )?;
                }

                write!(f, "          data: {}", Paint::cyan(hex::encode_prefixed(&log.data)))
            }
            RawOrDecodedLog::Decoded(name, params) => {
                let params = params
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<String>>()
                    .join(", ");

                write!(f, "emit {}({params})", Paint::cyan(name.clone()))
            }
        }
    }
}

/// Ordering enum for calls and logs
///
/// i.e. if Call 0 occurs before Log 0, it will be pushed into the `CallTraceNode`'s ordering before
/// the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogCallOrder {
    Log(usize),
    Call(usize),
}

/// Raw or decoded calldata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum TraceCallData {
    /// Raw calldata bytes.
    Raw(Bytes),
    /// Decoded calldata.
    Decoded {
        /// The function signature.
        signature: String,
        /// The function arguments.
        args: Vec<String>,
    },
}

impl Default for TraceCallData {
    fn default() -> Self {
        Self::Raw(Bytes::new())
    }
}

impl TraceCallData {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            TraceCallData::Raw(raw) => raw,
            TraceCallData::Decoded { .. } => &[],
        }
    }
}

/// Raw or decoded return data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum TraceRetData {
    /// Raw return data.
    Raw(Bytes),
    /// Decoded return data.
    Decoded(String),
}

impl Default for TraceRetData {
    fn default() -> Self {
        Self::Raw(Bytes::new())
    }
}

impl TraceRetData {
    /// Returns the data as [`Bytes`]
    pub fn to_bytes(&self) -> Bytes {
        match self {
            TraceRetData::Raw(raw) => raw.clone(),
            TraceRetData::Decoded(val) => val.as_bytes().to_vec().into(),
        }
    }

    pub fn to_raw(&self) -> Vec<u8> {
        self.to_bytes().to_vec()
    }
}

impl fmt::Display for TraceRetData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            TraceRetData::Raw(bytes) => {
                if bytes.is_empty() {
                    write!(f, "()")
                } else {
                    bytes.fmt(f)
                }
            }
            TraceRetData::Decoded(decoded) => f.write_str(decoded),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CallTraceStep {
    // Fields filled in `step`
    /// Call depth
    pub depth: u64,
    /// Program counter before step execution
    pub pc: usize,
    /// Opcode to be executed
    pub op: Instruction,
    /// Current contract address
    pub contract: Address,
    /// Stack before step execution
    pub stack: Stack,
    /// Memory before step execution
    pub memory: Memory,
    /// Remaining gas before step execution
    pub gas: u64,
    /// Gas refund counter before step execution
    pub gas_refund_counter: u64,

    // Fields filled in `step_end`
    /// Gas cost of step execution
    pub gas_cost: u64,
    /// Change of the contract state after step execution (effect of the SLOAD/SSTORE instructions)
    pub state_diff: Option<(U256, U256)>,
    /// Error (if any) after step execution
    pub error: Option<String>,
}

impl From<&CallTraceStep> for StructLog {
    fn from(step: &CallTraceStep) -> Self {
        StructLog {
            depth: step.depth,
            error: step.error.clone(),
            gas: step.gas,
            gas_cost: step.gas_cost,
            memory: Some(convert_memory(step.memory.data())),
            op: step.op.to_string(),
            pc: step.pc as u64,
            refund_counter: if step.gas_refund_counter > 0 {
                Some(step.gas_refund_counter)
            } else {
                None
            },
            stack: Some(step.stack.data().iter().copied().map(|v| v.to_ethers()).collect_vec()),
            // Filled in `CallTraceArena::geth_trace` as a result of compounding all slot changes
            storage: None,
        }
    }
}

/// A trace of a call.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CallTrace {
    /// The depth of the call
    pub depth: usize,
    /// Whether the call was successful
    pub success: bool,
    /// The name of the contract, if any.
    ///
    /// The format is `"<artifact>:<contract>"` for easy lookup in local contracts.
    ///
    /// This member is not used by the core call tracing functionality (decoding/displaying). The
    /// intended use case is for other components that may want to process traces by specific
    /// contracts (e.g. gas reports).
    pub contract: Option<String>,
    /// The label for the destination address, if any
    pub label: Option<String>,
    /// caller of this call
    pub caller: Address,
    /// The destination address of the call or the address from the created contract
    pub address: Address,
    /// The kind of call this is
    pub kind: CallKind,
    /// The value transferred in the call
    pub value: U256,
    /// The calldata for the call, or the init code for contract creations
    pub data: TraceCallData,
    /// The return data of the call if this was not a contract creation, otherwise it is the
    /// runtime bytecode of the created contract
    pub output: TraceRetData,
    /// The gas cost of the call
    pub gas_cost: u64,
    /// The status of the trace's call
    pub status: InstructionResult,
    /// call context of the runtime
    pub call_context: Option<CallContext>,
    /// Opcode-level execution steps
    pub steps: Vec<CallTraceStep>,
}

// === impl CallTrace ===

impl CallTrace {
    /// Whether this is a contract creation or not
    pub fn created(&self) -> bool {
        matches!(self.kind, CallKind::Create | CallKind::Create2)
    }
}

impl Default for CallTrace {
    fn default() -> Self {
        Self {
            depth: Default::default(),
            success: Default::default(),
            contract: Default::default(),
            label: Default::default(),
            caller: Default::default(),
            address: Default::default(),
            kind: Default::default(),
            value: Default::default(),
            data: Default::default(),
            output: Default::default(),
            gas_cost: Default::default(),
            status: InstructionResult::Continue,
            call_context: Default::default(),
            steps: Default::default(),
        }
    }
}

impl fmt::Display for CallTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let address = self.address.to_checksum(None);
        write!(f, "[{}] ", self.gas_cost)?;
        if self.created() {
            write!(
                f,
                "{}{} {}@{}",
                Paint::yellow(CALL),
                Paint::yellow("new"),
                self.label.as_deref().unwrap_or("<unknown>"),
                address
            )
        } else {
            let (func_name, inputs) = match &self.data {
                TraceCallData::Raw(bytes) => {
                    // We assume that the fallback function (`data.len() < 4`) counts as decoded
                    // calldata
                    let (selector, data) = bytes.split_at(4);
                    (hex::encode(selector), hex::encode(data))
                }
                TraceCallData::Decoded { signature, args } => {
                    let name = signature.split('(').next().unwrap();
                    (name.to_string(), args.join(", "))
                }
            };

            let action = match self.kind {
                // do not show anything for CALLs
                CallKind::Call => "",
                CallKind::StaticCall => " [staticcall]",
                CallKind::CallCode => " [callcode]",
                CallKind::DelegateCall => " [delegatecall]",
                CallKind::Create | CallKind::Create2 => unreachable!(),
            };

            let color = trace_color(self);
            write!(
                f,
                "{addr}::{func_name}{opt_value}({inputs}){action}",
                addr = color.paint(self.label.as_deref().unwrap_or(&address)),
                func_name = color.paint(func_name),
                opt_value = if self.value == U256::ZERO {
                    String::new()
                } else {
                    format!("{{value: {}}}", self.value)
                },
                action = Paint::yellow(action),
            )
        }
    }
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
