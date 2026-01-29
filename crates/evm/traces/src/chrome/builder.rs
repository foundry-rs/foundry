//! Chrome trace profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the Chrome Trace Event Format.
//! Gas is used as the time unit, so flame graph widths represent gas consumption.

use super::schema::{TraceEvent, TraceFile};
use crate::decoder::precompiles::is_known_precompile;
use alloy_primitives::{Address, hex::ToHexExt};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};

/// Frame category for coloring in Chrome trace viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FrameCategory {
    /// Test contract calls.
    Test,
    /// VM/cheatcode calls.
    Vm,
    /// Console logging calls.
    Console,
    /// Precompile calls.
    Precompile,
    /// External/library contract calls.
    External,
    /// Internal function calls.
    Internal,
}

impl FrameCategory {
    /// Returns the category string for Chrome trace.
    fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Vm => "vm",
            Self::Console => "console",
            Self::Precompile => "precompile",
            Self::External => "external",
            Self::Internal => "internal",
        }
    }
}

/// An open frame being tracked.
struct OpenFrame {
    name: String,
    category: FrameCategory,
    start_gas: u64,
}

/// Builder for Chrome trace profiles from EVM traces.
pub struct ChromeTraceBuilder<'a> {
    file: TraceFile<'a>,

    /// Address of the main test contract.
    test_address: Option<Address>,

    /// Current cumulative gas (used as timestamp).
    cumulative_gas: u64,

    /// Stack of open frames.
    open_frames: Vec<OpenFrame>,
}

impl<'a> ChromeTraceBuilder<'a> {
    /// Creates a new builder for the given test.
    pub fn new(_test_name: &str, _contract_name: &str) -> Self {
        Self {
            file: TraceFile::new(),
            test_address: None,
            cumulative_gas: 0,
            open_frames: Vec::new(),
        }
    }

    /// Builds the final Chrome trace file.
    pub fn finish(mut self) -> TraceFile<'a> {
        // Close any remaining open frames at the final timestamp.
        while let Some(frame) = self.open_frames.pop() {
            let dur = self.cumulative_gas.saturating_sub(frame.start_gas);
            self.file.add_event(TraceEvent::complete(
                frame.name,
                frame.category.as_str(),
                frame.start_gas,
                dur,
            ));
        }
        self.file
    }

    /// Determines the category for a call based on its address.
    fn category_for_address(&self, address: Address) -> FrameCategory {
        if address == CHEATCODE_ADDRESS {
            FrameCategory::Vm
        } else if address == HARDHAT_CONSOLE_ADDRESS {
            FrameCategory::Console
        } else if is_known_precompile(address, 1) {
            FrameCategory::Precompile
        } else if Some(address) == self.test_address {
            FrameCategory::Test
        } else {
            FrameCategory::External
        }
    }

    /// Processes a call node and all its children.
    pub fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize) {
        let node = &nodes[idx];
        let address = node.trace.address;

        // Set the test address from the first (root) call.
        if idx == 0 {
            self.test_address = Some(address);
        }

        // Determine category based on address.
        let category = self.category_for_address(address);

        // Extract contract label from decoded trace.
        let contract_label = node.trace.decoded.as_ref().and_then(|dc| dc.label.clone());

        // Build the function name for this call.
        let func_name = if node.trace.kind.is_any_create() {
            let contract_name = contract_label.as_deref().unwrap_or("Contract");
            format!("new {contract_name}")
        } else {
            let selector = node
                .selector()
                .map(|selector| selector.encode_hex_with_prefix())
                .unwrap_or_else(|| "fallback".to_string());
            // Extract just the function name from the signature (before the '(').
            let func_name = node
                .trace
                .decoded
                .as_ref()
                .and_then(|dc| dc.call_data.as_ref())
                .map(|dc| {
                    dc.signature.split_once('(').map(|(name, _)| name).unwrap_or(&dc.signature)
                })
                .unwrap_or(&selector);

            if let Some(label) = &contract_label {
                format!("{label}::{func_name}()")
            } else {
                format!("{func_name}()")
            }
        };

        // Push this frame onto the stack.
        self.open_frames.push(OpenFrame {
            name: func_name,
            category,
            start_gas: self.cumulative_gas,
        });

        // Track internal function step exits.
        let mut step_exits: Vec<(usize, OpenFrame)> = Vec::new();

        // Process children in order.
        let ordering = &node.ordering;
        for (i, order) in ordering.iter().enumerate() {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);

                    // Check if next item is a Call.
                    let next_is_call =
                        matches!(ordering.get(i + 1), Some(TraceMemberOrder::Call(_)));

                    self.process_step(&node.trace.steps, *step_idx, &mut step_exits, next_is_call);
                }
                TraceMemberOrder::Log(log_idx) => {
                    // Emit log as instant event.
                    if let Some(log) = node.logs.get(*log_idx) {
                        let log_name = log
                            .decoded
                            .as_ref()
                            .and_then(|d| d.name.as_ref())
                            .map(|s| s.as_str())
                            .unwrap_or("log");
                        self.file.add_event(TraceEvent::instant(
                            log_name.to_string(),
                            "log",
                            self.cumulative_gas,
                        ));
                    }
                }
            }
        }

        // Exit pending internal function calls.
        for (_, frame) in step_exits.drain(..).rev() {
            let dur = self.cumulative_gas.saturating_sub(frame.start_gas);
            self.file.add_event(TraceEvent::complete(
                frame.name,
                frame.category.as_str(),
                frame.start_gas,
                dur,
            ));
        }

        // Close this call frame.
        if let Some(frame) = self.open_frames.pop() {
            let dur = self.cumulative_gas.saturating_sub(frame.start_gas);
            self.file.add_event(TraceEvent::complete(
                frame.name,
                frame.category.as_str(),
                frame.start_gas,
                dur,
            ));
        }
    }

    /// Processes a single step, handling internal function calls.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<(usize, OpenFrame)>,
        is_call_opcode: bool,
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls.
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            step_exits.push((
                *step_end_idx,
                OpenFrame {
                    name: decoded_internal_call.func_name.clone(),
                    category: FrameCategory::Internal,
                    start_gas: self.cumulative_gas,
                },
            ));
        }

        // Advance cumulative gas for this step, unless it's a CALL opcode.
        if !is_call_opcode {
            self.cumulative_gas = self.cumulative_gas.saturating_add(step.gas_cost);
        }
    }

    /// Exit all previous internal calls that should end before step_idx.
    fn exit_previous_steps(&mut self, step_exits: &mut Vec<(usize, OpenFrame)>, step_idx: usize) {
        let mut to_close = Vec::new();
        step_exits.retain(|(end_idx, frame)| {
            if *end_idx <= step_idx {
                to_close.push(OpenFrame {
                    name: frame.name.clone(),
                    category: frame.category,
                    start_gas: frame.start_gas,
                });
                false
            } else {
                true
            }
        });

        // Close frames in reverse order (LIFO).
        for frame in to_close.into_iter().rev() {
            let dur = self.cumulative_gas.saturating_sub(frame.start_gas);
            self.file.add_event(TraceEvent::complete(
                frame.name,
                frame.category.as_str(),
                frame.start_gas,
                dur,
            ));
        }
    }
}

/// Builds a Chrome trace profile from a call trace arena.
///
/// - `arena`: The call trace arena containing the execution trace.
/// - `test_name`: Name of the test function.
/// - `contract_name`: Name of the contract being tested.
pub fn build<'a>(arena: &CallTraceArena, test_name: &str, contract_name: &str) -> TraceFile<'a> {
    let mut builder = ChromeTraceBuilder::new(test_name, contract_name);
    if !arena.nodes().is_empty() {
        builder.process_call_node(arena.nodes(), 0);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_profile() {
        let arena = CallTraceArena::default();
        let profile = build(&arena, "testExample", "TestContract");
        let json = serde_json::to_string(&profile).unwrap();

        assert!(json.contains("\"traceEvents\""));
    }
}
