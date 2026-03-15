//! Chrome trace profile generation for EVM execution traces.
//!
//! This module converts folded stack traces into the Chrome Trace Event Format.
//! Gas consumption is used as the time unit, so flame graph widths represent gas usage.

use super::schema::{TraceEvent, TraceFile};
use crate::folded_stack_trace;
use revm_inspectors::tracing::CallTraceArena;
use std::collections::HashMap;

/// Builds a Chrome trace profile from a call trace arena.
///
/// This converts the trace to folded stack format first (same as --flamechart),
/// then translates to Chrome's trace event format.
pub fn build<'a>(arena: &CallTraceArena, test_name: &str, _contract_name: &str) -> TraceFile<'a> {
    // Build folded stack trace (same as --flamechart uses).
    let folded = folded_stack_trace::build(arena);

    // Convert folded stack trace to Chrome format.
    folded_to_chrome(&folded, test_name)
}

/// An open frame being tracked.
struct OpenFrame {
    name: String,
    start_gas: u64,
}

/// Converts a folded stack trace to Chrome trace format.
///
/// Folded format: "func1;func2;func3 123" where 123 is gas consumed by func3 only.
/// Chrome format: Complete events with timestamp (start) and duration.
fn folded_to_chrome<'a>(folded: &[String], _test_name: &str) -> TraceFile<'a> {
    let mut file = TraceFile::new();

    // Current cumulative gas (used as timestamp).
    let mut cumulative_gas: u64 = 0;

    // Current open stack.
    let mut open_stack: Vec<OpenFrame> = Vec::new();

    // Track which names we've seen to assign categories.
    let mut name_cache: HashMap<String, &'static str> = HashMap::new();

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
            .take_while(|(open, name)| open.name == **name)
            .count();

        // Close frames that are no longer in the stack (in reverse order).
        while open_stack.len() > common_len {
            let frame = open_stack.pop().unwrap();
            let dur = cumulative_gas.saturating_sub(frame.start_gas);
            let cat = category_for_name(&frame.name, &mut name_cache);
            file.add_event(TraceEvent::complete(frame.name, cat, frame.start_gas, dur));
        }

        // Open new frames that are in this stack but not yet open.
        for name in stack.iter().skip(common_len) {
            open_stack.push(OpenFrame { name: name.to_string(), start_gas: cumulative_gas });
        }

        // Advance cumulative gas by this frame's gas consumption.
        cumulative_gas += gas;
    }

    // Close any remaining open frames.
    while let Some(frame) = open_stack.pop() {
        let dur = cumulative_gas.saturating_sub(frame.start_gas);
        let cat = category_for_name(&frame.name, &mut name_cache);
        file.add_event(TraceEvent::complete(frame.name, cat, frame.start_gas, dur));
    }

    file
}

/// Determines category for a frame name (for coloring in Chrome trace viewer).
fn category_for_name<'a>(
    name: &str,
    _cache: &mut HashMap<String, &'static str>,
) -> &'static str {
    if name.starts_with("VM::") || name.starts_with("Vm::") {
        "vm"
    } else if name.contains("console") {
        "console"
    } else if name.starts_with("new ") {
        "create"
    } else {
        "external"
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

        assert!(json.contains("\"traceEvents\""));
    }

    #[test]
    fn test_folded_to_chrome() {
        let folded = vec![
            "top 200".to_string(),
            "top;child_a 100".to_string(),
            "top;child_b 150".to_string(),
        ];

        let file = folded_to_chrome(&folded, "test");
        let json = serde_json::to_string(&file).unwrap();

        // Should have complete events.
        assert!(json.contains("\"ph\":\"X\""));
        assert!(json.contains("\"name\":\"top\""));
        assert!(json.contains("\"name\":\"child_a\""));
        assert!(json.contains("\"name\":\"child_b\""));
    }
}
