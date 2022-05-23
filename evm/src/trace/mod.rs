/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;

mod decoder;
pub mod node;
mod utils;

pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

use crate::{abi::CHEATCODE_ADDRESS, CallKind};
use ethers::{
    abi::{Address, RawLog},
    types::U256,
};
use node::CallTraceNode;
use revm::{CallContext, Return};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt::{self, Write},
};
use yansi::{Color, Paint};

/// An arena of [CallTraceNode]s
#[derive(Debug, Clone, Serialize, Deserialize)]
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
                let node = &mut self.arena[0];
                node.trace.update(new_trace);
                0
            }
            // We found the parent node, add the new trace as a child
            _ if self.arena[entry].trace.depth == new_trace.depth - 1 => {
                let id = self.arena.len();

                let trace_location = self.arena[entry].children.len();
                self.arena[entry].ordering.push(LogCallOrder::Call(trace_location));
                let node =
                    CallTraceNode { parent: Some(entry), trace: new_trace, ..Default::default() };
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

    pub fn addresses(&self) -> HashSet<(&Address, Option<&Vec<u8>>)> {
        self.arena
            .iter()
            .map(|node| {
                if node.trace.created() {
                    if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                        return (&node.trace.address, Some(bytes))
                    }
                }

                (&node.trace.address, None)
            })
            .collect()
    }
}

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

impl fmt::Display for CallTraceArena {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn inner(
            arena: &CallTraceArena,
            writer: &mut (impl Write + ?Sized),
            idx: usize,
            left: &str,
            child: &str,
        ) -> fmt::Result {
            let node = &arena.arena[idx];

            // Display trace header
            writeln!(writer, "{}{}", left, node.trace)?;

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
                        inner(arena, writer, node.children[*index], &left_prefix, &right_prefix)?;
                    }
                }
            }

            // Display trace return data
            let color = trace_color(&node.trace);
            write!(writer, "{}{}", child, EDGE)?;
            write!(writer, "{}", color.paint(RETURN))?;
            if node.trace.created() {
                if let RawOrDecodedReturnData::Raw(bytes) = &node.trace.output {
                    writeln!(writer, "{} bytes of code", bytes.len())?;
                } else {
                    unreachable!("We should never have decoded calldata for contract creations");
                }
            } else {
                writeln!(writer, "{}", node.trace.output)?;
            }

            Ok(())
        }

        inner(self, f, 0, "  ", "  ")
    }
}

/// A raw or decoded log.
#[derive(Debug, Clone, PartialEq)]
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RawOrDecodedLog::Raw(log) => {
                for (i, topic) in log.topics.iter().enumerate() {
                    writeln!(
                        f,
                        "{:>13}: {}",
                        if i == 0 { "emit topic 0".to_string() } else { format!("topic {i}") },
                        Paint::cyan(format!("0x{}", hex::encode(&topic)))
                    )?;
                }

                write!(
                    f,
                    "          data: {}",
                    Paint::cyan(format!("0x{}", hex::encode(&log.data)))
                )
            }
            RawOrDecodedLog::Decoded(name, params) => {
                let params = params
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<String>>()
                    .join(", ");

                write!(f, "emit {}({})", Paint::cyan(name.clone()), params)
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

// TODO: Maybe unify with output
/// Raw or decoded calldata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum RawOrDecodedCall {
    /// Raw calldata
    Raw(Vec<u8>),
    /// Decoded calldata.
    ///
    /// The first element in the tuple is the function name, and the second element is a vector of
    /// decoded parameters.
    Decoded(String, Vec<String>),
}

impl RawOrDecodedCall {
    pub fn to_raw(&self) -> Vec<u8> {
        match self {
            RawOrDecodedCall::Raw(raw) => raw.clone(),
            RawOrDecodedCall::Decoded(_, _) => {
                vec![]
            }
        }
    }
}

impl Default for RawOrDecodedCall {
    fn default() -> Self {
        RawOrDecodedCall::Raw(Vec::new())
    }
}

/// Raw or decoded return data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum RawOrDecodedReturnData {
    /// Raw return data
    Raw(Vec<u8>),
    /// Decoded return data
    Decoded(String),
}

impl RawOrDecodedReturnData {
    pub fn to_raw(&self) -> Vec<u8> {
        match self {
            RawOrDecodedReturnData::Raw(raw) => raw.clone(),
            RawOrDecodedReturnData::Decoded(val) => val.as_bytes().to_vec(),
        }
    }
}

impl Default for RawOrDecodedReturnData {
    fn default() -> Self {
        RawOrDecodedReturnData::Raw(Vec::new())
    }
}

impl fmt::Display for RawOrDecodedReturnData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self {
            RawOrDecodedReturnData::Raw(bytes) => {
                if bytes.is_empty() {
                    write!(f, "()")
                } else {
                    write!(f, "0x{}", hex::encode(&bytes))
                }
            }
            RawOrDecodedReturnData::Decoded(decoded) => write!(f, "{}", decoded.clone()),
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
    /// The destination address of the call
    pub address: Address,
    /// The kind of call this is
    pub kind: CallKind,
    /// The value transferred in the call
    pub value: U256,
    /// The calldata for the call, or the init code for contract creations
    pub data: RawOrDecodedCall,
    /// The return data of the call if this was not a contract creation, otherwise it is the
    /// runtime bytecode of the created contract
    pub output: RawOrDecodedReturnData,
    /// The gas cost of the call
    pub gas_cost: u64,
    /// The status of the trace's call
    pub status: Return,
    /// call context of the runtime
    pub call_context: Option<CallContext>,
}

// === impl CallTrace ===

impl CallTrace {
    /// Updates a trace given another trace
    fn update(&mut self, new_trace: Self) {
        self.success = new_trace.success;
        self.address = new_trace.address;
        self.kind = new_trace.kind;
        self.value = new_trace.value;
        self.data = new_trace.data;
        self.output = new_trace.output;
        self.address = new_trace.address;
        self.gas_cost = new_trace.gas_cost;
    }

    /// Whether this is a contract creation or not
    pub fn created(&self) -> bool {
        matches!(self.kind, CallKind::Create)
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
            status: Return::Continue,
            call_context: Default::default(),
        }
    }
}

impl fmt::Display for CallTrace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.created() {
            write!(
                f,
                "[{}] {}{} {}@{:?}",
                self.gas_cost,
                Paint::yellow(CALL),
                Paint::yellow("new"),
                self.label.as_ref().unwrap_or(&"<Unknown>".to_string()),
                self.address
            )?;
        } else {
            let (func, inputs) = match &self.data {
                RawOrDecodedCall::Raw(bytes) => {
                    // We assume that the fallback function (`data.len() < 4`) counts as decoded
                    // calldata
                    assert!(bytes.len() >= 4);
                    (hex::encode(&bytes[0..4]), hex::encode(&bytes[4..]))
                }
                RawOrDecodedCall::Decoded(func, inputs) => (func.clone(), inputs.join(", ")),
            };

            let action = match self.kind {
                // do not show anything for CALLs
                CallKind::Call => "",
                CallKind::StaticCall => "[staticcall]",
                CallKind::CallCode => "[callcode]",
                CallKind::DelegateCall => "[delegatecall]",
                _ => unreachable!(),
            };

            let color = trace_color(self);
            write!(
                f,
                "[{}] {}::{}{}({}) {}",
                self.gas_cost,
                color.paint(self.label.as_ref().unwrap_or(&self.address.to_string())),
                color.paint(func),
                if !self.value.is_zero() {
                    format!("{{value: {}}}", self.value)
                } else {
                    "".to_string()
                },
                inputs,
                Paint::yellow(action),
            )?;
        }

        Ok(())
    }
}

/// Specifies the kind of trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    Deployment,
    Setup,
    Execution,
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
