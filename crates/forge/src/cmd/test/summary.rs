use crate::cmd::test::TestOutcome;
use comfy_table::{
    Cell, Color, Row, Table,
    presets::{ASCII_FULL, ASCII_MARKDOWN},
};
use foundry_common::shell;
use foundry_evm::executors::invariant::InvariantMetrics;
use itertools::Itertools;
use serde_json::json;
use std::{collections::HashMap, fmt::Display};

/// Represents a test summary report.
pub struct TestSummaryReport<'a> {
    /// Whether the report should be detailed.
    is_detailed: bool,
    /// The test outcome to report.
    outcome: &'a TestOutcome,
}

impl<'a> TestSummaryReport<'a> {
    pub const fn new(is_detailed: bool, outcome: &'a TestOutcome) -> Self {
        Self { is_detailed, outcome }
    }
}

impl Display for TestSummaryReport<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        if shell::is_json() {
            writeln!(f, "{}", self.format_json_output())
        } else {
            writeln!(f, "\n{}", self.format_table_output())
        }
    }
}

impl TestSummaryReport<'_> {
    fn format_json_output(&self) -> String {
        let results = self
            .outcome
            .results
            .iter()
            .map(|(contract, suite)| {
                let (suite_path, suite_name) = contract.split_once(':').unwrap();
                let mut result = json!({
                    "suite": suite_name,
                    "passed": suite.successes().count(),
                    "failed": suite.failures().count(),
                    "skipped": suite.skips().count(),
                });
                if self.is_detailed {
                    result["file_path"] = suite_path.into();
                    result["duration"] = format!("{:.2?}", suite.duration).into();
                }
                result
            })
            .collect::<Vec<_>>();
        serde_json::to_string_pretty(&json!({ "results": results })).unwrap()
    }

    fn format_table_output(&self) -> Table {
        let mut table = new_table();
        let mut row = Row::from(vec![
            Cell::new("Test Suite"),
            Cell::new("Passed").fg(Color::Green),
            Cell::new("Failed").fg(Color::Red),
            Cell::new("Skipped").fg(Color::Yellow),
        ]);
        if self.is_detailed {
            row.add_cell(Cell::new("File Path").fg(Color::Cyan));
            row.add_cell(Cell::new("Duration").fg(Color::Cyan));
        }
        table.set_header(row);

        for (contract, suite) in &self.outcome.results {
            let (suite_path, suite_name) = contract.split_once(':').unwrap();
            let count_cell = |count: usize, color| {
                let cell = Cell::new(count);
                if count > 0 { cell.fg(color) } else { cell }
            };
            let mut row = Row::from(vec![
                Cell::new(suite_name),
                count_cell(suite.successes().count(), Color::Green),
                count_cell(suite.failures().count(), Color::Red),
                count_cell(suite.skips().count(), Color::Yellow),
            ]);
            if self.is_detailed {
                row.add_cell(Cell::new(suite_path));
                row.add_cell(Cell::new(format!("{:.2?}", suite.duration)));
            }
            table.add_row(row);
        }
        table
    }
}

/// Creates a table styled for the current shell output format.
fn new_table() -> Table {
    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_style(ASCII_MARKDOWN);
    } else {
        table.load_style(ASCII_FULL.with_rounded_corners());
    }
    table
}

/// Helper function to create the invariant metrics table.
///
/// ╭-----------------------+----------------+-------+---------+----------╮
/// | Contract              | Selector       | Calls | Reverts | Discards |
/// +=====================================================================+
/// | AnotherCounterHandler | doWork         | 7451  | 123     | 4941     |
/// |-----------------------+----------------+-------+---------+----------|
/// | AnotherCounterHandler | doWorkThing    | 7279  | 137     | 4849     |
/// |-----------------------+----------------+-------+---------+----------|
/// | CounterHandler        | doAnotherThing | 7302  | 150     | 4794     |
/// |-----------------------+----------------+-------+---------+----------|
/// | CounterHandler        | doSomething    | 7382  | 160     |4794      |
/// ╰-----------------------+----------------+-------+---------+----------╯
pub(crate) fn format_invariant_metrics_table(
    test_metrics: &HashMap<String, InvariantMetrics>,
) -> Table {
    let mut table = new_table();
    table.set_header(vec![
        Cell::new("Contract"),
        Cell::new("Selector"),
        Cell::new("Calls").fg(Color::Green),
        Cell::new("Reverts").fg(Color::Red),
        Cell::new("Discards").fg(Color::Yellow),
    ]);

    let count_cell =
        |count: usize, color| Cell::new(count).fg(if count > 0 { color } else { Color::White });
    for (name, metrics) in test_metrics.iter().sorted_by_key(|(name, _)| *name) {
        let Some((contract, selector)) =
            name.split_once(':').map_or(name.as_str(), |(_, contract)| contract).split_once('.')
        else {
            continue;
        };
        table.add_row(vec![
            Cell::new(contract),
            Cell::new(selector),
            count_cell(metrics.calls, Color::Green),
            count_cell(metrics.reverts, Color::Red),
            count_cell(metrics.discards, Color::Yellow),
        ]);
    }
    table
}

#[cfg(test)]
mod tests {
    use crate::cmd::test::summary::format_invariant_metrics_table;
    use foundry_evm::executors::invariant::InvariantMetrics;
    use std::collections::HashMap;

    #[test]
    fn test_invariant_metrics_table() {
        let mut test_metrics = HashMap::new();
        test_metrics.insert(
            "SystemConfig.setGasLimit".to_string(),
            InvariantMetrics { calls: 10, reverts: 1, discards: 1 },
        );
        test_metrics.insert(
            "src/universal/Proxy.sol:Proxy.changeAdmin".to_string(),
            InvariantMetrics { calls: 20, reverts: 2, discards: 2 },
        );
        let table = format_invariant_metrics_table(&test_metrics);
        assert_eq!(table.row_count(), 2);

        let mut first_row_content = table.row(0).unwrap().cell_iter();
        assert_eq!(first_row_content.next().unwrap().content(), "SystemConfig");
        assert_eq!(first_row_content.next().unwrap().content(), "setGasLimit");
        assert_eq!(first_row_content.next().unwrap().content(), "10");
        assert_eq!(first_row_content.next().unwrap().content(), "1");
        assert_eq!(first_row_content.next().unwrap().content(), "1");

        let mut second_row_content = table.row(1).unwrap().cell_iter();
        assert_eq!(second_row_content.next().unwrap().content(), "Proxy");
        assert_eq!(second_row_content.next().unwrap().content(), "changeAdmin");
        assert_eq!(second_row_content.next().unwrap().content(), "20");
        assert_eq!(second_row_content.next().unwrap().content(), "2");
        assert_eq!(second_row_content.next().unwrap().content(), "2");
    }
}
