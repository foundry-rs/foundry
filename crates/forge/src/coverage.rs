//! Coverage reports.

use alloy_primitives::map::{HashMap, HashSet};
use comfy_table::{Attribute, Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS};
use evm_disassembler::disassemble_bytes;
use foundry_common::fs;
use semver::Version;
use std::{
    collections::hash_map,
    io::Write,
    path::{Path, PathBuf},
};

pub use foundry_evm::coverage::*;

/// A coverage reporter.
pub trait CoverageReporter {
    /// Returns a debug string for the reporter.
    fn name(&self) -> &'static str;

    /// Returns `true` if the reporter needs source maps for the final report.
    fn needs_source_maps(&self) -> bool {
        false
    }

    /// Runs the reporter.
    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()>;
}

/// A simple summary reporter that prints the coverage results in a table.
pub struct CoverageSummaryReporter {
    /// The summary table.
    table: Table,
    /// The total coverage of the entire project.
    total: CoverageSummary,
}

impl Default for CoverageSummaryReporter {
    fn default() -> Self {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

        table.set_header(vec![
            Cell::new("File"),
            Cell::new("% Lines"),
            Cell::new("% Statements"),
            Cell::new("% Branches"),
            Cell::new("% Funcs"),
        ]);

        Self { table, total: CoverageSummary::default() }
    }
}

impl CoverageSummaryReporter {
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

impl CoverageReporter for CoverageSummaryReporter {
    fn name(&self) -> &'static str {
        "summary"
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        for (path, summary) in report.summary_by_file() {
            self.total.merge(&summary);
            self.add_row(path.display(), summary);
        }

        self.add_row("Total", self.total.clone());
        sh_println!("\n{}", self.table)?;
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

/// Writes the coverage report in [LCOV]'s [tracefile format].
///
/// [LCOV]: https://github.com/linux-test-project/lcov
/// [tracefile format]: https://man.archlinux.org/man/geninfo.1.en#TRACEFILE_FORMAT
pub struct LcovReporter {
    path: PathBuf,
    version: Version,
}

impl LcovReporter {
    /// Create a new LCOV reporter.
    pub fn new(path: PathBuf, version: Version) -> Self {
        Self { path, version }
    }
}

impl CoverageReporter for LcovReporter {
    fn name(&self) -> &'static str {
        "lcov"
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        let mut out = std::io::BufWriter::new(fs::create_file(&self.path)?);

        let mut fn_index = 0usize;
        for (path, items) in report.items_by_file() {
            let summary = CoverageSummary::from_items(items.iter().copied());

            writeln!(out, "TN:")?;
            writeln!(out, "SF:{}", path.display())?;

            let mut recorded_lines = HashSet::new();

            for item in items {
                let line = item.loc.lines.start;
                // `lines` is half-open, so we need to subtract 1 to get the last included line.
                let end_line = item.loc.lines.end - 1;
                let hits = item.hits;
                match item.kind {
                    CoverageItemKind::Function { ref name } => {
                        let name = format!("{}.{name}", item.loc.contract_name);
                        if self.version >= Version::new(2, 2, 0) {
                            // v2.2 changed the FN format.
                            writeln!(out, "FNL:{fn_index},{line},{end_line}")?;
                            writeln!(out, "FNA:{fn_index},{hits},{name}")?;
                            fn_index += 1;
                        } else if self.version >= Version::new(2, 0, 0) {
                            // v2.0 added end_line to FN.
                            writeln!(out, "FN:{line},{end_line},{name}")?;
                            writeln!(out, "FNDA:{hits},{name}")?;
                        } else {
                            writeln!(out, "FN:{line},{name}")?;
                            writeln!(out, "FNDA:{hits},{name}")?;
                        }
                    }
                    // Add lines / statement hits only once.
                    CoverageItemKind::Line | CoverageItemKind::Statement => {
                        if recorded_lines.insert(line) {
                            writeln!(out, "DA:{line},{hits}")?;
                        }
                    }
                    CoverageItemKind::Branch { branch_id, path_id, .. } => {
                        writeln!(
                            out,
                            "BRDA:{line},{branch_id},{path_id},{}",
                            if hits == 0 { "-".to_string() } else { hits.to_string() }
                        )?;
                    }
                }
            }

            // Function summary
            writeln!(out, "FNF:{}", summary.function_count)?;
            writeln!(out, "FNH:{}", summary.function_hits)?;

            // Line summary
            writeln!(out, "LF:{}", summary.line_count)?;
            writeln!(out, "LH:{}", summary.line_hits)?;

            // Branch summary
            writeln!(out, "BRF:{}", summary.branch_count)?;
            writeln!(out, "BRH:{}", summary.branch_hits)?;

            writeln!(out, "end_of_record")?;
        }

        out.flush()?;
        sh_println!("Wrote LCOV report.")?;

        Ok(())
    }
}

/// A super verbose reporter for debugging coverage while it is still unstable.
pub struct DebugReporter;

impl CoverageReporter for DebugReporter {
    fn name(&self) -> &'static str {
        "debug"
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        for (path, items) in report.items_by_file() {
            let uncovered = items.iter().copied().filter(|item| item.hits == 0);
            if uncovered.clone().count() == 0 {
                continue;
            }

            sh_println!("Uncovered for {}:", path.display())?;
            for item in uncovered {
                sh_println!("- {item}")?;
            }
            sh_println!()?;
        }

        for (contract_id, (cta, rta)) in &report.anchors {
            if cta.is_empty() && rta.is_empty() {
                continue;
            }

            sh_println!("Anchors for {contract_id}:")?;
            let anchors = cta
                .iter()
                .map(|anchor| (false, anchor))
                .chain(rta.iter().map(|anchor| (true, anchor)));
            for (is_runtime, anchor) in anchors {
                let kind = if is_runtime { " runtime" } else { "creation" };
                sh_println!(
                    "- {kind} {anchor}: {}",
                    report
                        .analyses
                        .get(&contract_id.version)
                        .and_then(|items| items.get(anchor.item_id))
                        .map_or_else(|| "None".to_owned(), |item| item.to_string())
                )?;
            }
            sh_println!()?;
        }

        Ok(())
    }
}

pub struct BytecodeReporter {
    root: PathBuf,
    destdir: PathBuf,
}

impl BytecodeReporter {
    pub fn new(root: PathBuf, destdir: PathBuf) -> Self {
        Self { root, destdir }
    }
}

impl CoverageReporter for BytecodeReporter {
    fn name(&self) -> &'static str {
        "bytecode"
    }

    fn needs_source_maps(&self) -> bool {
        true
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        use std::fmt::Write;

        fs::create_dir_all(&self.destdir)?;

        let no_source_elements = Vec::new();
        let mut line_number_cache = LineNumberCache::new(self.root.clone());

        for (contract_id, hits) in &report.bytecode_hits {
            let ops = disassemble_bytes(hits.bytecode().to_vec())?;
            let mut formatted = String::new();

            let source_elements =
                report.source_maps.get(contract_id).map(|sm| &sm.1).unwrap_or(&no_source_elements);

            for (code, source_element) in std::iter::zip(ops.iter(), source_elements) {
                let hits = hits
                    .get(code.offset)
                    .map(|h| format!("[{h:03}]"))
                    .unwrap_or("     ".to_owned());
                let source_id = source_element.index();
                let source_path = source_id.and_then(|i| {
                    report.source_paths.get(&(contract_id.version.clone(), i as usize))
                });

                let code = format!("{code:?}");
                let start = source_element.offset() as usize;
                let end = (source_element.offset() + source_element.length()) as usize;

                if let Some(source_path) = source_path {
                    let (sline, spos) = line_number_cache.get_position(source_path, start)?;
                    let (eline, epos) = line_number_cache.get_position(source_path, end)?;
                    writeln!(
                        formatted,
                        "{} {:40} // {}: {}:{}-{}:{} ({}-{})",
                        hits,
                        code,
                        source_path.display(),
                        sline,
                        spos,
                        eline,
                        epos,
                        start,
                        end
                    )?;
                } else if let Some(source_id) = source_id {
                    writeln!(formatted, "{hits} {code:40} // SRCID{source_id}: ({start}-{end})")?;
                } else {
                    writeln!(formatted, "{hits} {code:40}")?;
                }
            }
            fs::write(
                self.destdir.join(&*contract_id.contract_name).with_extension("asm"),
                formatted,
            )?;
        }

        Ok(())
    }
}

/// Cache line number offsets for source files
struct LineNumberCache {
    root: PathBuf,
    line_offsets: HashMap<PathBuf, Vec<usize>>,
}

impl LineNumberCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root, line_offsets: HashMap::default() }
    }

    pub fn get_position(&mut self, path: &Path, offset: usize) -> eyre::Result<(usize, usize)> {
        let line_offsets = match self.line_offsets.entry(path.to_path_buf()) {
            hash_map::Entry::Occupied(o) => o.into_mut(),
            hash_map::Entry::Vacant(v) => {
                let text = fs::read_to_string(self.root.join(path))?;
                let mut line_offsets = vec![0];
                for line in text.lines() {
                    let line_offset = line.as_ptr() as usize - text.as_ptr() as usize;
                    line_offsets.push(line_offset);
                }
                v.insert(line_offsets)
            }
        };
        let lo = match line_offsets.binary_search(&offset) {
            Ok(lo) => lo,
            Err(lo) => lo - 1,
        };
        let pos = offset - line_offsets.get(lo).unwrap() + 1;
        Ok((lo, pos))
    }
}
