//! Firefox Profiler compatible profile generation for EVM execution traces.
//!
//! This module converts EVM call traces into the Firefox Profiler's processed profile format
//! using the `fxprof-processed-profile` crate from the samply project.
//!
//! Gas is encoded as time: 1 gas = 1 nanosecond (0.000001 ms). This makes the flame graph
//! widths represent gas consumption, and the timeline shows gas usage over execution.

use crate::decoder::precompiles::is_known_precompile;
use alloy_primitives::{Address, hex::ToHexExt};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use fxprof_processed_profile::{
    CategoryColor, CategoryPairHandle, CounterHandle, CpuDelta, Frame, FrameFlags, FrameInfo,
    ProcessHandle, Profile, ReferenceTimestamp, SamplingInterval, ThreadHandle, Timestamp,
};
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};
use std::time::SystemTime;

/// Gas to milliseconds conversion factor.
/// 1 gas = 1 nanosecond = 0.000001 milliseconds.
const GAS_TO_MS: f64 = 0.000_001;

/// Builds a Firefox Profiler compatible profile from a call trace arena.
///
/// - `arena`: The call trace arena containing the execution trace.
/// - `test_name`: Name of the test function (used as thread name).
/// - `contract_name`: Name of the contract being tested.
pub fn build(arena: &CallTraceArena, test_name: &str, contract_name: &str) -> Profile {
    let mut builder = EvmProfileBuilder::new(test_name, contract_name);
    if !arena.nodes().is_empty() {
        builder.process_call_node(arena.nodes(), 0);
    }
    builder.finish()
}

/// Categories for different types of calls in the EVM profile.
struct EvmCategories {
    /// Test contract calls (green).
    test: CategoryPairHandle,
    /// VM/cheatcode calls (purple).
    vm: CategoryPairHandle,
    /// Console logging calls (blue).
    console: CategoryPairHandle,
    /// Precompile calls (orange).
    precompile: CategoryPairHandle,
    /// External/library contract calls (yellow).
    external: CategoryPairHandle,
    /// Internal function calls (light green).
    internal: CategoryPairHandle,
}

/// A frame with its associated category.
struct StackFrame {
    name: String,
    category: CategoryPairHandle,
}

/// Builder for Firefox Profiler profiles from EVM traces.
struct EvmProfileBuilder {
    profile: Profile,
    process: ProcessHandle,
    thread: ThreadHandle,
    categories: EvmCategories,
    /// Address of the main test contract.
    test_address: Option<Address>,
    /// Current call stack (function names with categories).
    stack: Vec<StackFrame>,
    /// Current cumulative gas (used as pseudo-time in nanoseconds).
    cumulative_gas: u64,
    /// Current contract label (for prefixing internal function names).
    current_contract_label: Option<String>,
    /// Memory usage counter handle.
    memory_counter: CounterHandle,
    /// Previous memory size (for computing deltas).
    prev_memory_size: u64,
}

impl EvmProfileBuilder {
    fn new(test_name: &str, contract_name: &str) -> Self {
        let product = format!("Foundry EVM Profile: {contract_name}::{test_name}");
        let mut profile = Profile::new(
            &product,
            ReferenceTimestamp::from(SystemTime::now()),
            SamplingInterval::from_nanos(1), // 1 sample per nanosecond (= 1 gas)
        );

        // Set product name for metadata.
        profile.set_product(&product);

        // Create categories with distinct colors.
        let categories = EvmCategories {
            test: profile.add_category("Test", CategoryColor::Green).into(),
            vm: profile.add_category("VM", CategoryColor::Purple).into(),
            console: profile.add_category("Console", CategoryColor::Blue).into(),
            precompile: profile.add_category("Precompile", CategoryColor::Orange).into(),
            external: profile.add_category("External", CategoryColor::Yellow).into(),
            internal: profile.add_category("Internal", CategoryColor::LightGreen).into(),
        };

        let process =
            profile.add_process(contract_name, 1, Timestamp::from_millis_since_reference(0.0));
        let thread = profile.add_thread(
            process,
            1,
            Timestamp::from_millis_since_reference(0.0),
            true, // is_main
        );
        // Name thread after the test function.
        profile.set_thread_name(thread, test_name);

        // Create memory usage counter.
        let memory_counter =
            profile.add_counter(process, "Memory", "Memory", "EVM memory usage in bytes");

        Self {
            profile,
            process,
            thread,
            categories,
            test_address: None,
            stack: Vec::new(),
            cumulative_gas: 0,
            current_contract_label: None,
            memory_counter,
            prev_memory_size: 0,
        }
    }

    fn finish(mut self) -> Profile {
        // Set the end time based on total gas consumed.
        let end_time_ms = self.cumulative_gas as f64 * GAS_TO_MS;
        let end_time = Timestamp::from_millis_since_reference(end_time_ms);
        self.profile.set_thread_end_time(self.thread, end_time);
        self.profile.set_process_end_time(self.process, end_time);
        self.profile
    }

    /// Determine the category for a call based on its address.
    fn category_for_address(&self, address: Address) -> CategoryPairHandle {
        if address == CHEATCODE_ADDRESS {
            self.categories.vm
        } else if address == HARDHAT_CONSOLE_ADDRESS {
            self.categories.console
        } else if is_known_precompile(address, 1) {
            self.categories.precompile
        } else if Some(address) == self.test_address {
            self.categories.test
        } else {
            self.categories.external
        }
    }

    /// Process a call node and all its children.
    fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize) {
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

        // Save previous context to restore after processing this call.
        let prev_contract_label = self.current_contract_label.take();

        // Set current context for internal function name resolution.
        self.current_contract_label = contract_label.clone();

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

        // Enter this function.
        self.stack.push(StackFrame { name: func_name, category });

        // Track internal function step exits.
        let mut step_exits: Vec<usize> = Vec::new();

        // Process children in order.
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);
                    self.process_step(&node.trace.steps, *step_idx, &mut step_exits, category);
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        // Exit pending internal function calls.
        for _ in 0..step_exits.len() {
            self.stack.pop();
        }

        // Exit this call.
        self.stack.pop();

        // Restore previous context.
        self.current_contract_label = prev_contract_label;
    }

    /// Process a single step, handling internal function calls.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<usize>,
        _parent_category: CategoryPairHandle,
    ) {
        let step = &steps[step_idx];

        // Handle internal function calls.
        if let Some(decoded_step) = &step.decoded
            && let DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) =
                decoded_step.as_ref()
        {
            // Internal calls use the internal category but inherit context from parent.
            // Prefix with contract name if available.
            let func_name = if let Some(label) = &self.current_contract_label {
                format!("{label}::{}", decoded_internal_call.func_name)
            } else {
                decoded_internal_call.func_name.clone()
            };
            self.stack.push(StackFrame { name: func_name, category: self.categories.internal });
            step_exits.push(*step_end_idx);
        }

        // Add a sample for this opcode step.
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
    ///
    /// Gas is encoded as time: 1 gas = 1 nanosecond.
    /// The sample timestamp is the cumulative gas at the start of this step.
    /// The sample "duration" (time until next sample) represents the gas cost.
    fn add_sample(&mut self, step: &CallTraceStep) {
        let gas_cost = step.gas_cost;

        // Timestamp = cumulative gas in milliseconds (1 gas = 1 ns = 0.000001 ms).
        let timestamp_ms = self.cumulative_gas as f64 * GAS_TO_MS;
        let timestamp = Timestamp::from_millis_since_reference(timestamp_ms);

        // Build the stack frames from the current call stack.
        let frames: Vec<_> = self
            .stack
            .iter()
            .map(|frame| {
                let name_handle = self.profile.intern_string(&frame.name);
                FrameInfo {
                    frame: Frame::Label(name_handle),
                    category_pair: frame.category,
                    flags: FrameFlags::empty(),
                }
            })
            .collect();

        // Build the stack handle from outermost to innermost.
        let stack = self.profile.intern_stack_frames(self.thread, frames.into_iter());

        // Add the sample. Weight = 1 since we're encoding gas as time.
        self.profile.add_sample(self.thread, timestamp, stack, CpuDelta::ZERO, 1);

        // Add memory counter sample if memory data is available.
        if let Some(memory) = &step.memory {
            let memory_size = memory.len() as u64;
            let memory_delta = memory_size as i64 - self.prev_memory_size as i64;
            self.profile.add_counter_sample(
                self.memory_counter,
                timestamp,
                memory_delta as f64,
                if memory_delta != 0 { 1 } else { 0 },
            );
            self.prev_memory_size = memory_size;
        }

        // Advance cumulative gas.
        self.cumulative_gas = self.cumulative_gas.saturating_add(gas_cost);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_profile() {
        let arena = CallTraceArena::default();
        let profile = build(&arena, "testExample", "TestContract");
        let json = serde_json::to_string(&profile).unwrap();
        // Profile should be valid JSON with meta and threads.
        assert!(json.contains("\"meta\""));
        assert!(json.contains("\"threads\""));
        assert!(json.contains("Foundry EVM Profile"));
    }
}
