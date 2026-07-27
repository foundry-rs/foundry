//! Coverage reports.

use crate::result::{TestKind, TestOutcome, TestResult, TestStatus};
use alloy_primitives::map::{HashMap, HashSet};
use comfy_table::{
    Attribute, Cell, Color, Row, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN,
};
use evm_disassembler::disassemble_bytes;
use foundry_common::{fs, shell};
use semver::Version;
use serde::{Serialize, ser::SerializeSeq};
use std::{
    collections::{BTreeMap, hash_map},
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
        if shell::is_markdown() {
            table.load_preset(ASCII_MARKDOWN);
        } else {
            table.apply_modifier(UTF8_ROUND_CORNERS);
        }

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
    if total == 0 {
        return Cell::new(format!("N/A ({hits}/{total})"))
            .fg(Color::Grey)
            .add_attribute(Attribute::Dim);
    }

    let percentage = hits as f64 / total as f64;
    Cell::new(format!("{:.2}% ({hits}/{total})", percentage * 100.)).fg(match percentage {
        _ if percentage < 0.5 => Color::Red,
        _ if percentage < 0.75 => Color::Yellow,
        _ => Color::Green,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_summary_cell_is_not_applicable() {
        assert_eq!(
            format_cell(0, 0),
            Cell::new("N/A (0/0)").fg(Color::Grey).add_attribute(Attribute::Dim)
        );
    }
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
    pub const fn new(path: PathBuf, version: Version) -> Self {
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

            // First pass: collect line hits for DA records.
            // Track both which lines have been recorded and the max hits per line.
            let mut line_hits: HashMap<u32, u32> = HashMap::default();
            for item in &items {
                if matches!(item.kind, CoverageItemKind::Line | CoverageItemKind::Statement) {
                    let line = item.loc.lines.start;
                    line_hits
                        .entry(line)
                        .and_modify(|h| *h = (*h).max(item.hits))
                        .or_insert(item.hits);
                }
            }

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
                    CoverageItemKind::Line | CoverageItemKind::Statement
                        if recorded_lines.insert(line) =>
                    {
                        writeln!(out, "DA:{line},{hits}")?;
                    }
                    CoverageItemKind::Branch { branch_id, path_id, .. } => {
                        // Per LCOV spec: "-" means the expression was never evaluated (line not
                        // executed), "0" means branch exists but was never taken.
                        // Check if the line containing this branch was hit.
                        let line_was_hit = line_hits.get(&line).is_some_and(|&h| h > 0);
                        let hits_str = if hits > 0 {
                            hits.to_string()
                        } else if line_was_hit {
                            "0".to_string()
                        } else {
                            "-".to_string()
                        };
                        writeln!(out, "BRDA:{line},{branch_id},{path_id},{hits_str}")?;
                    }
                    _ => {}
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

/// Writes per-test coverage attribution as JSON.
pub struct CoverageAttributionReporter {
    path: PathBuf,
}

/// A hit map resolved to the contract coverage metadata it belongs to.
pub struct ResolvedHitMap {
    pub contract_id: ContractId,
    pub is_deployed_code: bool,
}

pub type ResolvedHitMaps = alloy_primitives::map::B256HashMap<ResolvedHitMap>;

impl CoverageAttributionReporter {
    /// Create a new attribution reporter.
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Writes per-test coverage attribution for the provided outcome.
    pub fn report(
        &self,
        report: &CoverageReport,
        outcome: &TestOutcome,
        resolved_hit_maps: &ResolvedHitMaps,
    ) -> eyre::Result<()> {
        let payload = AttributionReport {
            version: 1,
            tests: AttributionTests { report, outcome, resolved_hit_maps },
        };
        let mut out = std::io::BufWriter::new(fs::create_file(&self.path)?);
        serde_json::to_writer(&mut out, &payload)?;
        writeln!(out)?;
        out.flush()?;

        sh_println!("Wrote coverage attribution report.")?;

        Ok(())
    }
}

/// Top-level JSON payload for per-test coverage attribution.
#[derive(Serialize)]
struct AttributionReport<'a> {
    version: u8,
    tests: AttributionTests<'a>,
}

/// Coverage attributed to a single executed test.
#[derive(Serialize)]
struct AttributionTest {
    suite: String,
    test: String,
    status: &'static str,
    kind: &'static str,
    covered: Vec<AttributionItem>,
}

/// A source range covered by a test, with hit counts and item metadata.
#[derive(Serialize)]
struct AttributionItem {
    source: String,
    contract: String,
    kind: &'static str,
    /// The start of a 1-based, half-open line range.
    line_start: u32,
    /// The end of a 1-based, half-open line range.
    line_end: u32,
    /// The start of a 0-based, half-open byte range.
    byte_start: u32,
    /// The end of a 0-based, half-open byte range.
    byte_end: u32,
    hits: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_id: Option<u32>,
}

/// Serializer state for streaming attribution entries from test results.
struct AttributionTests<'a> {
    report: &'a CoverageReport,
    outcome: &'a TestOutcome,
    resolved_hit_maps: &'a ResolvedHitMaps,
}

impl Serialize for AttributionTests<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let len = self.outcome.results.values().map(|suite| suite.test_results.len()).sum();
        let mut seq = serializer.serialize_seq(Some(len))?;

        for (suite, suite_result) in &self.outcome.results {
            for (test, result) in &suite_result.test_results {
                seq.serialize_element(&AttributionTest {
                    suite: suite.clone(),
                    test: test.clone(),
                    status: test_status_name(result.status),
                    kind: test_kind_name(&result.kind),
                    covered: attributed_items(self.report, self.resolved_hit_maps, result),
                })?;
            }
        }

        seq.end()
    }
}

fn attributed_items(
    report: &CoverageReport,
    resolved_hit_maps: &ResolvedHitMaps,
    result: &TestResult,
) -> Vec<AttributionItem> {
    type AttributionItemKey = (
        String,
        String,
        &'static str,
        u32,
        u32,
        u32,
        u32,
        Option<String>,
        Option<u32>,
        Option<u32>,
    );

    let mut items = BTreeMap::<AttributionItemKey, AttributionItem>::new();
    let Some(hit_maps) = result.line_coverage.as_ref() else { return Vec::new() };

    for (code_hash, map) in &hit_maps.0 {
        let Some(resolved) = resolved_hit_maps.get(code_hash) else { continue };

        for (item, hits) in
            report.hit_items_for_hit_map(&resolved.contract_id, map, resolved.is_deployed_code)
        {
            let Some(source_path) = report
                .source_paths
                .get(&(resolved.contract_id.version.clone(), item.loc.source_id))
            else {
                continue;
            };

            let source = source_path.display().to_string();
            let contract = item.loc.contract_name.to_string();
            let (kind, function, branch_id, path_id) = coverage_item_kind_fields(&item.kind);
            let line_start = item.loc.lines.start;
            let line_end = item.loc.lines.end;
            let byte_start = item.loc.bytes.start;
            let byte_end = item.loc.bytes.end;
            let key = (
                source.clone(),
                contract.clone(),
                kind,
                line_start,
                line_end,
                byte_start,
                byte_end,
                function.clone(),
                branch_id,
                path_id,
            );

            items.entry(key).and_modify(|item| item.hits += hits).or_insert(AttributionItem {
                source,
                contract,
                kind,
                line_start,
                line_end,
                byte_start,
                byte_end,
                hits,
                function,
                branch_id,
                path_id,
            });
        }
    }

    items.into_values().collect()
}

fn coverage_item_kind_fields(
    kind: &CoverageItemKind,
) -> (&'static str, Option<String>, Option<u32>, Option<u32>) {
    match kind {
        CoverageItemKind::Line => ("line", None, None, None),
        CoverageItemKind::Statement => ("statement", None, None, None),
        CoverageItemKind::Branch { branch_id, path_id, .. } => {
            ("branch", None, Some(*branch_id), Some(*path_id))
        }
        CoverageItemKind::Function { name } => ("function", Some(name.to_string()), None, None),
    }
}

const fn test_status_name(status: TestStatus) -> &'static str {
    match status {
        TestStatus::Success => "success",
        TestStatus::Failure => "failure",
        TestStatus::Skipped => "skipped",
    }
}

const fn test_kind_name(kind: &TestKind) -> &'static str {
    match kind {
        TestKind::Unit { .. } => "unit",
        TestKind::Fuzz { .. } => "fuzz",
        TestKind::Invariant { .. } => "invariant",
        TestKind::Table { .. } => "table",
        TestKind::Symbolic { .. } => "symbolic",
        TestKind::Replay { .. } => "replay",
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
            let src = fs::read_to_string(path)?;
            sh_println!("{}:", path.display())?;
            for item in items {
                sh_println!("- {}", item.fmt_with_source(Some(&src)))?;
            }
            sh_println!()?;
        }

        for (contract_id, (cta, rta)) in &report.anchors {
            if cta.is_empty() && rta.is_empty() {
                continue;
            }

            let anchors = cta
                .iter()
                .map(|anchor| (false, anchor))
                .chain(rta.iter().map(|anchor| (true, anchor)))
                .filter_map(|(is_runtime, anchor)| {
                    let item = report
                        .analyses
                        .get(&contract_id.version)
                        .and_then(|items| items.get(anchor.item_id))?;
                    // Source filters retain analyses to keep anchor item IDs stable, so debug
                    // output must apply the same reportable-source filter as other reporters.
                    report
                        .source_paths
                        .contains_key(&(contract_id.version.clone(), item.loc.source_id))
                        .then_some((is_runtime, anchor, item))
                })
                .collect::<Vec<_>>();
            if anchors.is_empty() {
                continue;
            }

            sh_println!("Anchors for {contract_id}:")?;
            for (is_runtime, anchor, item) in anchors {
                let kind = if is_runtime { " runtime" } else { "creation" };
                sh_println!("- {kind} {anchor}: {item}")?;
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
    pub const fn new(root: PathBuf, destdir: PathBuf) -> Self {
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
