//! Coverage reports.

use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, Color, Row, Table};
use evm_disassembler::disassemble_bytes;
use foundry_common::fs;
pub use foundry_evm::coverage::*;
use std::{
    collections::{hash_map, HashMap},
    io::Write,
    path::{Path, PathBuf},
};

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
            self.add_row(path.display(), summary);
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
    pub fn new(destination: &'a mut (dyn Write + 'a)) -> Self {
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
            writeln!(self.destination, "SF:{}", file.display())?;

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
                    CoverageItemKind::Branch { branch_id, path_id, .. } => {
                        writeln!(
                            self.destination,
                            "BRDA:{line},{branch_id},{path_id},{}",
                            if hits == 0 { "-".to_string() } else { hits.to_string() }
                        )?;
                    }
                    // Statements are not in the LCOV format.
                    // We don't add them in order to avoid doubling line hits.
                    _ => {}
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
            println!("Uncovered for {}:", path.display());
            items.iter().for_each(|item| {
                if item.hits == 0 {
                    println!("- {item}");
                }
            });
            println!();
        }

        for (contract_id, anchors) in &report.anchors {
            println!("Anchors for {contract_id}:");
            anchors
                .0
                .iter()
                .map(|anchor| (false, anchor))
                .chain(anchors.1.iter().map(|anchor| (true, anchor)))
                .for_each(|(is_deployed, anchor)| {
                    println!("- {anchor}");
                    if is_deployed {
                        println!("- Creation code");
                    } else {
                        println!("- Runtime code");
                    }
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
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        use std::fmt::Write;

        let no_source_elements = Vec::new();
        let mut line_number_cache = LineNumberCache::new(self.root.clone());

        for (contract_id, hits) in &report.bytecode_hits {
            let ops = disassemble_bytes(hits.bytecode.to_vec())?;
            let mut formatted = String::new();

            let source_elements =
                report.source_maps.get(contract_id).map(|sm| &sm.1).unwrap_or(&no_source_elements);

            for (code, source_element) in std::iter::zip(ops.iter(), source_elements) {
                let hits = hits
                    .hits
                    .get(&(code.offset as usize))
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
        Self { root, line_offsets: HashMap::new() }
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
