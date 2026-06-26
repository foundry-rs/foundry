//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts folded stack trace entries into the speedscope sampled profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.

use super::schema::{Frame, Profile, SampledProfile, SpeedscopeFile, ValueUnit};
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
/// Folded stack entries are aggregate self-gas samples, not an execution timeline, so they are
/// represented as weighted samples instead of synthetic open/close events.
fn entries_to_speedscope<'a>(
    entries: &[TraceEntry],
    test_name: &str,
    contract_name: &str,
) -> SpeedscopeFile<'a> {
    let name = format!("{contract_name}::{test_name}");
    let mut file = SpeedscopeFile::new(name.clone());
    let mut profile = SampledProfile::new(name, ValueUnit::None);

    // Frame cache: name -> frame index.
    let mut frame_cache: HashMap<&str, usize> = HashMap::new();

    for entry in entries {
        let sample = entry
            .names
            .iter()
            .map(|name| {
                *frame_cache
                    .entry(name.as_str())
                    .or_insert_with(|| file.add_frame(Frame::new(Cow::Owned(name.clone()))))
            })
            .collect();
        profile.add_sample(sample, entry.gas);
    }

    file.add_profile(Profile::Sampled(profile));
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
        // gas is aggregate self-time, not enough information to reconstruct event order.
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
    fn test_entries_to_speedscope_json() {
        let entries = vec![
            TraceEntry { names: vec!["top".into()], gas: 200 },
            TraceEntry { names: vec!["top".into(), "child_a".into()], gas: 100 },
            TraceEntry { names: vec!["top".into(), "child_b".into()], gas: 150 },
        ];

        let file = entries_to_speedscope(&entries, "test", "Test");
        let json = serde_json::to_string_pretty(&file).unwrap();

        snapbox::assert_data_eq!(
            json,
            snapbox::str![[r#"
{
  "$schema": "https://www.speedscope.app/file-format-schema.json",
  "shared": {
    "frames": [
      {
        "name": "top"
      },
      {
        "name": "child_a"
      },
      {
        "name": "child_b"
      }
    ]
  },
  "profiles": [
    {
      "type": "sampled",
      "name": "Test::test",
      "unit": "none",
      "startValue": 0,
      "endValue": 450,
      "samples": [
        [
          0
        ],
        [
          0,
          1
        ],
        [
          0,
          2
        ]
      ],
      "weights": [
        200,
        100,
        150
      ]
    }
  ],
  "name": "Test::test",
  "exporter": "foundry"
}
"#]],
        );
    }

    #[test]
    fn test_monotonic_events() {
        // Test that samples preserve weights in entry order.
        let entries = vec![
            TraceEntry { names: vec!["a".into()], gas: 100 },
            TraceEntry { names: vec!["a".into(), "b".into()], gas: 50 },
            TraceEntry { names: vec!["a".into(), "c".into()], gas: 75 },
        ];

        let file = entries_to_speedscope(&entries, "test", "Test");

        if let Profile::Sampled(profile) = &file.profiles[0] {
            assert_eq!(profile.weights, vec![100, 50, 75]);
            assert_eq!(profile.end_value, 225);
        } else {
            panic!("expected sampled profile");
        }
    }
}
