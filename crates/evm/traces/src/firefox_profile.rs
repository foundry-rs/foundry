//! Firefox Profiler compatible profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the Firefox Profiler's processed profile format
//! using the `fxprof-processed-profile` crate from the samply project.

use alloy_primitives::hex::ToHexExt;
use fxprof_processed_profile::{
    CategoryHandle, CpuDelta, Frame, FrameFlags, FrameInfo, ProcessHandle, Profile,
    ReferenceTimestamp, SamplingInterval, ThreadHandle, Timestamp,
};
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};

/// Builds a Firefox Profiler compatible profile from a call trace arena.
pub fn build(arena: &CallTraceArena, title: &str) -> Profile {
    let mut builder = EvmProfileBuilder::new(title);
    builder.process_call_node(arena.nodes(), 0);
    builder.finish()
}

/// Builder for Firefox Profiler profiles from EVM traces.
struct EvmProfileBuilder {
    profile: Profile,
    process: ProcessHandle,
    thread: ThreadHandle,
    /// Current call stack (function names).
    stack: Vec<String>,
    /// Current sample index (used as pseudo-timestamp).
    sample_idx: u64,
}

impl EvmProfileBuilder {
    fn new(title: &str) -> Self {
        let mut profile = Profile::new(
            title,
            ReferenceTimestamp::from_millis_since_unix_epoch(0.0),
            SamplingInterval::from_millis(1),
        );

        let process = profile.add_process("EVM", 1, Timestamp::from_millis_since_reference(0.0));
        let thread = profile.add_thread(
            process,
            1,
            Timestamp::from_millis_since_reference(0.0),
            true, // is_main
        );
        profile.set_thread_name(thread, "EVM Execution");

        Self { profile, process, thread, stack: Vec::new(), sample_idx: 0 }
    }

    fn finish(mut self) -> Profile {
        // Set the thread end time
        let end_time = Timestamp::from_millis_since_reference(self.sample_idx as f64);
        self.profile.set_thread_end_time(self.thread, end_time);
        self.profile.set_process_end_time(self.process, end_time);
        self.profile
    }

    /// Process a call node and all its children.
    fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize) {
        let node = &nodes[idx];

        // Build the function name for this call
        let func_name = if node.trace.kind.is_any_create() {
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
        };

        // Enter this function
        self.stack.push(func_name);

        // Track internal function step exits
        let mut step_exits: Vec<usize> = Vec::new();

        // Process children in order
        for order in &node.ordering {
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

        // Exit pending internal function calls
        for _ in 0..step_exits.len() {
            self.stack.pop();
        }

        // Exit this call
        self.stack.pop();
    }

    /// Process a single step, handling internal function calls.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<usize>,
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            self.stack.push(decoded_internal_call.func_name.clone());
            step_exits.push(*step_end_idx);
        }

        // Add a sample for this opcode step
        self.add_sample(step);
    }

    /// Exit all previous internal calls that should end before step_idx.
    fn exit_previous_steps(&mut self, step_exits: &mut Vec<usize>, step_idx: usize) {
        let initial_length = step_exits.len();
        step_exits.retain(|&end_idx| end_idx > step_idx);
        let num_exits = initial_length - step_exits.len();
        for _ in 0..num_exits {
            self.stack.pop();
        }
    }

    /// Add a sample for a single opcode step.
    fn add_sample(&mut self, step: &CallTraceStep) {
        let timestamp = Timestamp::from_millis_since_reference(self.sample_idx as f64);
        let gas_cost = step.gas_cost;

        // Build the stack frames from the current call stack (functions only, no opcodes).
        let frames: Vec<_> = self
            .stack
            .iter()
            .map(|func_name| {
                let name_handle = self.profile.intern_string(func_name);
                FrameInfo {
                    frame: Frame::Label(name_handle),
                    category_pair: CategoryHandle::OTHER.into(),
                    flags: FrameFlags::empty(),
                }
            })
            .collect();

        // Build the stack handle from outermost to innermost.
        let stack = self.profile.intern_stack_frames(self.thread, frames.into_iter());

        // Add the sample with weight = gas_cost.
        self.profile.add_sample(
            self.thread,
            timestamp,
            stack,
            CpuDelta::ZERO,
            gas_cost.try_into().unwrap_or(1),
        );

        self.sample_idx += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_profile() {
        let arena = CallTraceArena::default();
        let profile = build(&arena, "test");
        let json = serde_json::to_string(&profile).unwrap();
        // Profile should be valid JSON with meta and threads.
        assert!(json.contains("\"meta\""));
        assert!(json.contains("\"threads\""));
    }
}
