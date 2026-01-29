//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the speedscope evented profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.
//!
//! The profile captures the execution timeline where each frame's width represents
//! its total gas consumption (including children).

use super::schema::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit};
use alloy_primitives::hex::ToHexExt;
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};
use std::{borrow::Cow, collections::HashMap};

/// Builds a speedscope profile from a call trace arena.
///
/// This builder walks the trace tree directly and creates open/close events
/// where each frame spans its total gas consumption (including children).
pub fn build<'a>(
    arena: &CallTraceArena,
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    let name = format!("{contract_name}::{test_name}");
    let mut builder = SpeedscopeBuilder::new(name);

    if !arena.nodes().is_empty() {
        builder.process_node(arena.nodes(), 0);
    }

    builder.finish()
}

/// Pending internal call info for later closing.
struct PendingInternalCall {
    /// Step index where this internal call ends.
    step_end_idx: usize,
    /// Frame index in the speedscope file.
    frame_idx: usize,
    /// Gas consumed by this internal call.
    gas_used: u64,
}

/// Builder that walks the trace tree and emits speedscope events.
struct SpeedscopeBuilder<'a> {
    file: SpeedscopeFile<'a>,
    profile: EventedProfile<'a>,
    /// Frame name -> frame index in shared frames.
    frame_cache: HashMap<String, usize>,
    /// Current position in the gas timeline.
    current_gas: u64,
}

impl<'a> SpeedscopeBuilder<'a> {
    fn new(name: String) -> Self {
        Self {
            file: SpeedscopeFile::new(name.clone()),
            profile: EventedProfile::new(name, ValueUnit::None),
            frame_cache: HashMap::new(),
            current_gas: 0,
        }
    }

    /// Gets or creates a frame index for the given name.
    fn get_or_create_frame(&mut self, name: String) -> usize {
        if let Some(&idx) = self.frame_cache.get(&name) {
            idx
        } else {
            let idx = self.file.add_frame(Frame::new(Cow::Owned(name.clone())));
            self.frame_cache.insert(name, idx);
            idx
        }
    }

    /// Processes a call trace node, returns the gas consumed.
    fn process_node(&mut self, nodes: &[CallTraceNode], idx: usize) -> u64 {
        let node = &nodes[idx];
        let total_gas = node.trace.gas_used;

        let func_name = Self::get_func_name(node);
        let frame_idx = self.get_or_create_frame(func_name);

        // Open this frame at current position.
        let open_at = self.current_gas;
        self.profile.open_frame(frame_idx, open_at);

        // Track internal function exits.
        let mut pending_calls: Vec<PendingInternalCall> = vec![];

        // Process children in execution order.
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    // Close all pending internal calls before an external call.
                    self.close_pending_calls(&mut pending_calls, 0, true);

                    let child_node_idx = node.children[*child_idx];
                    // Recursively process - this advances current_gas.
                    self.process_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    // Close internal calls that end before this step.
                    self.close_pending_calls(&mut pending_calls, *step_idx, false);

                    // Process this step (may open internal call frames).
                    self.process_step(&node.trace.steps, *step_idx, &mut pending_calls);
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        // Close any remaining internal calls.
        self.close_pending_calls(&mut pending_calls, 0, true);

        // Close this frame: advance to cover total gas and close.
        // The frame spans from open_at to open_at + total_gas.
        self.current_gas = open_at + total_gas;
        self.profile.close_frame(frame_idx, self.current_gas);

        total_gas
    }

    /// Gets the function name for a node.
    fn get_func_name(node: &CallTraceNode) -> String {
        if node.trace.kind.is_any_create() {
            let contract_name = node
                .trace
                .decoded
                .as_ref()
                .and_then(|dc| dc.label.as_deref())
                .unwrap_or("Contract");
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

            if let Some(label) = node.trace.decoded.as_ref().and_then(|dc| dc.label.as_ref()) {
                format!("{label}.{signature}")
            } else {
                signature.clone()
            }
        }
    }

    /// Processes a step, potentially opening an internal call frame.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        pending_calls: &mut Vec<PendingInternalCall>,
    ) {
        let step = &steps[step_idx];
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            let gas_used = steps[*step_end_idx].gas_used.saturating_sub(step.gas_used);

            // Open the internal call frame at current position.
            let frame_idx = self.get_or_create_frame(decoded_internal_call.func_name.clone());
            self.profile.open_frame(frame_idx, self.current_gas);

            // Track for later closing.
            pending_calls.push(PendingInternalCall {
                step_end_idx: *step_end_idx,
                frame_idx,
                gas_used,
            });
        }
    }

    /// Closes internal calls that should exit before the given step index.
    ///
    /// If `close_all` is true, closes all pending internal calls (used before external calls
    /// and at the end of a node). Otherwise, only closes calls whose end index is before
    /// `before_step_idx`.
    fn close_pending_calls(
        &mut self,
        pending_calls: &mut Vec<PendingInternalCall>,
        before_step_idx: usize,
        close_all: bool,
    ) {
        // Process in reverse order (LIFO) since calls are nested.
        while let Some(call) = pending_calls.last() {
            if close_all || call.step_end_idx < before_step_idx {
                let call = pending_calls.pop().unwrap();
                // Advance gas and close the frame.
                self.current_gas += call.gas_used;
                self.profile.close_frame(call.frame_idx, self.current_gas);
            } else {
                break;
            }
        }
    }

    /// Finishes building and returns the speedscope file.
    fn finish(mut self) -> SpeedscopeFile<'a> {
        self.profile.set_end_value(self.current_gas);
        self.file.add_profile(Profile::Evented(self.profile));
        self.file
    }
}

#[cfg(test)]
mod tests {
    use super::{super::schema::EventType, *};

    #[test]
    fn test_empty_profile() {
        let arena = CallTraceArena::default();
        let profile = build(&arena, "testExample", "TestContract");
        let json = serde_json::to_string(&profile).unwrap();

        assert!(
            json.contains("\"$schema\":\"https://www.speedscope.app/file-format-schema.json\"")
        );
        assert!(json.contains("\"name\":\"TestContract::testExample\""));
        assert!(json.contains("\"exporter\":\"foundry\""));
    }

    #[test]
    fn test_builder_manual_frames() {
        // Test the builder directly to verify timing logic.
        let mut builder = SpeedscopeBuilder::new("test".to_string());

        // Simulate: parent(500) containing child_a(100) and child_b(200)
        // Parent self-time: 200

        // Open parent at 0
        let parent_frame = builder.get_or_create_frame("parent".to_string());
        builder.profile.open_frame(parent_frame, 0);

        // Child A: 0-100
        let child_a_frame = builder.get_or_create_frame("child_a".to_string());
        builder.profile.open_frame(child_a_frame, 0);
        builder.current_gas = 100;
        builder.profile.close_frame(child_a_frame, 100);

        // Child B: 100-300
        let child_b_frame = builder.get_or_create_frame("child_b".to_string());
        builder.profile.open_frame(child_b_frame, 100);
        builder.current_gas = 300;
        builder.profile.close_frame(child_b_frame, 300);

        // Close parent at 500 (showing 200 gas self-time at end)
        builder.current_gas = 500;
        builder.profile.close_frame(parent_frame, 500);

        builder.profile.set_end_value(500);

        // Verify events
        let events = &builder.profile.events;
        assert_eq!(events.len(), 6); // 3 opens + 3 closes

        // Check parent spans full range
        assert_eq!(events[0].event_type, EventType::Open);
        assert_eq!(events[0].frame, parent_frame);
        assert_eq!(events[0].at, 0);

        assert_eq!(events[5].event_type, EventType::Close);
        assert_eq!(events[5].frame, parent_frame);
        assert_eq!(events[5].at, 500);

        // Child A: 0-100
        assert_eq!(events[1].event_type, EventType::Open);
        assert_eq!(events[1].frame, child_a_frame);
        assert_eq!(events[1].at, 0);

        assert_eq!(events[2].event_type, EventType::Close);
        assert_eq!(events[2].frame, child_a_frame);
        assert_eq!(events[2].at, 100);

        // Child B: 100-300
        assert_eq!(events[3].event_type, EventType::Open);
        assert_eq!(events[3].frame, child_b_frame);
        assert_eq!(events[3].at, 100);

        assert_eq!(events[4].event_type, EventType::Close);
        assert_eq!(events[4].frame, child_b_frame);
        assert_eq!(events[4].at, 300);
    }
}
