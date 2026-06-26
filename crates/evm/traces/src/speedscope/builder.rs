//! Speedscope profile generation for EVM execution traces.
//!
//! This module converts EVM execution traces into the speedscope evented profile format.
//! Gas consumption is used as the value unit, so flame graph widths represent gas usage.

use super::schema::{EventedProfile, Frame, Profile, SpeedscopeFile, ValueUnit};
use alloy_primitives::hex::ToHexExt;
use revm_inspectors::tracing::{
    CallTraceArena,
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
};
use std::{borrow::Cow, collections::HashMap};

/// Builds a speedscope profile from a call trace arena.
///
/// Walks the trace arena directly so the Time Order view preserves execution order.
pub fn build<'a>(
    arena: &CallTraceArena,
    test_name: &str,
    contract_name: &str,
    isolate: bool,
) -> SpeedscopeFile<'a> {
    let name = format!("{contract_name}::{test_name}");
    let mut builder = SpeedscopeBuilder::new(name);

    if !arena.nodes().is_empty() {
        builder.process_call_node(arena.nodes(), 0, isolate);
    }

    builder.build()
}

struct SpeedscopeBuilder<'a> {
    file: SpeedscopeFile<'a>,
    profile: EventedProfile<'a>,
    frame_cache: HashMap<String, usize>,
    cumulative_gas: u64,
}

impl<'a> SpeedscopeBuilder<'a> {
    fn new(name: String) -> Self {
        Self {
            file: SpeedscopeFile::new(name.clone()),
            profile: EventedProfile::new(name, ValueUnit::None),
            frame_cache: HashMap::new(),
            cumulative_gas: 0,
        }
    }

    fn build(mut self) -> SpeedscopeFile<'a> {
        self.profile.set_end_value(self.cumulative_gas);
        self.file.add_profile(Profile::Evented(self.profile));
        self.file
    }

    fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize, isolate: bool) {
        let node = &nodes[idx];
        let frame_idx = self.frame_idx(call_frame_name(node));
        let start_gas = self.cumulative_gas;
        let gas_used = call_gas_used(node, isolate);

        self.profile.open_frame(frame_idx, self.cumulative_gas);

        let mut step_exits = Vec::new();
        for (order_idx, order) in node.ordering.iter().enumerate() {
            match order {
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);
                    let is_call_step =
                        matches!(node.ordering.get(order_idx + 1), Some(TraceMemberOrder::Call(_)));
                    self.process_step(&node.trace.steps, *step_idx, is_call_step, &mut step_exits);
                }
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx, isolate);
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        while let Some(step_exit) = step_exits.pop() {
            self.profile.close_frame(step_exit.frame_idx, self.cumulative_gas);
        }

        let consumed = self.cumulative_gas.saturating_sub(start_gas);
        self.cumulative_gas = self.cumulative_gas.saturating_add(gas_used.saturating_sub(consumed));
        self.profile.close_frame(frame_idx, self.cumulative_gas);
    }

    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        skip_gas: bool,
        step_exits: &mut Vec<StepExit>,
    ) {
        let Some(step) = steps.get(step_idx) else { return };

        if let Some(DecodedTraceStep::InternalCall(decoded, step_end_idx)) = step.decoded.as_deref()
        {
            let frame_idx = self.frame_idx(decoded.func_name.clone());
            self.profile.open_frame(frame_idx, self.cumulative_gas);
            step_exits.push(StepExit { step_idx: *step_end_idx, frame_idx });
        }

        if !skip_gas {
            self.cumulative_gas = self.cumulative_gas.saturating_add(step.gas_cost);
        }
    }

    fn exit_previous_steps(&mut self, step_exits: &mut Vec<StepExit>, step_idx: usize) {
        while step_exits.last().is_some_and(|exit| exit.step_idx <= step_idx) {
            let step_exit = step_exits.pop().unwrap();
            self.profile.close_frame(step_exit.frame_idx, self.cumulative_gas);
        }
    }

    fn frame_idx(&mut self, name: String) -> usize {
        if let Some(idx) = self.frame_cache.get(name.as_str()) {
            return *idx;
        }

        let idx = self.file.add_frame(Frame::new(Cow::Owned(name.clone())));
        self.frame_cache.insert(name, idx);
        idx
    }
}

struct StepExit {
    step_idx: usize,
    frame_idx: usize,
}

fn call_frame_name(node: &CallTraceNode) -> String {
    if node.trace.kind.is_any_create() {
        let contract_name =
            node.trace.decoded.as_ref().and_then(|dc| dc.label.as_deref()).unwrap_or("Contract");
        return format!("new {contract_name}");
    }

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

const fn call_gas_used(node: &CallTraceNode, isolate: bool) -> u64 {
    let mut gas_used = node.trace.gas_used;
    let max_refund_adjust_depth = if isolate { 1 } else { 0 };
    if node.trace.depth <= max_refund_adjust_depth {
        gas_used = gas_used.saturating_add(node.trace.gas_refund_counter);
    }
    gas_used
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallKind, CallTrace, DecodedCallData, DecodedCallTrace};
    use revm::{bytecode::opcode::OpCode, interpreter::InstructionResult};

    fn trace_step(gas_cost: u64) -> CallTraceStep {
        CallTraceStep {
            pc: 0,
            op: OpCode::STOP,
            stack: None,
            push_stack: None,
            memory: None,
            returndata: Default::default(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost,
            storage_change: None,
            status: Some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
    }

    fn decoded_call(label: &str, signature: &str) -> Option<Box<DecodedCallTrace>> {
        Some(Box::new(DecodedCallTrace {
            label: Some(label.to_string()),
            call_data: Some(DecodedCallData { signature: signature.to_string(), args: vec![] }),
            return_data: None,
        }))
    }

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
    fn test_build_preserves_parent_work_ordering() {
        let mut arena = CallTraceArena::default();
        {
            let root = &mut arena.nodes_mut()[0];
            root.trace = CallTrace {
                kind: CallKind::Call,
                gas_used: 400,
                steps: vec![
                    trace_step(20),
                    trace_step(1_000_000),
                    trace_step(30),
                    trace_step(1_000_000),
                    trace_step(50),
                ],
                decoded: decoded_call("Parent", "run()"),
                ..Default::default()
            };
            root.children = vec![1, 2];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Step(1),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(2),
                TraceMemberOrder::Step(3),
                TraceMemberOrder::Call(1),
                TraceMemberOrder::Step(4),
            ];
        }

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace {
                depth: 1,
                kind: CallKind::Call,
                gas_used: 100,
                decoded: decoded_call("Child", "first()"),
                ..Default::default()
            },
            ..Default::default()
        });
        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 2,
            trace: CallTrace {
                depth: 1,
                kind: CallKind::Call,
                gas_used: 200,
                decoded: decoded_call("Child", "second()"),
                ..Default::default()
            },
            ..Default::default()
        });

        let file = build(&arena, "test", "Test", false);
        let json = serde_json::to_string_pretty(&file).unwrap();

        snapbox::assert_data_eq!(
            json,
            snapbox::str![[r#"
{
  "$schema": "https://www.speedscope.app/file-format-schema.json",
  "shared": {
    "frames": [
      {
        "name": "Parent.run()"
      },
      {
        "name": "Child.first()"
      },
      {
        "name": "Child.second()"
      }
    ]
  },
  "profiles": [
    {
      "type": "evented",
      "name": "Test::test",
      "unit": "none",
      "startValue": 0,
      "endValue": 400,
      "events": [
        {
          "type": "O",
          "frame": 0,
          "at": 0
        },
        {
          "type": "O",
          "frame": 1,
          "at": 20
        },
        {
          "type": "C",
          "frame": 1,
          "at": 120
        },
        {
          "type": "O",
          "frame": 2,
          "at": 150
        },
        {
          "type": "C",
          "frame": 2,
          "at": 350
        },
        {
          "type": "C",
          "frame": 0,
          "at": 400
        }
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
        let mut arena = CallTraceArena::default();
        {
            let root = &mut arena.nodes_mut()[0];
            root.trace = CallTrace {
                gas_used: 225,
                steps: vec![trace_step(100), trace_step(75)],
                decoded: decoded_call("A", "a()"),
                ..Default::default()
            };
            root.children = vec![1];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
        }
        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace {
                depth: 1,
                gas_used: 50,
                decoded: decoded_call("B", "b()"),
                ..Default::default()
            },
            ..Default::default()
        });

        let file = build(&arena, "test", "Test", false);

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
