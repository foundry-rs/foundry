//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use alloy_primitives::{hex, LogData};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::constants::CHEATCODE_ADDRESS;
use futures::{future::BoxFuture, FutureExt};
use revm::interpreter::OpCode;
use revm_inspectors::tracing::{types::TraceMemberOrder, OpcodeFilter};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use yansi::{Color, Paint};

pub use revm_inspectors::tracing::{
    types::{CallKind, CallTrace, CallTraceNode},
    CallTraceArena, FourByteInspector, GethTraceBuilder, ParityTraceBuilder, StackSnapshotType,
    TracingInspector, TracingInspectorConfig,
};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::{LocalTraceIdentifier, TraceIdentifier};

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

pub mod debug;

pub mod folded_stack_trace;
use folded_stack_trace::FoldedStackTrace;

pub type Traces = Vec<(TraceKind, CallTraceArena)>;

#[derive(Default, Debug, Eq, PartialEq)]
pub struct DecodedCallData {
    pub signature: String,
    pub args: Vec<String>,
}

#[derive(Default, Debug)]
pub struct DecodedCallTrace {
    pub label: Option<String>,
    pub return_data: Option<String>,
    pub func: Option<DecodedCallData>,
    pub contract: Option<String>,
}

#[derive(Debug)]
pub enum DecodedCallLog<'a> {
    /// A raw log.
    Raw(&'a LogData),
    /// A decoded log.
    ///
    /// The first member of the tuple is the event name, and the second is a vector of decoded
    /// parameters.
    Decoded(String, Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub struct DecodedTraceStep {
    pub start_step_idx: usize,
    pub end_step_idx: Option<usize>,
    pub function_name: String,
    pub inputs: Option<Vec<String>>,
    pub outputs: Option<Vec<String>>,
    pub gas_used: i64,
}

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

/// Render a collection of call traces.
///
/// The traces will be decoded using the given decoder, if possible.
pub async fn render_trace_arena<'a>(
    arena: &CallTraceArena,
    decoder: &CallTraceDecoder,
) -> Result<(String, Vec<String>), std::fmt::Error> {
    decoder.prefetch_signatures(arena.nodes()).await;

    let identified_internals = &decoder.identify_arena_steps(arena);

    #[allow(clippy::too_many_arguments)]
    fn render_items<'a>(
        arena: &'a [CallTraceNode],
        decoder: &'a CallTraceDecoder,
        identified_internals: &'a [Vec<DecodedTraceStep>],
        s: &'a mut String,
        folded_stack_traces: &'a mut FoldedStackTrace,
        node_idx: usize,
        mut ordering_idx: usize,
        internal_end_step_idx: Option<usize>,
        left: &'a str,
        right: &'a str,
    ) -> BoxFuture<'a, Result<usize, std::fmt::Error>> {
        async move {
            let node = &arena[node_idx];

            while ordering_idx < node.ordering.len() {
                let child = &node.ordering[ordering_idx];
                match child {
                    TraceMemberOrder::Log(index) => {
                        let log = render_trace_log(&node.logs[*index].raw_log, decoder).await?;

                        // Prepend our tree structure symbols to each line of the displayed log
                        log.lines().enumerate().try_for_each(|(i, line)| {
                            writeln!(
                                s,
                                "{}{}",
                                if i == 0 { left } else { right },
                                line
                            )
                        })?;
                    }
                    TraceMemberOrder::Call(index) => {
                        inner(
                            arena,
                            decoder,
                            &identified_internals,
                            s,
                            folded_stack_traces,
                            node.children[*index],
                            left,
                            right,
                        )
                        .await?;
                    }
                    TraceMemberOrder::Step(step_idx) => {
                        if let Some(internal_step_end_idx) = internal_end_step_idx {
                            if *step_idx >= internal_step_end_idx {
                                return Ok(ordering_idx);
                            }
                        }
                        if let Some(decoded) = identified_internals[node_idx]
                            .iter()
                            .find(|d| *step_idx == d.start_step_idx)
                        {
                            writeln!(
                                s,
                                "{left}[{}] {}{}",
                                decoded.gas_used,
                                decoded.function_name,
                                decoded
                                    .inputs
                                    .as_ref()
                                    .map(|v| format!("({})", v.join(", ")))
                                    .unwrap_or_default()
                            )?;
                            folded_stack_traces
                                .enter(decoded.function_name.to_string(), decoded.gas_used);
                            let left_prefix = format!("{right}{BRANCH}");
                            let right_prefix = format!("{right}{PIPE}");
                            ordering_idx = render_items(
                                arena,
                                decoder,
                                identified_internals,
                                s,
                                folded_stack_traces,
                                node_idx,
                                ordering_idx + 1,
                                decoded.end_step_idx,
                                &left_prefix,
                                &right_prefix,
                            )
                            .await?;

                            write!(s, "{right}{EDGE}{RETURN}")?;

                            if let Some(outputs) = &decoded.outputs {
                                write!(s, " {}", outputs.join(", "))?;
                            }

                            writeln!(s)?;
                            folded_stack_traces.exit();
                        }
                    }
                }
                ordering_idx += 1;
            }

            Ok(ordering_idx)
        }
        .boxed()
    }

    #[allow(clippy::too_many_arguments)]
    fn inner<'a>(
        arena: &'a [CallTraceNode],
        decoder: &'a CallTraceDecoder,
        identified_internals: &'a [Vec<DecodedTraceStep>],
        s: &'a mut String,
        folded_stack_traces: &'a mut FoldedStackTrace,
        idx: usize,
        left: &'a str,
        child: &'a str,
    ) -> BoxFuture<'a, Result<(), std::fmt::Error>> {
        async move {
            let node = &arena[idx];

            // Display trace header
            let (trace, return_data) =
                render_trace(&node.trace, decoder, folded_stack_traces).await?;
            writeln!(s, "{left}{trace}")?;

            // Display logs and subcalls
            render_items(
                arena,
                decoder,
                identified_internals,
                s,
                folded_stack_traces,
                idx,
                0,
                None,
                &format!("{child}{BRANCH}"),
                &format!("{child}{PIPE}"),
            )
            .await?;

            // Display trace return data
            let color = trace_color(&node.trace);
            write!(
                s,
                "{child}{EDGE}{}{}",
                RETURN.fg(color),
                format!("[{:?}] ", node.trace.status).fg(color)
            )?;
            match return_data {
                Some(val) => write!(s, "{val}"),
                None if node.trace.kind.is_any_create() => {
                    write!(s, "{} bytes of code", node.trace.output.len())
                }
                None if node.trace.output.is_empty() => Ok(()),
                None => write!(s, "{}", node.trace.output),
            }?;
            writeln!(s)?;

            folded_stack_traces.exit();
            Ok(())
        }
        .boxed()
    }

    let mut s = String::new();
    let mut folded_stack_traces = FoldedStackTrace::default();
    inner(
        arena.nodes(),
        decoder,
        identified_internals,
        &mut s,
        &mut folded_stack_traces,
        0,
        "  ",
        "  ",
    )
    .await?;

    let folded = folded_stack_traces.fold();
    Ok((s, folded))
}

/// Render a call trace.
///
/// The trace will be decoded using the given decoder, if possible.
pub async fn render_trace(
    trace: &CallTrace,
    decoder: &CallTraceDecoder,
    folded_stack_traces: &mut FoldedStackTrace,
) -> Result<(String, Option<String>), std::fmt::Error> {
    let mut s = String::new();
    write!(&mut s, "[{}] ", trace.gas_used)?;
    let address = trace.address.to_checksum(None);

    let decoded = decoder.decode_function(trace).await;
    if trace.kind.is_any_create() {
        write!(
            &mut s,
            "{}{} {}@{}",
            CALL.yellow(),
            "new".yellow(),
            decoded.label.as_deref().unwrap_or("<unknown>"),
            address
        )?;
        folded_stack_traces.enter(
            decoded.label.as_deref().unwrap_or("<unknown>").to_string(),
            trace.gas_used as i64,
        );
    } else {
        let (func_name, inputs) = match &decoded.func {
            Some(DecodedCallData { signature, args }) => {
                let name = signature.split('(').next().unwrap();
                (name.to_string(), args.join(", "))
            }
            None => {
                debug!(target: "evm::traces", trace=?trace, "unhandled raw calldata");
                if trace.data.len() < 4 {
                    ("fallback".to_string(), hex::encode(&trace.data))
                } else {
                    let (selector, data) = trace.data.split_at(4);
                    (hex::encode(selector), hex::encode(data))
                }
            }
        };

        let action = match trace.kind {
            CallKind::Call => "",
            CallKind::StaticCall => " [staticcall]",
            CallKind::CallCode => " [callcode]",
            CallKind::DelegateCall => " [delegatecall]",
            CallKind::Create | CallKind::Create2 => unreachable!(),
            CallKind::AuthCall => " [authcall]",
        };

        let color = trace_color(trace);
        write!(
            &mut s,
            "{addr}::{func_name}{opt_value}({inputs}){action}",
            addr = decoded.label.as_deref().unwrap_or(&address).fg(color),
            func_name = func_name.fg(color),
            opt_value = if trace.value.is_zero() {
                String::new()
            } else {
                format!("{{value: {}}}", trace.value)
            },
            action = action.yellow(),
        )?;
        folded_stack_traces.enter(
            format!("{addr}::{func_name}", addr = decoded.label.as_deref().unwrap_or(&address),),
            trace.gas_used as i64,
        );
    }

    Ok((s, decoded.return_data))
}

/// Render a trace log.
async fn render_trace_log(
    log: &LogData,
    decoder: &CallTraceDecoder,
) -> Result<String, std::fmt::Error> {
    let mut s = String::new();
    let decoded = decoder.decode_event(log).await;

    match decoded {
        DecodedCallLog::Raw(log) => {
            for (i, topic) in log.topics().iter().enumerate() {
                writeln!(
                    s,
                    "{:>13}: {}",
                    if i == 0 { "emit topic 0".to_string() } else { format!("topic {i}") },
                    format!("{topic:?}").cyan()
                )?;
            }

            write!(s, "          data: {}", hex::encode_prefixed(&log.data).cyan())?;
        }
        DecodedCallLog::Decoded(name, params) => {
            let params = params
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<String>>()
                .join(", ");

            write!(s, "emit {}({params})", name.cyan())?;
        }
    }

    Ok(s)
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

/// Different kinds of traces used by different foundry components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TraceMode {
    /// Disabled tracing.
    #[default]
    None,
    /// Simple call trace, no steps tracing required.
    Call,
    /// Call trace with tracing for JUMP and JUMPDEST opcode steps.
    ///
    /// Used for internal functions identification.
    Jump,
    /// Call trace with complete steps tracing.
    ///
    /// Used by debugger.
    Debug,
}

impl TraceMode {
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub const fn is_call(self) -> bool {
        matches!(self, Self::Call)
    }

    pub const fn is_jump(self) -> bool {
        matches!(self, Self::Jump)
    }

    pub const fn is_debug(self) -> bool {
        matches!(self, Self::Debug)
    }

    pub fn into_config(self) -> Option<TracingInspectorConfig> {
        if self.is_none() {
            None
        } else {
            TracingInspectorConfig {
                record_steps: self.is_debug() || self.is_jump(),
                record_memory_snapshots: self.is_debug() || self.is_jump(),
                record_stack_snapshots: if self.is_debug() || self.is_jump() {
                    StackSnapshotType::Full
                } else {
                    StackSnapshotType::None
                },
                record_logs: true,
                record_state_diff: false,
                record_returndata_snapshots: self.is_debug(),
                record_opcodes_filter: self
                    .is_jump()
                    .then(|| OpcodeFilter::new().enable(OpCode::JUMP).enable(OpCode::JUMPDEST)),
                exclude_precompile_calls: false,
            }
            .into()
        }
    }
}
