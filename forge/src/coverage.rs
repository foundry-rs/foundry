use comfy_table::{Cell, Color, Row, Table};
pub use foundry_evm::coverage::*;
use std::io::Write;

/// A coverage reporter.
pub trait CoverageReporter {
    fn build(&mut self, map: CoverageMap);
    fn finalize(self) -> eyre::Result<()>;
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
    pub fn new() -> Self {
        Default::default()
    }

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
    fn build(&mut self, map: CoverageMap) {
        for file in map {
            let summary = file.summary();

            self.total.add(&summary);
            self.add_row(file.path.to_string_lossy(), summary);
        }
    }

    fn finalize(mut self) -> eyre::Result<()> {
        self.add_row("Total", self.total.clone());
        println!("{}", self.table);
        Ok(())
    }
}

fn format_cell(hits: usize, total: usize) -> Cell {
    let percentage = if total == 0 { 1. } else { hits as f64 / total as f64 };

    Cell::new(format!("{}% ({hits}/{total})", percentage * 100.)).fg(match percentage {
        _ if percentage < 0.5 => Color::Red,
        _ if percentage < 0.75 => Color::Yellow,
        _ => Color::Green,
    })
}

pub struct LcovReporter<W> {
    /// Destination buffer
    destination: W,
    /// The coverage map to write
    map: Option<CoverageMap>,
}

impl<W> LcovReporter<W> {
    pub fn new(destination: W) -> Self {
        Self { destination, map: None }
    }
}

impl<W> CoverageReporter for LcovReporter<W>
where
    W: Write,
{
    fn build(&mut self, map: CoverageMap) {
        self.map = Some(map);
    }

    fn finalize(mut self) -> eyre::Result<()> {
        let map = self.map.ok_or_else(|| eyre::eyre!("no coverage map given to reporter"))?;

        for file in map {
            let summary = file.summary();

            writeln!(self.destination, "TN:")?;
            writeln!(self.destination, "SF:{}", file.path.to_string_lossy())?;

            // TODO: Line numbers instead of byte offsets
            for item in file.items {
                match item {
                    CoverageItem::Function { name, offset, hits } => {
                        writeln!(self.destination, "FN:{offset},{name}")?;
                        writeln!(self.destination, "FNDA:{hits},{name}")?;
                    }
                    CoverageItem::Line { offset, hits } => {
                        writeln!(self.destination, "DA:{offset},{hits}")?;
                    }
                    CoverageItem::Branch { id, offset, hits, .. } => {
                        // TODO: Block ID
                        writeln!(
                            self.destination,
                            "BRDA:{offset},{id},{id},{}",
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
