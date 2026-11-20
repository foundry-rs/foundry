//! The `llvm-cov export -format=lcov` tracefile, ported from `CoverageExporterLcov.cpp`.

use super::{
    mapping::{CountedRegion, CoverageMapping, Expansion, LineCol},
    segments::CoverageData,
    summary::FileSummary,
};
use std::fmt::Write;

/// A branch with the path of the expansions that lead to it.
struct NestedBranch {
    region: CountedRegion,
    /// The start of each expansion on the way, or the branch's own start.
    path: Vec<LineCol>,
    /// The order in which the branch was collected.
    position: usize,
    /// Whether the branch was summed into the one before it.
    ignore: bool,
}

/// The branches of the expansions and of the expansions they contain.
fn collect_nested_branches(
    mapping: &CoverageMapping,
    expansions: &[Expansion],
    path: &mut Vec<LineCol>,
    position: &mut usize,
) -> Vec<NestedBranch> {
    let mut branches = Vec::new();
    for &expansion in expansions {
        let data = mapping.coverage_for_expansion(expansion);
        let region = mapping.expansion_region(expansion);
        path.push(region.start());
        branches.extend(collect_nested_branches(mapping, &data.expansions, path, position));
        for branch in data.branches.iter().filter(|b| b.file() == region.region.expanded_file) {
            branches.push(NestedBranch {
                region: *branch,
                path: path.clone(),
                position: *position,
                ignore: false,
            });
            *position += 1;
        }
        path.pop();
    }
    branches
}

fn render_branches(out: &mut String, mapping: &CoverageMapping, data: &CoverageData) {
    let unfolded = |region: &CountedRegion| !region.true_folded || !region.false_folded;
    let mut branches = data
        .branches
        .iter()
        .filter(|region| unfolded(region))
        .enumerate()
        .map(|(position, region)| NestedBranch {
            region: *region,
            path: vec![region.start()],
            position,
            ignore: false,
        })
        .collect::<Vec<_>>();
    let nested = collect_nested_branches(mapping, &data.expansions, &mut Vec::new(), &mut 0);
    branches.extend(nested.into_iter().filter(|branch| unfolded(&branch.region)));
    branches.sort_by(|a, b| (&a.path, a.position).cmp(&(&b.path, b.position)));

    // The instances of one branch, such as a modifier's expansions, add up.
    let mut next = 0;
    while next < branches.len() {
        let sum = next;
        next += 1;
        while next < branches.len() && branches[sum].path == branches[next].path {
            let (count, false_count) =
                (branches[next].region.count, branches[next].region.false_count);
            branches[sum].region.count = branches[sum].region.count.wrapping_add(count);
            branches[sum].region.false_count =
                branches[sum].region.false_count.wrapping_add(false_count);
            branches[next].ignore = true;
            next += 1;
        }
    }

    // Branches of one source line are numbered by pair and by side.
    for line_branches in branches.chunk_by(|a, b| a.path[0].0 == b.path[0].0) {
        let line = line_branches[0].path[0].0;
        let mut branch_index = 0;
        for (pair, branch) in line_branches.iter().filter(|branch| !branch.ignore).enumerate() {
            // LLVM keeps the counts of a branch in 32 bits.
            let (true_count, false_count) =
                (branch.region.count as u32, branch.region.false_count as u32);
            let not_executed = true_count == 0 && false_count == 0;
            for count in [true_count, false_count] {
                let _ = write!(out, "BRDA:{line},{pair},{branch_index},");
                if not_executed {
                    out.push('-');
                } else {
                    let _ = write!(out, "{count}");
                }
                out.push('\n');
                branch_index += 1;
            }
        }
    }
}

fn render_file(out: &mut String, mapping: &CoverageMapping, filename: &str) {
    let summary = FileSummary::new(mapping, filename);
    let _ = writeln!(out, "SF:{filename}");

    let functions = mapping.functions().iter().filter(|f| f.filenames[0] == filename);
    for function in functions.clone() {
        let line = function.regions.first().map_or(0, |region| region.start().0);
        let _ = writeln!(out, "FN:{line},{}", function.name);
    }
    for function in functions {
        let _ = writeln!(out, "FNDA:{},{}", function.execution_count, function.name);
    }
    let _ = writeln!(out, "FNF:{}", summary.functions.total);
    let _ = writeln!(out, "FNH:{}", summary.functions.covered);

    let data = mapping.coverage_for_file(filename);
    for stats in data.line_stats_from(1).into_iter().filter(|stats| stats.mapped) {
        let _ = writeln!(out, "DA:{},{}", stats.line, stats.execution_count);
    }
    render_branches(out, mapping, &data);
    let _ = writeln!(out, "BRF:{}", summary.data.branches.total);
    let _ = writeln!(out, "BRH:{}", summary.data.branches.covered);
    let _ = writeln!(out, "LF:{}", summary.data.lines.total);
    let _ = writeln!(out, "LH:{}", summary.data.lines.covered);
    out.push_str("end_of_record\n");
}

/// Renders the coverage mapping as the tracefile `llvm-cov export -format=lcov` prints.
pub fn export_lcov(mapping: &CoverageMapping) -> String {
    let mut out = String::new();
    for filename in mapping.unique_source_files() {
        render_file(&mut out, mapping, filename);
    }
    out
}
