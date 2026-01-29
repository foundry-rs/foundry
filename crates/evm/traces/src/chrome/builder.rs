//! Chrome trace profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the Chrome Trace Event Format.
//! Gas consumption is used as the time unit, so flame graph widths represent gas usage.

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

/// Builder for Chrome trace profiles from EVM traces.
pub struct ChromeTraceBuilder<'a> {
    file: TraceFile<'a>,

    /// Address of the main test contract.
    test_address: Option<Address>,

    /// Current cumulative gas (used as timestamp for event ordering).
    cumulative_gas: u64,
}

impl<'a> ChromeTraceBuilder<'a> {
    /// Creates a new builder for the given test.
    pub fn new(_test_name: &str, _contract_name: &str) -> Self {
        Self { file: TraceFile::new(), test_address: None, cumulative_gas: 0 }
    }

    /// Builds the final Chrome trace file.
    pub fn finish(self) -> TraceFile<'a> {
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

        let start_gas = self.cumulative_gas;

        // Track internal function step exits.
        let mut step_exits: Vec<(usize, String, FrameCategory, u64, u64)> = Vec::new();
        // (end_step_idx, name, category, start_gas, gas_used)

        // Process children in order.
        let ordering = &node.ordering;
        for order in ordering {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);
                    self.process_step(&node.trace.steps, *step_idx, &mut step_exits);
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
        for (_, name, cat, ts, dur) in step_exits.drain(..).rev() {
            self.file.add_event(TraceEvent::complete(name, cat.as_str(), ts, dur));
        }

        // Advance gas by this call's gas_used and emit the complete event.
        self.cumulative_gas += node.trace.gas_used;
        self.file.add_event(TraceEvent::complete(
            func_name,
            category.as_str(),
            start_gas,
            node.trace.gas_used,
        ));
    }

    /// Processes a single step, handling internal function calls.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<(usize, String, FrameCategory, u64, u64)>,
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls.
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            // Calculate gas used by this internal call.
            let gas_used = steps[*step_end_idx].gas_used.saturating_sub(step.gas_used);

            let start_gas = self.cumulative_gas;
            step_exits.push((
                *step_end_idx,
                decoded_internal_call.func_name.clone(),
                FrameCategory::Internal,
                start_gas,
                gas_used,
            ));

            // Advance cumulative gas by the internal call's gas.
            self.cumulative_gas += gas_used;
        }
    }

    /// Exit all previous internal calls that should end before step_idx.
    fn exit_previous_steps(
        &mut self,
        step_exits: &mut Vec<(usize, String, FrameCategory, u64, u64)>,
        step_idx: usize,
    ) {
        // Collect frames to close (in reverse order for proper nesting).
        let mut to_close = Vec::new();
        step_exits.retain(|(end_idx, name, cat, ts, dur)| {
            if *end_idx <= step_idx {
                to_close.push((name.clone(), *cat, *ts, *dur));
                false
            } else {
                true
            }
        });

        // Close frames in reverse order (LIFO).
        for (name, cat, ts, dur) in to_close.into_iter().rev() {
            self.file.add_event(TraceEvent::complete(name, cat.as_str(), ts, dur));
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
