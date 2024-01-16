use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, Color, Row, Table};
use foundry_evm::coverage::{CoverageReport, CoverageSummary};

use super::CoverageReporter;

/// A simple summary reporter that prints the coverage results in a table.
pub struct SummaryReporter {
    /// The summary table.
    table: Table,
    /// The total coverage of the entire project.
    total: CoverageSummary,
}

impl Default for SummaryReporter {
    fn default() -> Self {
        let mut table = Table::new();
        table.load_preset(ASCII_MARKDOWN);
        table.set_header(["File", "% Lines", "% Statements", "% Branches", "% Funcs"]);

        Self { table, total: CoverageSummary::default() }
    }
}

impl SummaryReporter {
    fn add_row(&mut self, name: impl Into<Cell>, summary: CoverageSummary) {
        let mut row = Row::new();
        row.add_cell(name.into())
            .add_cell(format_cell(summary.line_hits, summary.line_count))
            .add_cell(format_cell(summary.statement_hits, summary.statement_count))
            .add_cell(format_cell(summary.branch_hits, summary.branch_count))
            .add_cell(format_cell(summary.function_hits, summary.function_count));
        self.table.add_row(row);
    }
}

impl CoverageReporter for SummaryReporter {
    fn report(mut self, report: &CoverageReport) -> eyre::Result<()> {
        for (path, summary) in report.summary_by_file() {
            self.total += &summary;
            self.add_row(path, summary);
        }

        self.add_row("Total", self.total.clone());
        println!("{}", self.table);
        Ok(())
    }
}

fn format_cell(hits: usize, total: usize) -> Cell {
    let percentage = if total == 0 { 1. } else { hits as f64 / total as f64 };

    let mut cell =
        Cell::new(format!("{:.2}% ({hits}/{total})", percentage * 100.)).fg(match percentage {
            _ if total == 0 => Color::Grey,
            _ if percentage < 0.5 => Color::Red,
            _ if percentage < 0.75 => Color::Yellow,
            _ => Color::Green,
        });

    if total == 0 {
        cell = cell.add_attribute(Attribute::Dim);
    }
    cell
}
