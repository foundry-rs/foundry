use crate::cmd::test::TestOutcome;
use comfy_table::{
    Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN,
};
use foundry_common::shell;
use foundry_evm::executors::invariant::InvariantMetrics;
use itertools::Itertools;
use serde_json::json;
use std::{collections::HashMap, fmt::Display};

/// Represents a test summary report.
pub struct TestSummaryReport {
    /// Whether the report should be detailed.
    is_detailed: bool,
    /// The test outcome to report.
    outcome: TestOutcome,
}

impl TestSummaryReport {
    pub const fn new(is_detailed: bool, outcome: TestOutcome) -> Self {
        Self { is_detailed, outcome }
    }
}

impl Display for TestSummaryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        if shell::is_json() {
            writeln!(f, "{}", self.format_json_output(&self.is_detailed, &self.outcome))?;
        } else {
            writeln!(f, "\n{}", self.format_table_output(&self.is_detailed, &self.outcome))?;
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
        if shell::is_markdown() {
            table.load_preset(ASCII_MARKDOWN);
        } else {
            table.apply_modifier(UTF8_ROUND_CORNERS);
        }

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
                row.add_cell(Cell::new(format!("{:.2?}", suite.duration)));
            }

            table.add_row(row);
        }

        table
    }
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
    block_gas_limit: Option<u64>,
    show_max_gas: bool,
) -> Table {
    let mut table = Table::new();
    if shell::is_markdown() {
        table.load_preset(ASCII_MARKDOWN);
    } else {
        table.apply_modifier(UTF8_ROUND_CORNERS);
    }

    let show_max_gas = show_max_gas || test_metrics.values().any(|m| m.max_gas > 0);

    let mut header = vec![
        Cell::new("Contract"),
        Cell::new("Selector"),
        Cell::new("Calls").fg(Color::Green),
        Cell::new("Reverts").fg(Color::Red),
        Cell::new("Discards").fg(Color::Yellow),
    ];
    if show_max_gas {
        header.push(Cell::new("Max Gas").fg(Color::Cyan));
    }
    table.set_header(header);

    let render_max_gas =
        |max_gas: u64| -> Cell { Cell::new(max_gas).fg(max_gas_color(max_gas, block_gas_limit)) };

    for name in test_metrics.keys().sorted() {
        if let Some((contract, selector)) =
            name.split_once(':').map_or(name.as_str(), |(_, contract)| contract).split_once('.')
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

                if show_max_gas {
                    row.add_cell(render_max_gas(metrics.max_gas));
                }
            }

            table.add_row(row);
        }
    }
    table
}

/// Warn (yellow) when a single selector burns >= this % of the block gas limit.
const GAS_WARN_PCT: u64 = 25;
/// Danger (red) when it crosses this % — DoS risk territory.
const GAS_DANGER_PCT: u64 = 50;

/// Color for a `max_gas` cell by share of `block_gas_limit`.
fn max_gas_color(max_gas: u64, block_gas_limit: Option<u64>) -> Color {
    if max_gas == 0 {
        return Color::White;
    }
    let Some(cap) = block_gas_limit.filter(|c| *c > 0) else {
        return Color::Cyan;
    };
    match max_gas.saturating_mul(100) / cap {
        p if p >= GAS_DANGER_PCT => Color::Red,
        p if p >= GAS_WARN_PCT => Color::Yellow,
        _ => Color::Cyan,
    }
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
            InvariantMetrics { calls: 10, reverts: 1, discards: 1, ..Default::default() },
        );
        test_metrics.insert(
            "src/universal/Proxy.sol:Proxy.changeAdmin".to_string(),
            InvariantMetrics { calls: 20, reverts: 2, discards: 2, ..Default::default() },
        );
        let table = format_invariant_metrics_table(&test_metrics, None, false);
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

    #[test]
    fn max_gas_cell_color_by_share_of_block_gas() {
        use crate::cmd::test::summary::max_gas_color;
        use comfy_table::Color;

        let cap = Some(10_000_000_u64);
        assert_eq!(max_gas_color(6_000_000, cap), Color::Red); // 60% → danger
        assert_eq!(max_gas_color(3_000_000, cap), Color::Yellow); // 30% → warn
        assert_eq!(max_gas_color(500_000, cap), Color::Cyan); // 5%  → safe
        assert_eq!(max_gas_color(0, cap), Color::White);
        // Without a cap, gas is always cyan (no warning available).
        assert_eq!(max_gas_color(6_000_000, None), Color::Cyan);
    }

    #[test]
    fn max_gas_column_can_be_forced_when_all_maxes_are_zero() {
        let mut test_metrics = HashMap::new();
        test_metrics.insert(
            "SystemConfig.setGasLimit".to_string(),
            InvariantMetrics { calls: 10, reverts: 10, discards: 0, ..Default::default() },
        );

        let table = format_invariant_metrics_table(&test_metrics, None, true);
        let mut row = table.row(0).unwrap().cell_iter();
        assert_eq!(row.nth(5).unwrap().content(), "0");
    }
}
