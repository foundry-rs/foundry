//! Coverage items of compiler-instrumented builds.
//!
//! An instrumented build, such as Solar's `-Zinstrument-coverage`, maps each source's functions,
//! statements, and branches to counter expressions, and its probes count the counters at runtime.
//! The items here therefore need neither source maps nor bytecode anchors: an item's hits are its
//! counter expression evaluated over the counter hits that
//! [`LineCoverageCollector::counters`](crate::LineCoverageCollector::counters) records.

use crate::{CoverageItem, CoverageItemKind, SourceLocation};
use alloy_primitives::map::HashMap;
use foundry_common::TestFunctionExt;
use foundry_compilers::artifacts::{CoverageCount, SourceCoverage};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

/// Appends the coverage items of one source's coverage map to `items`, and their counter
/// expressions to `counts`.
///
/// Like source-map coverage, it skips the functions of contracts that define tests, gives every
/// branch of the source an ID in source order, disambiguates overloaded functions, and reports each
/// line where a function or statement starts with the hits of the first one on it.
pub fn source_items(
    source_id: usize,
    source: &str,
    coverage: &SourceCoverage,
    items: &mut Vec<CoverageItem>,
    counts: &mut Vec<CoverageCount>,
) {
    let line_starts = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index as u32 + 1))
        .collect::<Vec<_>>();
    let location = |contract_name: &Arc<str>, src: &str| {
        let bytes = parse_src(src)?;
        let line = |offset: u32| line_starts.partition_point(|&start| start <= offset) as u32;
        let lines = line(bytes.start)..line(bytes.end.saturating_sub(1).max(bytes.start)) + 1;
        Some(SourceLocation { source_id, contract_name: contract_name.clone(), bytes, lines })
    };

    let test_contracts = coverage
        .functions
        .iter()
        .filter(|function| function.name.is_any_test())
        .filter_map(|function| function.contract.as_deref())
        .collect::<Vec<_>>();
    let mut overloads = HashMap::<_, usize>::default();
    for function in &coverage.functions {
        *overloads.entry((function.contract.as_deref(), function.name.as_str())).or_default() += 1;
    }
    let mut overload_indices = HashMap::<_, usize>::default();

    let start = items.len();
    let mut branch_id = 0;
    for function in &coverage.functions {
        if function.contract.as_deref().is_some_and(|contract| test_contracts.contains(&contract)) {
            continue;
        }
        let contract_name = Arc::<str>::from(function.contract.as_deref().unwrap_or_default());
        let key = (function.contract.as_deref(), function.name.as_str());
        let name = if overloads[&key] > 1 {
            let index = overload_indices.entry(key).or_default();
            *index += 1;
            format!("{}.{}", function.name, *index - 1).into()
        } else {
            function.name.as_str().into()
        };
        let mut push = |kind, src: &str, count: &CoverageCount| {
            if let Some(loc) = location(&contract_name, src) {
                items.push(CoverageItem { kind, loc, anchor_loc: None, hits: 0 });
                counts.push(count.clone());
            }
        };
        push(CoverageItemKind::Function { name }, &function.src, &function.count);
        for statement in &function.statements {
            push(CoverageItemKind::Statement, &statement.src, &statement.count);
        }
        for branch in &function.branches {
            for (path_id, count) in branch.paths.iter().enumerate() {
                let kind = CoverageItemKind::Branch {
                    branch_id,
                    path_id: path_id as u32,
                    is_first_opcode: false,
                };
                push(kind, &branch.src, count);
            }
            branch_id += 1;
        }
    }

    // line => (first byte, item)
    let mut lines = BTreeMap::<u32, (u32, usize)>::new();
    for (index, item) in items.iter().enumerate().skip(start) {
        if !matches!(item.kind, CoverageItemKind::Function { .. } | CoverageItemKind::Statement) {
            continue;
        }
        let first = (item.loc.bytes.start, index);
        lines
            .entry(item.loc.lines.start)
            .and_modify(|line| *line = (*line).min(first))
            .or_insert(first);
    }
    for (_, index) in lines.into_values() {
        let loc = items[index].loc.clone();
        items.push(CoverageItem { kind: CoverageItemKind::Line, loc, anchor_loc: None, hits: 0 });
        counts.push(counts[index].clone());
    }
}

/// Parses the byte range of a `start:length:sourceIndex` source location.
fn parse_src(src: &str) -> Option<Range<u32>> {
    let mut parts = src.split(':');
    let start = parts.next()?.parse::<u32>().ok()?;
    let length = parts.next()?.parse::<u32>().ok()?;
    Some(start..start + length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_follow_the_coverage_map() {
        let source = "contract C {\n  function f() {\n    a(); b();\n  }\n}\n";
        let coverage: SourceCoverage = serde_json::from_value(serde_json::json!({
            "functions": [
                {"name": "f", "contract": "C", "src": "15:31:0", "count": 1,
                 "statements": [{"src": "35:4:0", "count": 2}, {"src": "40:4:0", "count": 3}],
                 "branches": [{"src": "35:9:0", "paths": [3, {"sub": [2, 3]}]}]},
                {"name": "testF", "contract": "T", "src": "0:1:0", "count": 4},
            ]
        }))
        .unwrap();
        let (mut items, mut counts) = (Vec::new(), Vec::new());
        source_items(7, source, &coverage, &mut items, &mut counts);
        let summary = items
            .iter()
            .zip(&counts)
            .map(|(item, count)| {
                let hits = count.eval(|counter| [0, 10, 6, 4, 9][counter as usize]);
                let kind = match item.kind {
                    CoverageItemKind::Line => "line",
                    CoverageItemKind::Statement => "statement",
                    CoverageItemKind::Branch { .. } => "branch",
                    CoverageItemKind::Function { .. } => "function",
                };
                format!("{kind} {:?} {:?} {hits}", item.loc.lines, item.loc.bytes)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                "function 2..5 15..46 10",
                "statement 3..4 35..39 6",
                "statement 3..4 40..44 4",
                "branch 3..4 35..44 4",
                "branch 3..4 35..44 2",
                "line 2..5 15..46 10",
                "line 3..4 35..39 6",
            ]
        );
        assert!(
            items.iter().all(|item| item.loc.source_id == 7 && &*item.loc.contract_name == "C")
        );
    }
}
