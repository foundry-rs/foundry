/// Helps to build a folded stack trace.
#[derive(Debug, Clone, Default)]
pub(crate) struct FoldedStackTraceBuilder {
    traces: Vec<(Vec<String>, i64)>,
    exits: Option<u64>,
}

impl FoldedStackTraceBuilder {
    /// Enter execution of a function call that consumes `gas`.
    pub fn enter(&mut self, label: String, gas: i64) {
        let mut trace_entry = self.traces.last().map(|entry| entry.0.clone()).unwrap_or_default();

        let mut exits = self.exits.unwrap_or_default();
        while exits > 0 {
            trace_entry.pop();
            exits -= 1;
        }
        self.exits = None;

        trace_entry.push(label);
        self.traces.push((trace_entry, gas));
    }

    /// Exit execution of a function call.
    pub fn exit(&mut self) {
        self.exits = self.exits.map(|exits| exits + 1).or(Some(1));
    }

    /// Returns folded stack trace.
    pub fn build(mut self) -> Vec<String> {
        self.subtract_children();
        self.build_without_subtraction()
    }

    /// Internal method to build the folded stack trace without subtracting gas consumed by
    /// the children function calls.
    pub fn build_without_subtraction(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for (trace, gas) in self.traces.iter() {
            lines.push(format!("{} {}", trace.join(";"), gas));
        }
        lines
    }

    /// Subtracts gas consumed by the children function calls from the parent function calls.
    fn subtract_children(&mut self) {
        // Iterate over each trace to find the children and subtract their values from the parents
        for i in 0..self.traces.len() {
            let (left, right) = self.traces.split_at_mut(i);
            let (trace, gas) = &right[0];
            if trace.len() > 1 {
                let parent_trace_to_match = &trace[..trace.len() - 1];
                for parent in left {
                    if parent.0 == parent_trace_to_match {
                        parent.1 -= gas;
                        break;
                    }
                }
            }
        }
    }
}

mod tests {
    #[test]
    fn test_insert_1() {
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
    fn test_insert_2() {
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
    fn test_insert_3() {
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
