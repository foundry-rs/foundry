//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts folded stack traces into the speedscope evented profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.

use super::schema::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit};
use crate::folded_stack_trace;
use revm_inspectors::tracing::CallTraceArena;
use std::{borrow::Cow, collections::HashMap};

/// Builds a speedscope profile from a call trace arena.
///
/// This converts the trace to folded stack format first (same as --flamechart),
/// then translates to speedscope's evented profile format.
pub fn build<'a>(
    arena: &CallTraceArena,
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    // Build folded stack trace (same as --flamechart uses).
    let folded = folded_stack_trace::build(arena);

    // Convert folded stack trace to speedscope format.
    folded_to_speedscope(&folded, test_name, contract_name)
}

/// Converts a folded stack trace to speedscope format.
///
/// Folded format: "func1;func2;func3 123" where 123 is gas consumed by func3 only.
/// Speedscope evented format: open/close events with cumulative timestamps.
fn folded_to_speedscope<'a>(
    folded: &[String],
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    let name = format!("{contract_name}::{test_name}");
    let mut file = SpeedscopeFile::new(name.clone());
    let mut profile = EventedProfile::new(name, ValueUnit::None);

    // Frame cache: name -> frame index.
    let mut frame_cache: HashMap<String, usize> = HashMap::new();

    // Current cumulative gas (used as timestamp).
    let mut cumulative_gas: u64 = 0;

    // Current open stack (frame indices).
    let mut open_stack: Vec<usize> = Vec::new();

    for line in folded {
        // Parse line: "func1;func2;func3 123"
        let Some((stack_part, gas_str)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(gas) = gas_str.parse::<u64>() else {
            continue;
        };

        // Parse the stack into function names.
        let stack: Vec<&str> = stack_part.split(';').collect();

        // Find common prefix length with current open stack.
        let common_len = open_stack
            .iter()
            .zip(stack.iter())
            .take_while(|(open_idx, name)| {
                frame_cache.get(&name.to_string()).is_some_and(|idx| idx == *open_idx)
            })
            .count();

        // Close frames that are no longer in the stack (in reverse order).
        while open_stack.len() > common_len {
            let frame_idx = open_stack.pop().unwrap();
            profile.close_frame(frame_idx, cumulative_gas);
        }

        // Open new frames that are in this stack but not yet open.
        for name in stack.iter().skip(common_len) {
            let frame_idx = *frame_cache.entry(name.to_string()).or_insert_with(|| {
                file.add_frame(Frame::new(Cow::Owned(name.to_string())))
            });
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
        let profile = build(&arena, "testExample", "TestContract");
        let json = serde_json::to_string(&profile).unwrap();

        assert!(
            json.contains("\"$schema\":\"https://www.speedscope.app/file-format-schema.json\"")
        );
        assert!(json.contains("\"name\":\"TestContract::testExample\""));
        assert!(json.contains("\"exporter\":\"foundry\""));
    }

    #[test]
    fn test_folded_to_speedscope() {
        let folded = vec![
            "top 200".to_string(),       // top consumes 200 (after child subtraction)
            "top;child_a 100".to_string(), // child_a consumes 100
            "top;child_b 150".to_string(), // child_b consumes 150
        ];

        let file = folded_to_speedscope(&folded, "test", "Test");
        let json = serde_json::to_string_pretty(&file).unwrap();

        // Total gas should be 200 + 100 + 150 = 450
        assert!(json.contains("\"endValue\": 450"));
    }
}
