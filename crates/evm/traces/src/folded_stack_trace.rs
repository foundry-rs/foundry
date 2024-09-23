use alloy_primitives::hex::ToHexExt;
use revm_inspectors::tracing::{
    types::{CallTraceNode, CallTraceStep, DecodedTraceStep, TraceMemberOrder},
    CallTraceArena,
};

/// Builds a folded stack trace from a call trace arena.
pub fn build(arena: &CallTraceArena) -> Vec<String> {
    let mut fst = EvmFoldedStackTraceBuilder::default();
    fst.process_call_node(arena.nodes(), 0);
    fst.build()
}

/// Wrapper for building a folded stack trace using EVM call trace node.
#[derive(Default)]
pub struct EvmFoldedStackTraceBuilder {
    /// Raw folded stack trace builder.
    fst: FoldedStackTraceBuilder,
}

impl EvmFoldedStackTraceBuilder {
    /// Returns the folded stack trace.
    pub fn build(self) -> Vec<String> {
        self.fst.build()
    }

    /// Creates an entry for a EVM CALL in the folded stack trace. This method recursively processes
    /// all the children nodes of the call node and at the end it exits.
    pub fn process_call_node(&mut self, nodes: &[CallTraceNode], idx: usize) {
        let node = &nodes[idx];

        let func_name = if node.trace.kind.is_any_create() {
            let contract_name = node.trace.decoded.label.as_deref().unwrap_or("Contract");
            format!("new {contract_name}")
        } else {
            let selector = node
                .selector()
                .map(|selector| selector.encode_hex_with_prefix())
                .unwrap_or_else(|| "fallback".to_string());
            let signature =
                node.trace.decoded.call_data.as_ref().map(|dc| &dc.signature).unwrap_or(&selector);

            if let Some(label) = &node.trace.decoded.label {
                format!("{label}.{signature}")
            } else {
                signature.clone()
            }
        };

        self.fst.enter(func_name, node.trace.gas_used as i64);

        // Track internal function step exits to do in this call context.
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

        // Exit pending internal function calls if any.
        for _ in 0..step_exits.len() {
            self.fst.exit();
        }

        // Exit from this call context in the folded stack trace.
        self.fst.exit();
    }

    /// Creates an entry for an internal function call in the folded stack trace. This method only
    /// enters the function in the folded stack trace, we cannot exit since we need to exit at a
    /// future step. Hence, we keep track of the step end index in the `step_exits`.
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
                    self.fst.enter(decoded_internal_call.func_name.clone(), gas_used as i64);
                    step_exits.push(*step_end_idx);
                }
                DecodedTraceStep::Line(_) => {}
            }
        }
    }

    /// Exits all the previous internal calls that should end before starting step_idx.
    fn exit_previous_steps(&mut self, step_exits: &mut Vec<usize>, step_idx: usize) {
        let initial_length = step_exits.len();
        step_exits.retain(|&number| number > step_idx);

        let num_exits = initial_length - step_exits.len();
        for _ in 0..num_exits {
            self.fst.exit();
        }
    }
}

/// Helps to translate a function enter-exit flow into a folded stack trace.
///
/// Example:
/// ```solidity
/// function top() { child_a(); child_b() } // consumes 500 gas
/// function child_a() {} // consumes 100 gas
/// function child_b() {} // consumes 200 gas
/// ```
///
/// For execution of the `top` function looks like:
/// 1. enter `top`
/// 2. enter `child_a`
/// 3. exit `child_a`
/// 4. enter `child_b`
/// 5. exit `child_b`
/// 6. exit `top`
///
/// The translated folded stack trace lines look like:
/// 1. top
/// 2. top;child_a
/// 3. top;child_b
///
/// Including the gas consumed by the function by itself.
/// 1. top 200 // 500 - 100 - 200
/// 2. top;child_a 100
/// 3. top;child_b 200
#[derive(Debug, Default)]
pub struct FoldedStackTraceBuilder {
    /// Trace entries.
    traces: Vec<TraceEntry>,
    /// Number of exits to be done before entering a new function.
    exits: usize,
}

#[derive(Debug, Default)]
struct TraceEntry {
    /// Names of all functions in the call stack of this trace.
    names: Vec<String>,
    /// Gas consumed by this function, allowed to be negative due to refunds.
    gas: i64,
}

impl FoldedStackTraceBuilder {
    /// Enter execution of a function call that consumes `gas`.
    pub fn enter(&mut self, label: String, gas: i64) {
        let mut names = self.traces.last().map(|entry| entry.names.clone()).unwrap_or_default();

        while self.exits > 0 {
            names.pop();
            self.exits -= 1;
        }

        names.push(label);
        self.traces.push(TraceEntry { names, gas });
    }

    /// Exit execution of a function call.
    pub fn exit(&mut self) {
        self.exits += 1;
    }

    /// Returns folded stack trace.
    pub fn build(mut self) -> Vec<String> {
        self.subtract_children();
        self.build_without_subtraction()
    }

    /// Internal method to build the folded stack trace without subtracting gas consumed by
    /// the children function calls.
    fn build_without_subtraction(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for TraceEntry { names, gas } in self.traces.iter() {
            lines.push(format!("{} {}", names.join(";"), gas));
        }
        lines
    }

    /// Subtracts gas consumed by the children function calls from the parent function calls.
    fn subtract_children(&mut self) {
        // Iterate over each trace to find the children and subtract their values from the parents.
        for i in 0..self.traces.len() {
            let (left, right) = self.traces.split_at_mut(i);
            let TraceEntry { names, gas } = &right[0];
            if names.len() > 1 {
                let parent_trace_to_match = &names[..names.len() - 1];
                for parent in left.iter_mut().rev() {
                    if parent.names == parent_trace_to_match {
                        parent.gas -= gas;
                        break;
                    }
                }
            }
        }
    }
}

mod tests {
    #[test]
    fn test_fst_1() {
        let mut trace = super::FoldedStackTraceBuilder::default();
        trace.enter("top".to_string(), 500);
        trace.enter("child_a".to_string(), 100);
        trace.exit();
        trace.enter("child_b".to_string(), 200);

        assert_eq!(
            trace.build_without_subtraction(),
            vec![
                "top 500", //
                "top;child_a 100",
                "top;child_b 200",
            ]
        );
        assert_eq!(
            trace.build(),
            vec![
                "top 200", // 500 - 100 - 200
                "top;child_a 100",
                "top;child_b 200",
            ]
        );
    }

    #[test]
    fn test_fst_2() {
        let mut trace = super::FoldedStackTraceBuilder::default();
        trace.enter("top".to_string(), 500);
        trace.enter("child_a".to_string(), 300);
        trace.enter("child_b".to_string(), 100);
        trace.exit();
        trace.exit();
        trace.enter("child_c".to_string(), 100);

        assert_eq!(
            trace.build_without_subtraction(),
            vec![
                "top 500", //
                "top;child_a 300",
                "top;child_a;child_b 100",
                "top;child_c 100",
            ]
        );

        assert_eq!(
            trace.build(),
            vec![
                "top 100",         // 500 - 300 - 100
                "top;child_a 200", // 300 - 100
                "top;child_a;child_b 100",
                "top;child_c 100",
            ]
        );
    }

    #[test]
    fn test_fst_3() {
        let mut trace = super::FoldedStackTraceBuilder::default();
        trace.enter("top".to_string(), 1700);
        trace.enter("child_a".to_string(), 500);
        trace.exit();
        trace.enter("child_b".to_string(), 500);
        trace.enter("child_c".to_string(), 500);
        trace.exit();
        trace.exit();
        trace.exit();
        trace.enter("top2".to_string(), 1700);

        assert_eq!(
            trace.build_without_subtraction(),
            vec![
                "top 1700", //
                "top;child_a 500",
                "top;child_b 500",
                "top;child_b;child_c 500",
                "top2 1700",
            ]
        );

        assert_eq!(
            trace.build(),
            vec![
                "top 700", //
                "top;child_a 500",
                "top;child_b 0",
                "top;child_b;child_c 500",
                "top2 1700",
            ]
        );
    }
}
