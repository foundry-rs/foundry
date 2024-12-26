use crate::cmd::test::TestOutcome;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, Cell, Color, Row, Table};
use foundry_common::reports::{report_kind, ReportKind};
use foundry_evm::executors::invariant::InvariantMetrics;
use itertools::Itertools;
use serde_json::json;
use std::{collections::HashMap, fmt::Display};

/// Represents a test summary report.
pub struct TestSummaryReport {
    /// The kind of report to generate.
    report_kind: ReportKind,
    /// Whether the report should be detailed.
    is_detailed: bool,
    /// The test outcome to report.
    outcome: TestOutcome,
}

impl TestSummaryReport {
    pub fn new(is_detailed: bool, outcome: TestOutcome) -> Self {
        Self { report_kind: report_kind(), is_detailed, outcome }
    }
}

impl Display for TestSummaryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.report_kind {
            ReportKind::Text => {
                writeln!(f, "\n{}", &self.format_table_output(&self.is_detailed, &self.outcome))?;
            }
            ReportKind::JSON => {
                writeln!(f, "{}", &self.format_json_output(&self.is_detailed, &self.outcome))?;
            }
        }

        Ok(())
    }
}

impl TestSummaryReport {
    // Helper function to format the JSON output.
    fn format_json_output(&self, is_detailed: &bool, outcome: &TestOutcome) -> String {
        let output = json!({
            "results": outcome.results.iter().map(|(contract, suite)| {
                let (suite_path, suite_name) = contract.split_once(':').unwrap();
                let passed = suite.successes().count();
                let failed = suite.failures().count();
                let skipped = suite.skips().count();
                let mut result = json!({
                    "suite": suite_name,
                    "passed": passed,
                    "failed": failed,
                    "skipped": skipped,
                });

                if *is_detailed {
                    result["file_path"] = serde_json::Value::String(suite_path.to_string());
                    result["duration"] = serde_json::Value::String(format!("{:.2?}", suite.duration));
                }

                result
            }).collect::<Vec<serde_json::Value>>(),
        });

        serde_json::to_string_pretty(&output).unwrap()
    }

    fn format_table_output(&self, is_detailed: &bool, outcome: &TestOutcome) -> Table {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

        let mut row = Row::from(vec![
            Cell::new("Test Suite"),
            Cell::new("Passed").fg(Color::Green),
            Cell::new("Failed").fg(Color::Red),
            Cell::new("Skipped").fg(Color::Yellow),
        ]);
        if *is_detailed {
            row.add_cell(Cell::new("File Path").fg(Color::Cyan));
            row.add_cell(Cell::new("Duration").fg(Color::Cyan));
        }
        table.set_header(row);

        // Traverse the test_results vector and build the table
        for (contract, suite) in &outcome.results {
            let mut row = Row::new();
            let (suite_path, suite_name) = contract.split_once(':').unwrap();

            let passed = suite.successes().count();
            let mut passed_cell = Cell::new(passed);

            let failed = suite.failures().count();
            let mut failed_cell = Cell::new(failed);

            let skipped = suite.skips().count();
            let mut skipped_cell = Cell::new(skipped);

            row.add_cell(Cell::new(suite_name));

            if passed > 0 {
                passed_cell = passed_cell.fg(Color::Green);
            }
            row.add_cell(passed_cell);

            if failed > 0 {
                failed_cell = failed_cell.fg(Color::Red);
            }
            row.add_cell(failed_cell);

            if skipped > 0 {
                skipped_cell = skipped_cell.fg(Color::Yellow);
            }
            row.add_cell(skipped_cell);

            if self.is_detailed {
                row.add_cell(Cell::new(suite_path));
                row.add_cell(Cell::new(format!("{:.2?}", suite.duration).to_string()));
            }

            table.add_row(row);
        }

        table
    }
}

/// Helper function to print the invariant metrics.
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
pub(crate) fn print_invariant_metrics(test_metrics: &HashMap<String, InvariantMetrics>) {
    if !test_metrics.is_empty() {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![
            Cell::new("Contract"),
            Cell::new("Selector"),
            Cell::new("Calls").fg(Color::Green),
            Cell::new("Reverts").fg(Color::Red),
            Cell::new("Discards").fg(Color::Yellow),
        ]);

        for name in test_metrics.keys().sorted() {
            if let Some((contract, selector)) =
                name.split_once(':').and_then(|(_, contract)| contract.split_once('.'))
            {
                let mut row = Row::new();
                row.add_cell(Cell::new(contract));
                row.add_cell(Cell::new(selector));

                if let Some(metrics) = test_metrics.get(name) {
                    let calls_cell = Cell::new(metrics.calls).fg(if metrics.calls > 0 {
                        Color::Green
                    } else {
                        Color::White
                    });

                    let reverts_cell = Cell::new(metrics.reverts).fg(if metrics.reverts > 0 {
                        Color::Red
                    } else {
                        Color::White
                    });

                    let discards_cell = Cell::new(metrics.discards).fg(if metrics.discards > 0 {
                        Color::Yellow
                    } else {
                        Color::White
                    });

                    row.add_cell(calls_cell);
                    row.add_cell(reverts_cell);
                    row.add_cell(discards_cell);
                }

                table.add_row(row);
            }
        }

        let _ = sh_println!("\n{table}\n");
    }
}
