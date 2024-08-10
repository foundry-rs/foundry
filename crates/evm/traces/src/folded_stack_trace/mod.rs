use alloy_primitives::hex::ToHexExt;
use builder::FoldedStackTraceBuilder;
use revm_inspectors::tracing::{
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
    CallTraceArena,
};

mod builder;

/// Builds a folded stack trace from the given `arena`.
pub fn build(arena: &CallTraceArena) -> Vec<String> {
    let mut fst = FoldedStackTraceBuilder::default();

    fst.process_call_node(arena.nodes(), 0);

    fst.build()
}

impl FoldedStackTraceBuilder {
    /// Creates an entry for a EVM CALL in the folded stack trace.
    fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize) {
        let node = &nodes[idx];

        let label = if node.trace.kind.is_any_create() {
            let default_contract_name = "Contract".to_string();
            let contract_name = node.trace.decoded.label.as_ref().unwrap_or(&default_contract_name);
            format!("new {contract_name}")
        } else {
            let selector = node
                .selector()
                .map(|selector| selector.encode_hex_with_prefix())
                .unwrap_or("fallback".to_string());
            let signature =
                node.trace.decoded.call_data.as_ref().map(|dc| &dc.signature).unwrap_or(&selector);

            if let Some(label) = &node.trace.decoded.label {
                format!("{label}.{signature}")
            } else {
                signature.clone()
            }
        };

        self.enter(label, node.trace.gas_used as i64);

        // Track step exits to do in this call context
        let mut step_exits = vec![];

        // Process children nodes.
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Call(child_idx) => {
                    let child_node_idx = node.children[*child_idx];
                    self.process_call_node(nodes, child_node_idx);
                }
                TraceMemberOrder::Step(step_idx) => {
                    self.exit_previous_steps(&mut step_exits, *step_idx);
                    self.process_step(&node.trace.steps, *step_idx, &mut step_exits)
                }
                TraceMemberOrder::Log(_) => {}
            }
        }

        // Exit from this call context in the folded stack trace.
        self.exit();
    }

    /// Creates an entry for an internal function call in the folded stack trace.
    fn process_step(
        &mut self,
        steps: &[CallTraceStep],
        step_idx: usize,
        step_exits: &mut Vec<usize>,
    ) {
        let step = &steps[step_idx];
        if let Some(decoded_step) = &step.decoded {
            match decoded_step {
                DecodedTraceStep::InternalCall(decoded_internal_call, step_end_idx) => {
                    let gas_used = steps[*step_end_idx].gas_used.saturating_sub(step.gas_used);
                    self.enter(decoded_internal_call.func_name.clone(), gas_used as i64);
                    step_exits.push(*step_end_idx);
                }
                DecodedTraceStep::Line(_) => {}
            }
        }
    }

    /// Exits all the previous internal calls that should end before starting step_idx.
    fn exit_previous_steps(&mut self, step_exits: &mut Vec<usize>, step_idx: usize) {
        let initial_length = step_exits.len();
        step_exits.retain(|&number| number >= step_idx);

        let num_exits = initial_length - step_exits.len();
        for _ in 0..num_exits {
            self.exit();
        }
    }
}
