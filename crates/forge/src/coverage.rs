//! Coverage reports.

use crate::result::{TestKind, TestOutcome, TestResult, TestStatus};
use alloy_primitives::map::{HashMap, HashSet};
use comfy_table::{
    Attribute, Cell, Color, Row, Table,
    presets::{ASCII_FULL, ASCII_MARKDOWN},
};
use foundry_common::{fs, shell};
use foundry_compilers::artifacts::Source;
use foundry_evm::core::bytecode::InstIter;
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
            table.load_style(ASCII_MARKDOWN);
        } else {
            table.load_style(ASCII_FULL.with_rounded_corners());
        }

        table.set_header(vec![
            Cell::new("File"),
            Cell::new("% Lines"),
            Cell::new("% Statements"),
            Cell::new("% Branches"),
            Cell::new("% Funcs"),
        ]);

        Self { table, total: CoverageSummary::new() }
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
        if report.is_instrumented() {
            return self.report_instrumented(report);
        }
        for (path, summary) in report.summary_by_file() {
            self.total.merge(&summary);
            self.add_row(path.display(), summary);
        }

        self.add_row("Total", self.total.clone());
        sh_println!("\n{}", self.table)?;
        Ok(())
    }
}

impl CoverageSummaryReporter {
    /// Prints the summary of an instrumented build, which counts what `llvm-cov report` counts:
    /// lines by `llvm-cov`'s line rules, code regions, branch sides, and functions, and the
    /// conditions that an independence pair covers when a source has MC/DC decisions.
    fn report_instrumented(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        let summaries = report.llvm_file_summaries();
        let mut total = llvm::FileSummary::default();
        for (_, summary) in &summaries {
            total.add(summary);
        }
        let mcdc = total.data.mcdc.total > 0;

        let mut header = vec!["File", "% Lines", "% Regions", "% Branches", "% Funcs"];
        if mcdc {
            header.push("% MC/DC");
        }
        self.table.set_header(header.into_iter().map(Cell::new));
        let rows = summaries
            .iter()
            .map(|(path, summary)| (Cell::new(path.display()), summary))
            .chain([(Cell::new("Total"), &total)]);
        for (name, summary) in rows {
            let mut row = Row::new();
            row.add_cell(name)
                .add_cell(coverage_cell(summary.data.lines))
                .add_cell(coverage_cell(summary.data.regions))
                .add_cell(coverage_cell(summary.data.branches))
                .add_cell(coverage_cell(summary.functions));
            if mcdc {
                row.add_cell(coverage_cell(summary.data.mcdc));
            }
            self.table.add_row(row);
        }
        sh_println!("\n{}", self.table)?;
        Ok(())
    }
}

fn coverage_cell(coverage: llvm::Coverage) -> Cell {
    format_cell(coverage.covered, coverage.total)
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

        // An instrumented build counts the lines, functions, and branches of each file like
        // `llvm-cov`, and reports the lines `llvm-cov` maps.
        let instrumented = report.is_instrumented();
        let llvm_summaries = report.llvm_file_summaries().into_iter().collect::<BTreeMap<_, _>>();

        let mut fn_index = 0usize;
        for (path, items) in report.items_by_file() {
            let summary = if let Some(llvm) = llvm_summaries.get(path) {
                CoverageSummary {
                    line_count: llvm.data.lines.total,
                    line_hits: llvm.data.lines.covered,
                    statement_count: llvm.data.regions.total,
                    statement_hits: llvm.data.regions.covered,
                    branch_count: llvm.data.branches.total,
                    branch_hits: llvm.data.branches.covered,
                    function_count: llvm.functions.total,
                    function_hits: llvm.functions.covered,
                }
            } else {
                CoverageSummary::from_items(&items)
            };

            writeln!(out, "TN:")?;
            writeln!(out, "SF:{}", path.display())?;

            // First pass: collect line hits for DA records.
            // Track both which lines have been recorded and the max hits per line.
            let mut line_hits: HashMap<u32, u32> = HashMap::default();
            for item in &items {
                if matches!(item.kind, CoverageItemKind::Line)
                    || (!instrumented && matches!(item.kind, CoverageItemKind::Statement))
                {
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
                        // Free (file-level) functions have no contract scope; emit the bare
                        // name rather than a leading-dot `.name`.
                        let name = if item.loc.contract_name.is_empty() {
                            name.to_string()
                        } else {
                            format!("{}.{name}", item.loc.contract_name)
                        };
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
                        if (!instrumented || matches!(item.kind, CoverageItemKind::Line))
                            && recorded_lines.insert(line) =>
                    {
                        writeln!(out, "DA:{line},{}", line_hits[&line])?;
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
            let Some(source_path) =
                report.get_source_path(&resolved.contract_id.build_id, item.loc.source_id)
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
            if cta.anchors.is_empty() && rta.anchors.is_empty() {
                continue;
            }

            let anchors = cta
                .anchors
                .iter()
                .map(|anchor| (false, anchor))
                .chain(rta.anchors.iter().map(|anchor| (true, anchor)))
                .filter_map(|(is_runtime, anchor)| {
                    let item = report
                        .analyses
                        .get(&contract_id.build_id)
                        .and_then(|items| items.get(anchor.item_id))?;
                    // Source filters retain analyses to keep anchor item IDs stable, so debug
                    // output must apply the same reportable-source filter as other reporters.
                    report
                        .get_source_path(&contract_id.build_id, item.loc.source_id)
                        .is_some()
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
            let mut formatted = String::new();

            let source_elements =
                report.source_maps.get(contract_id).map(|sm| &sm.1).unwrap_or(&no_source_elements);

            let ops = InstIter::new(hits.bytecode()).with_pc();
            for ((pc, inst), source_element) in std::iter::zip(ops, source_elements) {
                let hits =
                    hits.get(pc as u32).map(|h| format!("[{h:03}]")).unwrap_or("     ".to_owned());
                let source_id = source_element.index();
                let source_path = source_id
                    .and_then(|i| report.get_source_path(&contract_id.build_id, i as usize));

                let code = format!("{pc:08x}: {inst}");
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

/// Writes the coverage of an instrumented build as the files LLVM's tools read:
/// `coverage.proftext`, a text instrumentation profile for `llvm-profdata merge`;
/// `coverage.covmap.o`, an object file with the coverage mapping for `llvm-cov`; and
/// `coverage.json`, the `llvm-cov export` JSON of the two.
pub struct LlvmReporter {
    root: PathBuf,
}

impl LlvmReporter {
    /// Create a new LLVM reporter that writes its files to `root`.
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl CoverageReporter for LlvmReporter {
    fn name(&self) -> &'static str {
        "llvm"
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        let inputs = report.llvm_inputs();
        let object = llvm::CoverageObject::new(&inputs)?;
        fs::write(self.root.join("coverage.proftext"), &object.proftext)?;
        fs::write(self.root.join("coverage.covmap.o"), &object.object)?;

        let json = llvm::export_json(&llvm::CoverageMapping::new(inputs));
        fs::write(self.root.join("coverage.json"), json)?;

        sh_println!(
            "Wrote LLVM coverage report: coverage.proftext, coverage.covmap.o, coverage.json."
        )?;
        Ok(())
    }
}

/// Writes the uncovered coverage items of each source file as JSON.
pub struct GapsReporter {
    root: PathBuf,
    path: PathBuf,
}

impl GapsReporter {
    /// Create a new gaps reporter.
    pub const fn new(root: PathBuf, path: PathBuf) -> Self {
        Self { root, path }
    }
}

impl CoverageReporter for GapsReporter {
    fn name(&self) -> &'static str {
        "gaps"
    }

    fn report(&mut self, report: &CoverageReport) -> eyre::Result<()> {
        let mut files = BTreeMap::new();
        for (path, items) in report.items_by_file() {
            if items.iter().all(|item| item.hits > 0) {
                continue;
            }

            // Item offsets refer to the source the compiler read, with CRLF normalized to LF.
            let src = Source::read(&self.root.join(path))?.content;
            let functions = items
                .iter()
                .filter_map(|item| match &item.kind {
                    CoverageItemKind::Function { name } => Some((&item.loc.bytes, name)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut uncovered = items
                .iter()
                .filter(|item| item.hits == 0)
                .map(|item| {
                    let (kind, name, branch_id, path_id) = coverage_item_kind_fields(&item.kind);
                    let function = name.or_else(|| {
                        functions
                            .iter()
                            .find(|(bytes, _)| {
                                bytes.start <= item.loc.bytes.start
                                    && item.loc.bytes.end <= bytes.end
                            })
                            .map(|(_, name)| name.to_string())
                    });
                    let snippet = src
                        .get(item.loc.bytes())
                        .and_then(|text| text.lines().next())
                        .unwrap_or_default()
                        .trim_end();
                    // A line item reuses the location of the first item on its line, so keep only
                    // that line. `lines` is half-open, so subtract 1 to get the last included line.
                    let (line_end, byte_end) = if matches!(item.kind, CoverageItemKind::Line) {
                        (item.loc.lines.start, item.loc.bytes.start + snippet.len() as u32)
                    } else {
                        (item.loc.lines.end - 1, item.loc.bytes.end)
                    };
                    GapItem {
                        kind,
                        contract: item.loc.contract_name.to_string(),
                        function,
                        line_start: item.loc.lines.start,
                        line_end,
                        byte_start: item.loc.bytes.start,
                        byte_end,
                        snippet: snippet.to_string(),
                        branch_id,
                        path_id,
                    }
                })
                .collect::<Vec<_>>();
            // Drop statements that only repeat a line item.
            let lines = uncovered
                .iter()
                .filter(|item| item.kind == "line")
                .map(|item| (item.byte_start, item.byte_end))
                .collect::<HashSet<_>>();
            uncovered.retain(|item| {
                item.kind != "statement" || !lines.contains(&(item.byte_start, item.byte_end))
            });
            files.insert(path, uncovered);
        }

        let mut out = std::io::BufWriter::new(fs::create_file(&self.path)?);
        serde_json::to_writer_pretty(&mut out, &GapsReport { version: 1, files })?;
        writeln!(out)?;
        out.flush()?;

        sh_println!("Wrote coverage gaps report.")?;

        Ok(())
    }
}

/// Top-level JSON payload for the coverage gaps report.
#[derive(Serialize)]
struct GapsReport<'a> {
    version: u8,
    files: BTreeMap<&'a Path, Vec<GapItem>>,
}

/// An uncovered coverage item.
#[derive(Serialize)]
struct GapItem {
    kind: &'static str,
    contract: String,
    /// The function that contains the item.
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    /// The first line of the item, 1-based.
    line_start: u32,
    /// The last line of the item, 1-based and inclusive.
    line_end: u32,
    /// The start of a 0-based byte range in the LF-normalized source.
    byte_start: u32,
    /// The end of the byte range, exclusive.
    byte_end: u32,
    /// The first source line of the item.
    snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_id: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::instrumented::{InstrumentedBuild, InstrumentedSource};
    use serde_json::json;

    #[test]
    fn empty_summary_cell_is_not_applicable() {
        assert_eq!(
            format_cell(0, 0),
            Cell::new("N/A (0/0)").fg(Color::Grey).add_attribute(Attribute::Dim)
        );
    }

    const SOURCE: &str = "contract C {
    function f(uint a, uint b) public returns (uint) {
        if (a > 1 && b > 2) {
            return 1;
        }
        return 2;
    }
}
";

    /// An instrumented report of `C.f` called three times: with `a` below 2, with `b` below 3,
    /// and with both above.
    fn instrumented_report() -> CoverageReport {
        let span = |needle: &str| (SOURCE.find(needle).unwrap(), needle.len());
        let region = |kind: i64, needle: &str, rest: &[i64]| {
            let (start, len) = span(needle);
            let mut region = vec![kind, 0, start as i64, len as i64];
            region.extend(rest);
            json!(region)
        };
        let body = "function f(uint a, uint b) public returns (uint) {
        if (a > 1 && b > 2) {
            return 1;
        }
        return 2;
    }";
        // Counters 1 to 3 are the entry, `a > 1`, and `b > 2`, as the tagged values 5, 9, and 13.
        // Expressions 0 and 1 are the differences 1 - 2 and 2 - 3, which the tagged values 2 and 6
        // refer to.
        let coverage = serde_json::from_value(json!({
            "version": 2,
            "functions": [{
                "name": "src/C.sol:C.f(uint256,uint256)",
                "hash": "0x1",
                "counters": [1, 3],
                "files": [0],
                "expressions": [[5, 9], [9, 13]],
                "regions": [
                    region(0, body, &[5]),
                    region(5, "a > 1 && b > 2", &[0]),
                    region(6, "a > 1", &[9, 2, 0, -1, 1]),
                    region(6, "b > 2", &[13, 6, 1, -1, -1]),
                    region(0, "{
            return 1;
        }", &[13]),
                    region(0, "return 2;", &[2]),
                ],
                "decisions": [{"id": 0, "conditions": 2, "vectors": ["FFF", "TFF", "TTT"]}],
            }]
        }))
        .unwrap();
        let source = InstrumentedSource::new("src/C.sol".into(), SOURCE.into(), true, coverage);
        let mut build = InstrumentedBuild::new(BTreeMap::from([(0, source)]));
        build.add_hits(
            [(1, 3), (2, 2), (3, 1)].into_iter(),
            [(0, 0), (0, 1), (0, 2)].into_iter(),
            &[],
        );
        let mut report = CoverageReport::default();
        report.add_source("build".into(), 0, "src/C.sol".into());
        report.add_instrumented_build("build".into(), build);
        report.finalize_instrumented();
        report
    }

    #[test]
    fn instrumented_reports_follow_llvm_cov() {
        let report = instrumented_report();
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();

        LlvmReporter::new(dir.to_path_buf()).report(&report).unwrap();
        LcovReporter::new(dir.join("lcov.info"), Version::new(1, 0, 0)).report(&report).unwrap();
        let json = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(dir.join("coverage.json")).unwrap(),
        )
        .unwrap();
        let totals = &json["data"][0]["totals"];
        // The function body, the `if` block, and the last return are the code regions.
        assert_eq!(totals["lines"]["count"], 6);
        assert_eq!(totals["lines"]["covered"], 6);
        assert_eq!(totals["regions"]["count"], 3);
        assert_eq!(totals["branches"]["count"], 4);
        assert_eq!(totals["functions"]["count"], 1);
        assert_eq!(totals["mcdc"]["covered"], 2);
        assert_eq!(json["type"], "llvm.coverage.json.export");
        assert!(fs::read_to_string(dir.join("coverage.proftext")).unwrap().starts_with(":fe\n"));
        assert_eq!(&fs::read(dir.join("coverage.covmap.o")).unwrap()[..4], b"\x7fELF");
        let lcov = fs::read_to_string(dir.join("lcov.info")).unwrap();

        // Line 6 counts the three calls: `llvm-cov` takes the larger of the count a line starts
        // with, from the function around it, and the counts of the regions that start on it.
        assert_eq!(
            lcov,
            "TN:
SF:src/C.sol
DA:2,3
FN:2,C.f
FNDA:3,C.f
DA:3,3
BRDA:3,0,0,2
BRDA:3,0,1,1
BRDA:3,1,0,1
BRDA:3,1,1,1
DA:4,1
DA:5,1
DA:6,3
DA:7,3
FNF:1
FNH:1
LF:6
LH:6
BRF:4
BRH:4
end_of_record
"
        );
    }
}
