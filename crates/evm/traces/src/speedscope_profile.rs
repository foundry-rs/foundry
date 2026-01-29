//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the speedscope evented profile format.
//! Gas is used directly as the value unit (unit: "none"), so flame graph widths
//! represent gas consumption and the timeline shows gas usage over execution.

use crate::{
    decoder::precompiles::is_known_precompile,
    speedscope::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit},
};
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

    /// Current cumulative gas (used as timestamp).
    cumulative_gas: u64,

    /// Stack of (frame_index, open_gas) for tracking closes.
    open_frames: Vec<(usize, u64)>,
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
            open_frames: Vec::new(),
        }
    }

    /// Builds the final speedscope file.
    pub fn finish(mut self) -> SpeedscopeFile<'a> {
        // Close any remaining open frames at the final timestamp.
        while let Some((frame_idx, _)) = self.open_frames.pop() {
            self.profile.close_frame(frame_idx, self.cumulative_gas);
        }

        self.profile.set_end_value(self.cumulative_gas);
        self.file.add_profile(Profile::Evented(self.profile));
        self.file
    }

    /// Gets or creates a frame index for the given name and category.
    fn get_or_create_frame(&mut self, name: &str, category: FrameCategory) -> usize {
        let full_name = format!("{}{}", category.prefix(), name);
        if let Some(&idx) = self.frame_cache.get(&full_name) {
            return idx;
        }

        let idx = self.file.add_frame(Frame::new(Cow::Owned(full_name.clone())));
        self.frame_cache.insert(full_name, idx);
        idx
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
            let signature = node
                .trace
                .decoded
                .as_ref()
                .and_then(|dc| dc.call_data.as_ref())
                .map(|dc| &dc.signature)
                .unwrap_or(&selector);

            if let Some(label) = &contract_label {
                format!("{label}::{signature}")
            } else {
                signature.clone()
            }
        };

        // Open this function frame.
        let frame_idx = self.get_or_create_frame(&func_name, category);
        self.profile.open_frame(frame_idx, self.cumulative_gas);
        self.open_frames.push((frame_idx, self.cumulative_gas));

        // Track internal function step exits.
        let mut step_exits: Vec<(usize, usize)> = Vec::new(); // (end_step_idx, frame_idx)

        // Process children in order.
        // We need to look ahead to see if a Step is followed by a Call - if so, the step is a
        // CALL opcode whose gas_cost includes the subcall's gas, which we'll account for
        // separately.
        let ordering = &node.ordering;
        for (i, order) in ordering.iter().enumerate() {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);

                    // Check if next item is a Call - if so, this step is a CALL opcode
                    // and its gas_cost includes the subcall's gas.
                    let next_is_call =
                        matches!(ordering.get(i + 1), Some(TraceMemberOrder::Call(_)));

                    self.process_step(&node.trace.steps, *step_idx, &mut step_exits, next_is_call);
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        // Exit pending internal function calls.
        for (_, frame_idx) in step_exits.drain(..).rev() {
            self.profile.close_frame(frame_idx, self.cumulative_gas);
        }

        // Close this call frame.
        if let Some((frame_idx, _)) = self.open_frames.pop() {
            self.profile.close_frame(frame_idx, self.cumulative_gas);
        }
    }

    /// Processes a single step, handling internal function calls.
    ///
    /// `is_call_opcode` indicates this step is followed by a Call in the ordering,
    /// meaning it's a CALL/DELEGATECALL/etc opcode whose gas_cost includes the
    /// subcall's gas consumption. We skip adding that gas since the subcall will
    /// account for it.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<(usize, usize)>, // (end_step_idx, frame_idx)
        is_call_opcode: bool,
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls.
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            // func_name is already in format "Contract::function" from the debug trace identifier.
            let frame_idx =
                self.get_or_create_frame(&decoded_internal_call.func_name, FrameCategory::Internal);
            self.profile.open_frame(frame_idx, self.cumulative_gas);
            step_exits.push((*step_end_idx, frame_idx));
        }

        // Advance cumulative gas for this step, unless it's a CALL opcode.
        // CALL opcodes have gas_cost that includes the subcall's gas, which we
        // account for separately when processing the child call node.
        if !is_call_opcode {
            self.cumulative_gas = self.cumulative_gas.saturating_add(step.gas_cost);
        }
    }

    /// Exit all previous internal calls that should end before step_idx.
    fn exit_previous_steps(&mut self, step_exits: &mut Vec<(usize, usize)>, step_idx: usize) {
        // Collect frames to close (in reverse order for proper nesting).
        let mut to_close = Vec::new();
        step_exits.retain(|&(end_idx, frame_idx)| {
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
