//! The `llvm-cov export` JSON, ported from `CoverageExporterJson.cpp`.
//!
//! The text matches what LLVM 23 prints: compact, with the same member order and numbers.

use super::{
    mapping::{CondState, CountedRegion, CoverageMapping, Expansion, McdcRecord},
    segments::CoverageSegment,
    summary::{Coverage, FileSummary},
};

/// The `llvm-cov export` version the JSON follows.
pub const EXPORT_VERSION: &str = "3.1.0";

/// The `llvm-cov export` type identifier.
pub const EXPORT_TYPE: &str = "llvm.coverage.json.export";

/// Clamps a count to the signed range, like `clamp_uint64_to_int64`.
fn clamp(count: u64) -> i64 {
    count.min(i64::MAX as u64) as i64
}

/// A JSON array of the rendered items.
fn array(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

/// A JSON object of the rendered members, in order.
fn object<'a>(members: impl IntoIterator<Item = (&'a str, String)>) -> String {
    let members = members.into_iter().map(|(key, value)| format!("\"{key}\":{value}"));
    format!("{{{}}}", members.collect::<Vec<_>>().join(","))
}

fn string(value: &str) -> String {
    serde_json::to_string(value).expect("a string is valid JSON")
}

fn render_segment(segment: &CoverageSegment) -> String {
    array([
        segment.line.to_string(),
        segment.col.to_string(),
        clamp(segment.count).to_string(),
        segment.has_count.to_string(),
        segment.is_region_entry.to_string(),
        segment.is_gap_region.to_string(),
    ])
}

fn render_region(region: &CountedRegion) -> String {
    array([
        region.region.start.0.to_string(),
        region.region.start.1.to_string(),
        region.region.end.0.to_string(),
        region.region.end.1.to_string(),
        clamp(region.count).to_string(),
        region.region.file.to_string(),
        region.region.expanded_file.to_string(),
        (region.region.kind as u8).to_string(),
    ])
}

fn render_branch(region: &CountedRegion) -> String {
    array([
        region.region.start.0.to_string(),
        region.region.start.1.to_string(),
        region.region.end.0.to_string(),
        region.region.end.1.to_string(),
        clamp(region.count).to_string(),
        clamp(region.false_count).to_string(),
        region.region.file.to_string(),
        region.region.expanded_file.to_string(),
        (region.region.kind as u8).to_string(),
    ])
}

const fn render_cond_state(state: CondState) -> &'static str {
    match state {
        CondState::DontCare => "null",
        CondState::True => "true",
        CondState::False => "false",
    }
}

fn render_mcdc_record(record: &McdcRecord) -> String {
    let region = &record.region;
    let (true_decisions, false_decisions) = record.decisions();
    let conditions = array(
        (0..record.num_conditions()).map(|c| record.is_independence_pair_covered(c).to_string()),
    );
    let vectors = array(record.executed.iter().map(|(vector, result)| {
        let conditions = array(
            (0..record.num_conditions())
                .map(|c| render_cond_state(record.condition(vector, c)).to_string()),
        );
        object([
            ("conditions", conditions),
            ("executed", "true".into()),
            ("result", render_cond_state(*result).into()),
        ])
    }));
    array([
        region.start.0.to_string(),
        region.start.1.to_string(),
        region.end.0.to_string(),
        region.end.1.to_string(),
        true_decisions.to_string(),
        false_decisions.to_string(),
        region.file.to_string(),
        region.expanded_file.to_string(),
        (region.kind as u8).to_string(),
        conditions,
        vectors,
    ])
}

fn render_branch_regions(regions: &[CountedRegion]) -> String {
    array(
        regions
            .iter()
            .filter(|region| !region.true_folded || !region.false_folded)
            .map(render_branch),
    )
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

fn render_expansion(mapping: &CoverageMapping, expansion: Expansion) -> String {
    let function = mapping.expansion_function(expansion);
    object([
        ("filenames", array(function.filenames.iter().map(|name| string(name)))),
        ("branches", render_branch_regions(&collect_nested_branches(mapping, &[expansion]))),
        ("source_region", render_region(mapping.expansion_region(expansion))),
        ("target_regions", array(function.regions.iter().map(render_region))),
    ])
}

/// Renders a double like `printf("%.17g")`, which is how LLVM's JSON writer prints one.
fn format_double(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    // 17 significant digits as `d.dddddddddddddddde±x`.
    let scientific = format!("{value:.16e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific notation");
    let exponent = exponent.parse::<i32>().expect("an exponent");
    let negative = mantissa.starts_with('-');
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    let sign = if negative { "-" } else { "" };
    let mut text = if (-4..17).contains(&exponent) {
        if exponent >= 0 {
            let split = exponent as usize + 1;
            format!("{}.{}", &digits[..split], &digits[split..])
        } else {
            format!("0.{}{}", "0".repeat((-exponent - 1) as usize), digits)
        }
    } else {
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!("{}.{}e{exponent_sign}{:02}", &digits[..1], &digits[1..], exponent.abs())
    };
    // `%g` drops the trailing zeros and a bare decimal point.
    if let Some((mantissa, exponent)) = text.split_once('e') {
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        text = format!("{mantissa}e{exponent}");
    } else if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    format!("{sign}{text}")
}

fn render_coverage(coverage: Coverage, not_covered: bool) -> String {
    let mut members =
        vec![("count", coverage.total.to_string()), ("covered", coverage.covered.to_string())];
    if not_covered {
        members.push(("notcovered", (coverage.total - coverage.covered).to_string()));
    }
    members.push(("percent", format_double(coverage.percent())));
    object(members)
}

fn render_summary(summary: &FileSummary) -> String {
    object([
        ("lines", render_coverage(summary.data.lines, false)),
        ("functions", render_coverage(summary.functions, false)),
        ("instantiations", render_coverage(summary.instantiations, false)),
        ("regions", render_coverage(summary.data.regions, true)),
        ("branches", render_coverage(summary.data.branches, true)),
        ("mcdc", render_coverage(summary.data.mcdc, true)),
    ])
}

fn render_file(mapping: &CoverageMapping, filename: &str, summary: &FileSummary) -> String {
    let data = mapping.coverage_for_file(filename);
    object([
        ("filename", string(filename)),
        ("branches", array(data.branches.iter().map(render_branch))),
        (
            "expansions",
            array(data.expansions.iter().map(|&expansion| render_expansion(mapping, expansion))),
        ),
        ("mcdc_records", array(data.mcdc_records.iter().map(render_mcdc_record))),
        ("segments", array(data.segments.iter().map(render_segment))),
        ("summary", render_summary(summary)),
    ])
}

fn render_function(function: &super::mapping::FunctionRecord) -> String {
    object([
        ("branches", render_branch_regions(&function.branch_regions)),
        ("count", clamp(function.execution_count).to_string()),
        ("filenames", array(function.filenames.iter().map(|name| string(name)))),
        ("mcdc_records", array(function.mcdc_records.iter().map(render_mcdc_record))),
        ("name", string(&function.name)),
        ("regions", array(function.regions.iter().map(render_region))),
    ])
}

/// Renders the coverage mapping as `llvm-cov export` JSON.
pub fn export_json(mapping: &CoverageMapping) -> String {
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
    let data = object([
        ("files", array(files)),
        ("functions", array(mapping.functions().iter().map(render_function))),
        ("totals", render_summary(&totals)),
    ]);
    object([
        ("data", array([data])),
        ("type", string(EXPORT_TYPE)),
        ("version", string(EXPORT_VERSION)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_print_like_printf_g17() {
        assert_eq!(format_double(100.0), "100");
        assert_eq!(format_double(0.0), "0");
        assert_eq!(format_double(50.0), "50");
        assert_eq!(format_double(12.0 / 13.0 * 100.0), "92.307692307692307");
        assert_eq!(format_double(5.0 / 6.0 * 100.0), "83.333333333333343");
        assert_eq!(format_double(0.1), "0.10000000000000001");
        assert_eq!(format_double(1.0 / 3.0 * 0.001), "0.00033333333333333332");
    }
}
