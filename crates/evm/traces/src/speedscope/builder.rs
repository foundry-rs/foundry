//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts folded stack trace entries into the speedscope evented profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.

use super::schema::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit};
use crate::folded_stack_trace::{self, TraceEntry};
use revm_inspectors::tracing::CallTraceArena;
use std::{borrow::Cow, collections::HashMap};

/// Builds a speedscope profile from a call trace arena.
///
/// Uses the same trace processing as --flamechart for consistent gas values.
pub fn build<'a>(
    arena: &CallTraceArena,
    test_name: &str,
    contract_name: &str,
    isolate: bool,
) -> SpeedscopeFile<'a> {
    let entries = folded_stack_trace::build_entries(arena, isolate);
    entries_to_speedscope(&entries, test_name, contract_name)
}

/// Converts trace entries to speedscope format.
///
/// Each entry represents a stack state with its self-time gas.
/// We convert these to open/close events for the speedscope evented format.
fn entries_to_speedscope<'a>(
    entries: &[TraceEntry],
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    let name = format!("{contract_name}::{test_name}");
    let mut file = SpeedscopeFile::new(name.clone());
    let mut profile = EventedProfile::new(name, ValueUnit::None);

    // Frame cache: name -> frame index.
    let mut frame_cache: HashMap<&str, usize> = HashMap::new();

    // Current cumulative gas (used as timestamp).
    let mut cumulative_gas: u64 = 0;

    // Current open stack (frame indices).
    let mut open_stack: Vec<usize> = Vec::new();

    for entry in entries {
        let stack = &entry.names;
        let gas = entry.gas;

        // Find common prefix length with current open stack.
        let common_len = open_stack
            .iter()
            .zip(stack.iter())
            .take_while(|(open_idx, name)| {
                frame_cache.get(name.as_str()).is_some_and(|idx| idx == *open_idx)
            })
            .count();

        // Close frames that are no longer in the stack (in reverse order).
        while open_stack.len() > common_len {
            let frame_idx = open_stack.pop().unwrap();
            profile.close_frame(frame_idx, cumulative_gas);
        }

        // Open new frames that are in this stack but not yet open.
        for name in stack.iter().skip(common_len) {
            let frame_idx = *frame_cache
                .entry(name.as_str())
                .or_insert_with(|| file.add_frame(Frame::new(Cow::Owned(name.clone()))));
            profile.open_frame(frame_idx, cumulative_gas);
            open_stack.push(frame_idx);
        }

        // Advance cumulative gas by this frame's gas consumption.
        cumulative_gas += gas;
    }

    // Close any remaining open frames.
    while let Some(frame_idx) = open_stack.pop() {
        profile.close_frame(frame_idx, cumulative_gas);
    }

    profile.set_end_value(cumulative_gas);
    file.add_profile(Profile::Evented(profile));
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_profile() {
        let arena = CallTraceArena::default();
        let profile = build(&arena, "testExample", "TestContract", false);
        let json = serde_json::to_string(&profile).unwrap();

        assert!(
            json.contains("\"$schema\":\"https://www.speedscope.app/file-format-schema.json\"")
        );
        assert!(json.contains("\"name\":\"TestContract::testExample\""));
        assert!(json.contains("\"exporter\":\"foundry\""));
    }

    #[test]
    fn test_entries_to_speedscope() {
        // Entries as they come from folded stack trace (after subtract_children):
        // Entry order is call order, gas is self-time.
        let entries = vec![
            TraceEntry { names: vec!["top".into()], gas: 200 },
            TraceEntry { names: vec!["top".into(), "child_a".into()], gas: 100 },
            TraceEntry { names: vec!["top".into(), "child_b".into()], gas: 150 },
        ];

        let file = entries_to_speedscope(&entries, "test", "Test");
        let json = serde_json::to_string_pretty(&file).unwrap();

        // Total gas should be 200 + 100 + 150 = 450
        assert!(json.contains("\"endValue\": 450"));
    }

    #[test]
    fn test_monotonic_events() {
        // Test that events are in strictly non-decreasing order.
        let entries = vec![
            TraceEntry { names: vec!["a".into()], gas: 100 },
            TraceEntry { names: vec!["a".into(), "b".into()], gas: 50 },
            TraceEntry { names: vec!["a".into(), "c".into()], gas: 75 },
        ];

        let file = entries_to_speedscope(&entries, "test", "Test");

        // Extract events
        if let Profile::Evented(profile) = &file.profiles[0] {
            let mut last_at = 0u64;
            for event in &profile.events {
                assert!(
                    event.at >= last_at,
                    "Event at {} is less than previous {}",
                    event.at,
                    last_at
                );
                last_at = event.at;
            }
        }
    }
}
