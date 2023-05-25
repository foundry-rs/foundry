use std::io::Write;

use super::CoverageReporter;
use foundry_evm::coverage::{CoverageItemKind, CoverageReport, CoverageSummary};

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
