//! Coverage reports.

use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, Color, Row, Table};
pub use foundry_evm::coverage::*;
use std::io::Write;

/// A coverage reporter.
pub trait CoverageReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()>;
}

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

pub struct LcovReporter<'a> {
    /// Destination buffer
    destination: &'a mut (dyn Write + 'a),
}

impl<'a> LcovReporter<'a> {
    pub fn new(destination: &'a mut (dyn Write + 'a)) -> LcovReporter<'a> {
        Self { destination }
    }
}

impl<'a> CoverageReporter for LcovReporter<'a> {
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        for (file, items) in report.items_by_source() {
            let summary = items.iter().fold(CoverageSummary::default(), |mut summary, item| {
                summary += item;
                summary
            });

            writeln!(self.destination, "TN:")?;
            writeln!(self.destination, "SF:{file}")?;

            for item in items {
                let line = item.loc.line;
                let hits = item.hits;
                match item.kind {
                    CoverageItemKind::Function { name } => {
                        let name = format!("{}.{name}", item.loc.contract_name);
                        writeln!(self.destination, "FN:{line},{name}")?;
                        writeln!(self.destination, "FNDA:{hits},{name}")?;
                    }
                    CoverageItemKind::Line => {
                        writeln!(self.destination, "DA:{line},{hits}")?;
                    }
                    CoverageItemKind::Branch { branch_id, path_id } => {
                        writeln!(
                            self.destination,
                            "BRDA:{line},{branch_id},{path_id},{}",
                            if hits == 0 { "-".to_string() } else { hits.to_string() }
                        )?;
                    }
                    // Statements are not in the LCOV format
                    CoverageItemKind::Statement => (),
                }
            }

            // Function summary
            writeln!(self.destination, "FNF:{}", summary.function_count)?;
            writeln!(self.destination, "FNH:{}", summary.function_hits)?;

            // Line summary
            writeln!(self.destination, "LF:{}", summary.line_count)?;
            writeln!(self.destination, "LH:{}", summary.line_hits)?;

            // Branch summary
            writeln!(self.destination, "BRF:{}", summary.branch_count)?;
            writeln!(self.destination, "BRH:{}", summary.branch_hits)?;

            writeln!(self.destination, "end_of_record")?;
        }

        println!("Wrote LCOV report.");

        Ok(())
    }
}

/// A super verbose reporter for debugging coverage while it is still unstable.
pub struct DebugReporter;

impl CoverageReporter for DebugReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        for (path, items) in report.items_by_source() {
            println!("Uncovered for {path}:");
            items.iter().for_each(|item| {
                if item.hits == 0 {
                    println!("- {item}");
                }
            });
            println!();
        }

        for (contract_id, anchors) in &report.anchors {
            println!("Anchors for {contract_id}:");
            anchors.iter().for_each(|anchor| {
                println!("- {anchor}");
                println!(
                    "  - Refers to item: {}",
                    report
                        .items
                        .get(&contract_id.version)
                        .and_then(|items| items.get(anchor.item_id))
                        .map_or("None".to_owned(), |item| item.to_string())
                );
            });
            println!();
        }

        Ok(())
    }
}
