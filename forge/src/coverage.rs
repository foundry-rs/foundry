use comfy_table::{Attribute, Cell, Color, Row, Table};
pub use foundry_evm::coverage::*;
use std::{collections::HashMap, io::Write, path::PathBuf};

/// A coverage reporter.
pub trait CoverageReporter {
    fn report(self, map: CoverageMap) -> eyre::Result<()>;
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
        table.set_header(&["File", "% Lines", "% Statements", "% Branches", "% Funcs"]);

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
    fn report(mut self, map: CoverageMap) -> eyre::Result<()> {
        for file in map {
            let summary = file.summary();

            self.total += &summary;
            self.add_row(file.path.to_string_lossy(), summary);
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
    fn report(self, map: CoverageMap) -> eyre::Result<()> {
        for file in map {
            let summary = file.summary();

            writeln!(self.destination, "TN:")?;
            writeln!(self.destination, "SF:{}", file.path.to_string_lossy())?;

            for item in file.items {
                match item {
                    CoverageItem::Function {
                        loc: SourceLocation { line, .. }, name, hits, ..
                    } => {
                        writeln!(self.destination, "FN:{line},{name}")?;
                        writeln!(self.destination, "FNDA:{hits},{name}")?;
                    }
                    CoverageItem::Line { loc: SourceLocation { line, .. }, hits, .. } => {
                        writeln!(self.destination, "DA:{line},{hits}")?;
                    }
                    CoverageItem::Branch {
                        loc: SourceLocation { line, .. },
                        branch_id,
                        path_id,
                        hits,
                        ..
                    } => {
                        writeln!(
                            self.destination,
                            "BRDA:{line},{branch_id},{path_id},{}",
                            if hits == 0 { "-".to_string() } else { hits.to_string() }
                        )?;
                    }
                    // Statements are not in the LCOV format
                    CoverageItem::Statement { .. } => (),
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
pub struct DebugReporter {
    /// The summary table.
    table: Table,
    /// The total coverage of the entire project.
    total: CoverageSummary,
    /// Uncovered items
    uncovered: HashMap<PathBuf, Vec<CoverageItem>>,
}

impl Default for DebugReporter {
    fn default() -> Self {
        let mut table = Table::new();
        table.set_header(&["File", "% Lines", "% Statements", "% Branches", "% Funcs"]);

        Self { table, total: CoverageSummary::default(), uncovered: HashMap::default() }
    }
}

impl DebugReporter {
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

impl CoverageReporter for DebugReporter {
    fn report(mut self, map: CoverageMap) -> eyre::Result<()> {
        for file in map {
            let summary = file.summary();

            self.total += &summary;
            self.add_row(file.path.to_string_lossy(), summary);

            file.items.iter().for_each(|item| {
                if item.hits() == 0 {
                    self.uncovered.entry(file.path.clone()).or_default().push(item.clone());
                }
            })
        }

        self.add_row("Total", self.total.clone());
        println!("{}", self.table);

        for (path, items) in self.uncovered {
            println!("Uncovered for {}:", path.to_string_lossy());
            items.iter().for_each(|item| {
                if item.hits() == 0 {
                    println!("- {}", item);
                }
            });
            println!();
        }
        Ok(())
    }
}
