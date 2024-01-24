//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![warn(unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]

#[macro_use]
extern crate tracing;

use alloy_primitives::LogData;
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::constants::CHEATCODE_ADDRESS;
use futures::{future::BoxFuture, FutureExt};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Write};
use yansi::{Color, Paint};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::LocalTraceIdentifier;

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

use revm_inspectors::tracing::types::LogCallOrder;
pub use revm_inspectors::tracing::{
    types::{CallKind, CallTrace, CallTraceNode},
    CallTraceArena, FourByteInspector, GethTraceBuilder, ParityTraceBuilder, StackSnapshotType,
    TracingInspectorConfig,
};

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

const PIPE: &str = "  │ ";
const EDGE: &str = "  └─ ";
const BRANCH: &str = "  ├─ ";
const CALL: &str = "→ ";
const RETURN: &str = "← ";

/// Render a collection of call traces.
///
/// The traces will be decoded using the given decoder, if possible.
pub async fn render_trace_arena(
    arena: &CallTraceArena,
    decoder: &CallTraceDecoder,
) -> Result<String, std::fmt::Error> {
    decoder.prefetch_signatures(arena.nodes()).await;

    fn inner<'a>(
        arena: &'a [CallTraceNode],
        decoder: &'a CallTraceDecoder,
        s: &'a mut String,
        idx: usize,
        left: &'a str,
        child: &'a str,
    ) -> BoxFuture<'a, Result<(), std::fmt::Error>> {
        async move {
            let node = &arena[idx];

            // Display trace header
            let (trace, return_data) = render_trace(&node.trace, decoder).await?;
            writeln!(s, "{left}{}", trace)?;

            // Display logs and subcalls
            let left_prefix = format!("{child}{BRANCH}");
            let right_prefix = format!("{child}{PIPE}");
            for child in &node.ordering {
                match child {
                    LogCallOrder::Log(index) => {
                        let log = render_trace_log(&node.logs[*index], decoder).await?;

                        // Prepend our tree structure symbols to each line of the displayed log
                        log.lines().enumerate().try_for_each(|(i, line)| {
                            writeln!(
                                s,
                                "{}{}",
                                if i == 0 { &left_prefix } else { &right_prefix },
                                line
                            )
                        })?;
                    }
                    LogCallOrder::Call(index) => {
                        inner(
                            arena,
                            decoder,
                            s,
                            node.children[*index],
                            &left_prefix,
                            &right_prefix,
                        )
                        .await?;
                    }
                }
            }

            // Display trace return data
            let color = trace_color(&node.trace);
            write!(s, "{child}{EDGE}{}", color.paint(RETURN))?;
            if node.trace.kind.is_any_create() {
                match &return_data {
                    None => {
                        writeln!(s, "{} bytes of code", node.trace.data.len())?;
                    }
                    Some(val) => {
                        writeln!(s, "{val}")?;
                    }
                }
            } else {
                match &return_data {
                    None if node.trace.output.is_empty() => writeln!(s, "()")?,
                    None => writeln!(s, "{}", node.trace.output)?,
                    Some(val) => writeln!(s, "{val}")?,
                }
            }

            Ok(())
        }
        .boxed()
    }

    let mut s = String::new();
    inner(arena.nodes(), decoder, &mut s, 0, "  ", "  ").await?;
    Ok(s)
}

/// Render a call trace.
///
/// The trace will be decoded using the given decoder, if possible.
pub async fn render_trace(
    trace: &CallTrace,
    decoder: &CallTraceDecoder,
) -> Result<(String, Option<String>), std::fmt::Error> {
    let mut s = String::new();
    write!(&mut s, "[{}] ", trace.gas_used)?;
    let address = trace.address.to_checksum(None);

    let decoded = decoder.decode_function(trace).await;
    if trace.kind.is_any_create() {
        write!(
            &mut s,
            "{}{} {}@{}",
            Paint::yellow(CALL),
            Paint::yellow("new"),
            decoded.label.as_deref().unwrap_or("<unknown>"),
            address
        )?;
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
        };

        let color = trace_color(trace);
        write!(
            &mut s,
            "{addr}::{func_name}{opt_value}({inputs}){action}",
            addr = color.paint(decoded.label.as_deref().unwrap_or(&address)),
            func_name = color.paint(func_name),
            opt_value = if trace.value.is_zero() {
                String::new()
            } else {
                format!("{{value: {}}}", trace.value)
            },
            action = Paint::yellow(action),
        )?;
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
                    Paint::cyan(format!("{topic:?}"))
                )?;
            }

            write!(s, "          data: {}", Paint::cyan(hex::encode_prefixed(&log.data)))?;
        }
        DecodedCallLog::Decoded(name, params) => {
            let params = params
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<String>>()
                .join(", ");

            write!(s, "emit {}({params})", Paint::cyan(name.clone()))?;
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
                return Some((*addr, (name.clone(), abi.clone())));
            }
            None
        })
        .collect()
}
