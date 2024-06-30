#[derive(Debug, Clone, Default)]
pub struct FoldedStackTrace {
    traces: Vec<(Vec<String>, i64)>,
    exits: Option<u64>,
}

impl FoldedStackTrace {
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

    pub fn exit(&mut self) {
        self.exits = self.exits.map(|exits| exits + 1).or(Some(1));
    }

    pub fn fold(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (trace, gas) in self.traces.iter() {
            lines.push(format!("{} {}", trace.join(";"), gas));
        }
        lines
    }

    // TODO complete this impl
    pub fn adjust_gas(&mut self) {
        self.traces.sort();

        let mut depth = 0;
        for (trace, _) in self.traces.iter() {
            depth = depth.max(trace.len());
        }

        let mut idx = 0;
        let mut matcher: Option<Vec<String>> = None;
        let mut sum = 0;
        loop {
            if self.traces[idx].0.len() == depth {
                if let Some(matcher) = matcher.as_ref() {
                    if matcher == &self.traces[idx].0 {
                        sum += self.traces[idx].1;
                    }
                } else {
                    let mut value = self.traces[idx].0.clone();
                    value.pop();
                    matcher = Some(value);
                }
            }
        }
    }
}

mod tests {
    #[test]
    fn test_insert_1() {
        let mut trace = super::FoldedStackTrace::default();
        trace.enter("top".to_string(), 1600);
        trace.enter("child_a".to_string(), 500);
        trace.exit();
        trace.enter("child_b".to_string(), 500);
        trace.enter("child_c".to_string(), 500);
        assert_eq!(
            trace.fold(),
            vec![
                "top 1600", // TODO this should be 100 i.e. 1600 - 500 - 500 - 500
                "top;child_a 500",
                "top;child_b 500",
                "top;child_b:child_c; 500"
            ]
        );
    }
}
