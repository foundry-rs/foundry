use crate::cmd::test::TestOutcome;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN, Attribute, Cell, CellAlignment, Color,
    Row, Table,
};
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
    pub outcome: TestOutcome,
}

impl TestSummaryReport {
    pub fn new(is_detailed: bool, outcome: TestOutcome) -> Self {
        Self { report_kind: report_kind(), is_detailed, outcome }
    }
}

impl Display for TestSummaryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.report_kind {
            ReportKind::Markdown => {
                writeln!(f, "\nTest Summary:\n")?;
                writeln!(f, "{}", &self.format_table_output(&self.is_detailed, &self.outcome))?;
            }
            ReportKind::JSON => {
                writeln!(f, "{}", &self.format_json_output(&self.is_detailed, &self.outcome))?;
            }
        }

        Ok(())
    }
}

impl TestSummaryReport {
    // Helper function to format the JSON output
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

    // Helper function to format the Markdown table output
    fn format_table_output(&self, is_detailed: &bool, outcome: &TestOutcome) -> Table {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);
        let mut row = Row::from(vec![
            Cell::new("Test Suite")
                .set_alignment(CellAlignment::Center)
                .add_attribute(Attribute::Bold),
            Cell::new("Passed")
                .set_alignment(CellAlignment::Center)
                .add_attribute(Attribute::Bold)
                .fg(Color::Green),
            Cell::new("Failed")
                .set_alignment(CellAlignment::Center)
                .add_attribute(Attribute::Bold)
                .fg(Color::Red),
            Cell::new("Skipped")
                .set_alignment(CellAlignment::Center)
                .add_attribute(Attribute::Bold)
                .fg(Color::Yellow),
        ]);
        if *is_detailed {
            row.add_cell(
                Cell::new("File Path")
                    .set_alignment(CellAlignment::Center)
                    .add_attribute(Attribute::Bold),
            );
            row.add_cell(
                Cell::new("Duration")
                    .set_alignment(CellAlignment::Center)
                    .add_attribute(Attribute::Bold),
            );
        }
        table.set_header(row);

        // Traverse the test_results vector and build the table
        for (contract, suite) in &outcome.results {
            let mut row = Row::new();
            let (suite_path, suite_name) = contract.split_once(':').unwrap();

            let passed = suite.successes().count();
            let mut passed_cell = Cell::new(passed).set_alignment(CellAlignment::Center);

            let failed = suite.failures().count();
            let mut failed_cell = Cell::new(failed).set_alignment(CellAlignment::Center);

            let skipped = suite.skips().count();
            let mut skipped_cell = Cell::new(skipped).set_alignment(CellAlignment::Center);

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

/// Helper to create and render invariant metrics summary table:
/// | Contract              | Selector       | Calls | Reverts | Discards |
/// |-----------------------|----------------|-------|---------|----------|
/// | AnotherCounterHandler | doWork         |  7451 |   123   |   4941   |
/// | AnotherCounterHandler | doWorkThing    |  7279 |   137   |   4849   |
/// | CounterHandler        | doAnotherThing |  7302 |   150   |   4794   |
/// | CounterHandler        | doSomething    |  7382 |   160   |   4830   |
pub(crate) fn print_invariant_metrics(test_metrics: &HashMap<String, InvariantMetrics>) {
    if !test_metrics.is_empty() {
        let mut table = Table::new();
        table.load_preset(ASCII_MARKDOWN);
        table.set_header(["Contract", "Selector", "Calls", "Reverts", "Discards"]);

        for name in test_metrics.keys().sorted() {
            if let Some((contract, selector)) =
                name.split_once(':').and_then(|(_, contract)| contract.split_once('.'))
            {
                let mut row = Row::new();
                row.add_cell(Cell::new(contract).set_alignment(CellAlignment::Left));
                row.add_cell(Cell::new(selector).set_alignment(CellAlignment::Left));
                if let Some(metrics) = test_metrics.get(name) {
                    row.add_cell(Cell::new(metrics.calls).set_alignment(CellAlignment::Center));
                    row.add_cell(Cell::new(metrics.reverts).set_alignment(CellAlignment::Center));
                    row.add_cell(Cell::new(metrics.discards).set_alignment(CellAlignment::Center));
                }

                table.add_row(row);
            }
        }

        let _ = sh_println!("{table}\n");
    }
}
