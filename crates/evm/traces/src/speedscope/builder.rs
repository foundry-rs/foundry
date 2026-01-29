//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the speedscope evented profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.

use super::schema::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit};
use crate::decoder::precompiles::is_known_precompile;
use alloy_primitives::{Address, hex::ToHexExt};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};
use std::{borrow::Cow, collections::HashMap};

/// Frame category for coloring in speedscope.
/// Encoded in the frame name as a prefix for visual distinction.
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
    /// Returns a color emoji prefix for visual distinction in speedscope.
    /// Speedscope doesn't have native category coloring, so we use unicode symbols.
    fn prefix(self) -> &'static str {
        match self {
            Self::Test => "🟢 ",       // Green
            Self::Vm => "🟣 ",         // Purple
            Self::Console => "🔵 ",    // Blue
            Self::Precompile => "🟠 ", // Orange
            Self::External => "🟡 ",   // Yellow
            Self::Internal => "⚪ ",   // Light/white for internal
        }
    }
}

/// Builder for speedscope profiles from EVM traces.
pub struct SpeedscopeProfileBuilder<'a> {
    file: SpeedscopeFile<'a>,
    profile: EventedProfile<'a>,

    /// Address of the main test contract.
    test_address: Option<Address>,

    /// Cache of frame names to frame indices.
    frame_cache: HashMap<String, usize>,

    /// Current cumulative gas (used as timestamp for event ordering).
    cumulative_gas: u64,
}

impl<'a> SpeedscopeProfileBuilder<'a> {
    /// Creates a new builder for the given test.
    pub fn new(test_name: &str, contract_name: &str) -> Self {
        let name = format!("{contract_name}::{test_name}");
        let file = SpeedscopeFile::new(name.clone());
        let profile = EventedProfile::new(name, ValueUnit::None);

        Self {
            file,
            profile,
            test_address: None,
            frame_cache: HashMap::new(),
            cumulative_gas: 0,
        }
    }

    /// Builds the final speedscope file.
    pub fn finish(mut self) -> SpeedscopeFile<'a> {
        self.profile.set_end_value(self.cumulative_gas);
        self.file.add_profile(Profile::Evented(self.profile));
        self.file
    }

    /// Gets or creates a frame index for the given name and category.
    fn get_or_create_frame(&mut self, name: &str, category: FrameCategory) -> usize {
        let full_name = format!("{}{}", category.prefix(), name);
        let file = &mut self.file;
        *self
            .frame_cache
            .entry(full_name.clone())
            .or_insert_with(|| file.add_frame(Frame::new(Cow::Owned(full_name))))
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

        // Open this function frame.
        let frame_idx = self.get_or_create_frame(&func_name, category);
        let open_gas = self.cumulative_gas;
        self.profile.open_frame(frame_idx, open_gas);

        // Track internal function step exits.
        let mut step_exits: Vec<(usize, usize, u64)> = Vec::new(); // (end_step_idx, frame_idx, open_gas)

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
                TraceMemberOrder::Log(_) => {}
            }
        }

        // Exit pending internal function calls.
        for (_, frame_idx, _) in step_exits.drain(..).rev() {
            self.profile.close_frame(frame_idx, self.cumulative_gas);
        }

        // Advance gas by this call's gas_used and close the frame.
        self.cumulative_gas += node.trace.gas_used;
        self.profile.close_frame(frame_idx, self.cumulative_gas);
    }

    /// Processes a single step, handling internal function calls.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<(usize, usize, u64)>, // (end_step_idx, frame_idx, open_gas)
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls.
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            // Calculate gas used by this internal call.
            let gas_used = steps[*step_end_idx].gas_used.saturating_sub(step.gas_used);

            let frame_idx =
                self.get_or_create_frame(&decoded_internal_call.func_name, FrameCategory::Internal);
            let open_gas = self.cumulative_gas;
            self.profile.open_frame(frame_idx, open_gas);
            step_exits.push((*step_end_idx, frame_idx, open_gas));

            // Advance cumulative gas by the internal call's gas.
            self.cumulative_gas += gas_used;
        }
    }

    /// Exit all previous internal calls that should end before step_idx.
    fn exit_previous_steps(
        &mut self,
        step_exits: &mut Vec<(usize, usize, u64)>,
        step_idx: usize,
    ) {
        // Collect frames to close (in reverse order for proper nesting).
        let mut to_close = Vec::new();
        step_exits.retain(|&(end_idx, frame_idx, _)| {
            if end_idx <= step_idx {
                to_close.push(frame_idx);
                false
            } else {
                true
            }
        });

        // Close frames in reverse order (LIFO).
        for frame_idx in to_close.into_iter().rev() {
            self.profile.close_frame(frame_idx, self.cumulative_gas);
        }
    }
}

/// Builds a speedscope profile from a call trace arena.
///
/// - `arena`: The call trace arena containing the execution trace.
/// - `test_name`: Name of the test function (used as profile name).
/// - `contract_name`: Name of the contract being tested.
pub fn build<'a>(
    arena: &CallTraceArena,
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    let mut builder = SpeedscopeProfileBuilder::new(test_name, contract_name);
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

        // Profile should be valid JSON with speedscope schema.
        assert!(
            json.contains("\"$schema\":\"https://www.speedscope.app/file-format-schema.json\"")
        );
        assert!(json.contains("\"name\":\"TestContract::testExample\""));
        assert!(json.contains("\"exporter\":\"foundry\""));
    }
}
