//! The `llvm-cov export` JSON, ported from `CoverageExporterJson.cpp`.

use super::{
    mapping::{CondState, CountedRegion, CoverageMapping, Expansion, McdcRecord},
    segments::CoverageSegment,
    summary::{Coverage, FileSummary},
};
use serde_json::{Value, json};

/// The `llvm-cov export` version the JSON follows.
pub const EXPORT_VERSION: &str = "3.1.0";

/// The `llvm-cov export` type identifier.
pub const EXPORT_TYPE: &str = "llvm.coverage.json.export";

/// Clamps a count to the signed range, like `clamp_uint64_to_int64`.
fn clamp(count: u64) -> i64 {
    count.min(i64::MAX as u64) as i64
}

fn render_segment(segment: &CoverageSegment) -> Value {
    json!([
        segment.line,
        segment.col,
        clamp(segment.count),
        segment.has_count,
        segment.is_region_entry,
        segment.is_gap_region
    ])
}

fn render_region(region: &CountedRegion) -> Value {
    json!([
        region.region.start.0,
        region.region.start.1,
        region.region.end.0,
        region.region.end.1,
        clamp(region.count),
        region.region.file,
        region.region.expanded_file,
        region.region.kind as u8
    ])
}

fn render_branch(region: &CountedRegion) -> Value {
    json!([
        region.region.start.0,
        region.region.start.1,
        region.region.end.0,
        region.region.end.1,
        clamp(region.count),
        clamp(region.false_count),
        region.region.file,
        region.region.expanded_file,
        region.region.kind as u8
    ])
}

const fn render_cond_state(state: CondState) -> Value {
    match state {
        CondState::DontCare => Value::Null,
        CondState::True => Value::Bool(true),
        CondState::False => Value::Bool(false),
    }
}

fn render_mcdc_record(record: &McdcRecord) -> Value {
    let region = &record.region;
    let (true_decisions, false_decisions) = record.decisions();
    let conditions = (0..record.num_conditions())
        .map(|c| record.is_independence_pair_covered(c))
        .collect::<Vec<_>>();
    let vectors = record
        .executed
        .iter()
        .map(|(vector, result)| {
            let conditions = (0..record.num_conditions())
                .map(|c| render_cond_state(record.condition(vector, c)))
                .collect::<Vec<_>>();
            json!({"conditions": conditions, "executed": true, "result": render_cond_state(*result)})
        })
        .collect::<Vec<_>>();
    json!([
        region.start.0,
        region.start.1,
        region.end.0,
        region.end.1,
        true_decisions,
        false_decisions,
        region.file,
        region.expanded_file,
        region.kind as u8,
        conditions,
        vectors
    ])
}

fn render_branch_regions(regions: &[CountedRegion]) -> Value {
    regions
        .iter()
        .filter(|region| !region.true_folded || !region.false_folded)
        .map(render_branch)
        .collect()
}

/// The branches of the expansions and of the expansions they contain.
fn collect_nested_branches(
    mapping: &CoverageMapping,
    expansions: &[Expansion],
) -> Vec<CountedRegion> {
    let mut branches = Vec::new();
    for &expansion in expansions {
        let data = mapping.coverage_for_expansion(expansion);
        branches.extend(collect_nested_branches(mapping, &data.expansions));
        let file = mapping.expansion_region(expansion).region.expanded_file;
        branches.extend(data.branches.iter().filter(|branch| branch.file() == file).copied());
    }
    branches
}

fn render_expansion(mapping: &CoverageMapping, expansion: Expansion) -> Value {
    let function = mapping.expansion_function(expansion);
    json!({
        "filenames": function.filenames,
        "branches": render_branch_regions(&collect_nested_branches(mapping, &[expansion])),
        "source_region": render_region(mapping.expansion_region(expansion)),
        "target_regions": function.regions.iter().map(render_region).collect::<Vec<_>>(),
    })
}

fn render_coverage(coverage: Coverage, not_covered: bool, covered_key: &str) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("count".into(), coverage.total.into());
    object.insert(covered_key.into(), coverage.covered.into());
    if not_covered {
        object.insert("notcovered".into(), (coverage.total - coverage.covered).into());
    }
    object.insert("percent".into(), coverage.percent().into());
    Value::Object(object)
}

fn render_summary(summary: &FileSummary) -> Value {
    json!({
        "lines": render_coverage(summary.data.lines, false, "covered"),
        "functions": render_coverage(summary.functions, false, "covered"),
        "instantiations": render_coverage(summary.instantiations, false, "covered"),
        "regions": render_coverage(summary.data.regions, true, "covered"),
        "branches": render_coverage(summary.data.branches, true, "covered"),
        "mcdc": render_coverage(summary.data.mcdc, true, "covered"),
    })
}

fn render_file(mapping: &CoverageMapping, filename: &str, summary: &FileSummary) -> Value {
    let data = mapping.coverage_for_file(filename);
    json!({
        "filename": filename,
        "branches": data.branches.iter().map(render_branch).collect::<Vec<_>>(),
        "expansions": data
            .expansions
            .iter()
            .map(|&expansion| render_expansion(mapping, expansion))
            .collect::<Vec<_>>(),
        "mcdc_records": data.mcdc_records.iter().map(render_mcdc_record).collect::<Vec<_>>(),
        "segments": data.segments.iter().map(render_segment).collect::<Vec<_>>(),
        "summary": render_summary(summary),
    })
}

fn render_function(function: &super::mapping::FunctionRecord) -> Value {
    json!({
        "branches": render_branch_regions(&function.branch_regions),
        "count": clamp(function.execution_count),
        "filenames": function.filenames,
        "mcdc_records": function.mcdc_records.iter().map(render_mcdc_record).collect::<Vec<_>>(),
        "name": function.name,
        "regions": function.regions.iter().map(render_region).collect::<Vec<_>>(),
    })
}

/// Renders the coverage mapping as `llvm-cov export` JSON.
pub fn export_json(mapping: &CoverageMapping) -> Value {
    let mut totals = FileSummary::default();
    let files = mapping
        .unique_source_files()
        .into_iter()
        .map(|filename| {
            let summary = FileSummary::new(mapping, filename);
            totals.add(&summary);
            render_file(mapping, filename, &summary)
        })
        .collect::<Vec<_>>();
    json!({
        "data": [{
            "files": files,
            "functions": mapping.functions().iter().map(render_function).collect::<Vec<_>>(),
            "totals": render_summary(&totals),
        }],
        "type": EXPORT_TYPE,
        "version": EXPORT_VERSION,
    })
}
