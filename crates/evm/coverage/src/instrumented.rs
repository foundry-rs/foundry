//! Coverage of compiler-instrumented builds.
//!
//! An instrumented build, such as Solar's `-Zinstrument-coverage`, maps each source's functions to
//! LLVM-style records: regions over counters and counter expressions. Its probes count the
//! counters at runtime, so a record's coverage needs neither source maps nor bytecode anchors.
//!
//! [`InstrumentedBuild`] holds the sources of a build with the counter and MC/DC hits of every
//! executed code object. It turns them into the coverage items that the item-based reports use,
//! and into the [`FunctionInput`]s of the LLVM coverage mapping that the summary, LCOV, and LLVM
//! reports use.

use crate::{
    CoverageItem, CoverageItemKind, SourceLocation,
    llvm::{
        Counter, Expression, FunctionInput, MappingRegion, McdcParams, RegionKind, decision_vectors,
    },
};
use alloy_primitives::map::rustc_hash::{FxHashMap, FxHashSet};
use foundry_common::TestFunctionExt;
use foundry_compilers::artifacts::{
    Counter as TaggedCounter, CoverageAlias, FunctionCoverage, Region, SourceCoverage,
};
use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc};

/// A source of an instrumented build with its coverage map.
#[derive(Clone, Debug)]
pub struct InstrumentedSource {
    /// The project-relative path.
    pub path: PathBuf,
    /// The text the compiler read, with line endings normalized.
    pub text: String,
    /// Whether the report includes the source. A source that the report excludes, such as a
    /// library, still maps the regions of modifiers expanded in an included source.
    pub reportable: bool,
    /// The coverage map.
    pub coverage: SourceCoverage,
    /// The byte offset of the start of each line.
    line_starts: Vec<u32>,
}

impl InstrumentedSource {
    /// Creates a source.
    pub fn new(path: PathBuf, text: String, reportable: bool, coverage: SourceCoverage) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index as u32 + 1))
            .collect();
        Self { path, text, reportable, coverage, line_starts }
    }

    /// The 1-based line and column of a byte offset.
    fn line_col(&self, offset: u32) -> (u32, u32) {
        let offset = offset.min(self.text.len() as u32);
        let line = self.line_starts.partition_point(|&start| start <= offset);
        (line as u32, offset - self.line_starts[line - 1] + 1)
    }

    /// The 1-based line of a byte offset.
    fn line(&self, offset: u32) -> u32 {
        self.line_col(offset).0
    }

    /// The byte range of a 1-based line, without its line break.
    pub fn line_bytes(&self, line: u32) -> Range<u32> {
        let start =
            self.line_starts.get(line as usize - 1).copied().unwrap_or(self.text.len() as u32);
        let end = self
            .line_starts
            .get(line as usize)
            .map_or(self.text.len() as u32, |&next| next.saturating_sub(1))
            .max(start);
        start..end
    }

    /// The path with `/` separators, as the coverage mapping names the file.
    pub fn filename(&self) -> String {
        self.path.to_string_lossy().replace('\\', "/")
    }
}

/// The counter and MC/DC hits of the executed code objects of a build, and the build's sources.
#[derive(Clone, Debug, Default)]
pub struct InstrumentedBuild {
    /// The sources by source ID.
    pub sources: BTreeMap<u32, InstrumentedSource>,
    /// The hits of each counter, summed over all code objects.
    hits: FxHashMap<u32, u64>,
    /// The executed test vector indices of each MC/DC decision, by decision ID.
    mcdc: FxHashMap<u32, FxHashSet<u32>>,
}

/// An item's count: a tagged counter of a function record.
#[derive(Clone, Copy, Debug)]
pub struct ItemCount {
    /// The source ID.
    source: u32,
    /// The index of the record in the source's map.
    function: u32,
    /// The tagged counter.
    count: u32,
}

impl InstrumentedBuild {
    /// Creates a build from its sources, by source ID.
    pub fn new(sources: BTreeMap<u32, InstrumentedSource>) -> Self {
        Self { sources, ..Default::default() }
    }

    /// Adds the hits of one code object, after the compiler's counter aliases of that object.
    pub fn add_hits(
        &mut self,
        counters: impl Iterator<Item = (u32, u32)>,
        mcdc: impl Iterator<Item = (u32, u32)>,
        aliases: &[CoverageAlias],
    ) {
        if aliases.is_empty() {
            for (counter, hits) in counters {
                *self.hits.entry(counter).or_default() += u64::from(hits);
            }
        } else {
            // A dropped counter takes the hits of the counter that stands in for it.
            let mut local = counters.collect::<FxHashMap<_, _>>();
            for &[dropped, kept] in aliases {
                let hits = local.get(&kept).copied().unwrap_or(0);
                local.insert(dropped, hits);
            }
            for (counter, hits) in local {
                *self.hits.entry(counter).or_default() += u64::from(hits);
            }
        }
        for (decision, vector) in mcdc {
            self.mcdc.entry(decision).or_default().insert(vector);
        }
    }

    fn hits(&self, counter: u32) -> u64 {
        self.hits.get(&counter).copied().unwrap_or(0)
    }

    /// The hits of a tagged counter of a record. A negative count is zero.
    fn eval(&self, function: &FunctionCoverage, tagged: u32, hits: &impl Fn(u32) -> u64) -> u64 {
        let value = self.eval_signed(function, tagged, hits);
        u64::try_from(value).unwrap_or(0)
    }

    fn eval_signed(
        &self,
        function: &FunctionCoverage,
        tagged: u32,
        hits: &impl Fn(u32) -> u64,
    ) -> i64 {
        match TaggedCounter::decode(tagged) {
            TaggedCounter::Zero => 0,
            TaggedCounter::Counter(id) => hits(id) as i64,
            TaggedCounter::Subtract(id) | TaggedCounter::Add(id) => {
                let Some(&[lhs, rhs]) = function.expressions.get(id as usize) else { return 0 };
                let (lhs, rhs) =
                    (self.eval_signed(function, lhs, hits), self.eval_signed(function, rhs, hits));
                if matches!(TaggedCounter::decode(tagged), TaggedCounter::Subtract(_)) {
                    lhs.wrapping_sub(rhs)
                } else {
                    lhs.wrapping_add(rhs)
                }
            }
        }
    }

    /// The coverage items of the reportable sources and the count of each item.
    ///
    /// Like source-map coverage, the items skip the functions of contracts that define tests,
    /// give every branch of a source its own ID, and disambiguate overloaded functions. A
    /// function is one item with the entry count, each code region of the function's own file is a
    /// statement, and each side of a branch is a path of a branch.
    pub fn items(&self) -> (Vec<CoverageItem>, Vec<ItemCount>) {
        let (mut items, mut counts) = (Vec::new(), Vec::new());
        for (&source_id, source) in &self.sources {
            if !source.reportable {
                continue;
            }
            let names = source
                .coverage
                .functions
                .iter()
                .map(|function| parse_name(&function.name))
                .collect::<Vec<_>>();
            let test_contracts = names
                .iter()
                .filter(|(_, function)| function.is_any_test())
                .filter_map(|(contract, _)| *contract)
                .collect::<Vec<_>>();
            let mut overloads = FxHashMap::<_, usize>::default();
            for &(contract, function) in &names {
                *overloads.entry((contract, function)).or_default() += 1;
            }
            let mut overload_indices = FxHashMap::<_, usize>::default();
            let mut branch_id = 0;
            for (index, (function, &(contract, function_name))) in
                source.coverage.functions.iter().zip(&names).enumerate()
            {
                if contract.is_some_and(|contract| test_contracts.contains(&contract)) {
                    continue;
                }
                let Some(first) = function.regions.first() else { continue };
                let contract_name = Arc::<str>::from(contract.unwrap_or_default());
                let name = if overloads[&(contract, function_name)] > 1 {
                    let overload = overload_indices.entry((contract, function_name)).or_default();
                    *overload += 1;
                    format!("{function_name}.{}", *overload - 1).into()
                } else {
                    function_name.into()
                };
                let location = |region: &Region| {
                    let bytes = region.start()..region.start().saturating_add(region.len());
                    let end = bytes.end.saturating_sub(1).max(bytes.start);
                    let lines = source.line(bytes.start)..source.line(end) + 1;
                    SourceLocation {
                        source_id: source_id as usize,
                        contract_name: contract_name.clone(),
                        bytes,
                        lines,
                    }
                };
                let mut push = |kind, region: &Region, count| {
                    items.push(CoverageItem {
                        kind,
                        loc: location(region),
                        anchor_loc: None,
                        hits: 0,
                    });
                    counts.push(ItemCount { source: source_id, function: index as u32, count });
                };
                // A record's entry count is its first region's count. The record of the
                // creation-time code outside functions, `<init>`, has no body region, so it is not
                // a function.
                let has_body = !function_name.starts_with('<');
                if has_body && let Region::Code { count, .. } = first {
                    push(CoverageItemKind::Function { name }, first, *count);
                }
                for region in function.regions.iter().filter(|region| region.file() == 0) {
                    match *region {
                        Region::Code { count, .. } if !has_body || !std::ptr::eq(region, first) => {
                            push(CoverageItemKind::Statement, region, count);
                        }
                        Region::Branch { true_count, false_count, .. }
                        | Region::McdcBranch { true_count, false_count, .. } => {
                            for (path_id, count) in
                                [true_count, false_count].into_iter().enumerate()
                            {
                                // A side that constant folding removed is not a path.
                                if count & 3 == 0 {
                                    continue;
                                }
                                let kind = CoverageItemKind::Branch {
                                    branch_id,
                                    path_id: path_id as u32,
                                    is_first_opcode: false,
                                };
                                push(kind, region, count);
                            }
                            branch_id += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        (items, counts)
    }

    /// The hits of an item, from the hits of a code object after its aliases.
    pub fn item_hits(&self, count: ItemCount, hits: &impl Fn(u32) -> u64) -> u32 {
        let function = &self.sources[&count.source].coverage.functions[count.function as usize];
        u32::try_from(self.eval(function, count.count, hits)).unwrap_or(u32::MAX)
    }

    /// The hits of an item, from the hits of all code objects.
    pub fn total_item_hits(&self, count: ItemCount) -> u32 {
        self.item_hits(count, &|counter| self.hits(counter))
    }

    /// The function records for the LLVM coverage mapping, with their profile counters and MC/DC
    /// bitmap. A function record needs one of the reportable sources to define it.
    pub fn function_inputs(&self) -> Vec<FunctionInput> {
        let mut inputs = Vec::new();
        for source in self.sources.values().filter(|source| source.reportable) {
            let test_contracts = source
                .coverage
                .functions
                .iter()
                .map(|function| parse_name(&function.name))
                .filter(|(_, function)| function.is_any_test())
                .filter_map(|(contract, _)| contract)
                .collect::<Vec<_>>();
            for function in &source.coverage.functions {
                if parse_name(&function.name).0.is_some_and(|c| test_contracts.contains(&c)) {
                    continue;
                }
                inputs.extend(self.function_input(function));
            }
        }
        inputs
    }

    /// Builds a function record for the LLVM coverage mapping.
    fn function_input(&self, function: &FunctionCoverage) -> Option<FunctionInput> {
        // Drop the files that the report excludes, and the regions in them.
        let sources = function
            .files
            .iter()
            .map(|id| self.sources.get(id))
            .collect::<Vec<Option<&InstrumentedSource>>>();
        sources.first().copied().flatten()?;
        let mut file_map = vec![None; sources.len()];
        let mut filenames = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            if let Some(source) = source
                && (index == 0 || source.reportable)
                && function.regions.iter().any(|region| region.file() as usize == index)
            {
                file_map[index] = Some(filenames.len() as u32);
                filenames.push(source.filename());
            }
        }
        file_map[0]?;
        // An expansion into a dropped file goes too.
        let regions = function
            .regions
            .iter()
            .filter(|region| {
                file_map.get(region.file() as usize).copied().flatten().is_some()
                    && match region {
                        Region::Expansion { expanded_file, .. } => {
                            file_map.get(*expanded_file as usize).copied().flatten().is_some()
                        }
                        _ => true,
                    }
            })
            .collect::<Vec<_>>();

        // The record's own counters are the first counters of the profile.
        let [first, len] = function.counters;
        let mut builder =
            ExpressionBuilder { function, expressions: Vec::new(), built: Default::default() };

        let mut mapped = Vec::with_capacity(regions.len());
        for region in &regions {
            let file = file_map[region.file() as usize].unwrap();
            let source = sources[region.file() as usize].unwrap();
            let start = source.line_col(region.start());
            let end = source.line_col(region.start().saturating_add(region.len()));
            let mut decision = None;
            let (kind, count, false_count, mcdc, expanded_file) = match **region {
                Region::Code { count, .. } => {
                    (RegionKind::Code, builder.counter(count), Counter::Zero, McdcParams::None, 0)
                }
                Region::Expansion { expanded_file, .. } => (
                    RegionKind::Expansion,
                    Counter::Zero,
                    Counter::Zero,
                    McdcParams::None,
                    file_map[expanded_file as usize].unwrap(),
                ),
                Region::Skipped { .. } => {
                    (RegionKind::Skipped, Counter::Zero, Counter::Zero, McdcParams::None, 0)
                }
                Region::Gap { count, .. } => {
                    (RegionKind::Gap, builder.counter(count), Counter::Zero, McdcParams::None, 0)
                }
                Region::Branch { true_count, false_count, .. } => (
                    RegionKind::Branch,
                    builder.counter(true_count),
                    builder.counter(false_count),
                    McdcParams::None,
                    0,
                ),
                Region::Decision { decision: index, .. } => {
                    let conditions = function.decisions.get(index as usize)?.conditions;
                    decision = Some(index as usize);
                    // The bitmap index is set once the decision's conditions are known.
                    let params = McdcParams::Decision { bitmap_idx: 0, num_conditions: conditions };
                    (RegionKind::McdcDecision, Counter::Zero, Counter::Zero, params, 0)
                }
                Region::McdcBranch {
                    true_count,
                    false_count,
                    cond_id,
                    false_next,
                    true_next,
                    ..
                } => (
                    RegionKind::McdcBranch,
                    builder.counter(true_count),
                    builder.counter(false_count),
                    McdcParams::Branch { id: cond_id as i32, false_next, true_next },
                    0,
                ),
            };
            let region =
                MappingRegion { kind, count, false_count, mcdc, file, expanded_file, start, end };
            mapped.push((region, decision));
        }
        let mut expressions = builder.expressions;

        // Number the counters: the record's own first, then the others by ID. The others are the
        // counters of a modifier's expansions, which its record sums.
        let mut foreign = std::collections::BTreeSet::new();
        let counters = mapped
            .iter()
            .flat_map(|(region, _)| [region.count, region.false_count])
            .chain(expressions.iter().flat_map(|e| [e.lhs, e.rhs]));
        for counter in counters {
            if let Counter::Counter(id) = counter
                && !(first..first.saturating_add(len)).contains(&id)
            {
                foreign.insert(id);
            }
        }
        let foreign = foreign.into_iter().collect::<Vec<_>>();
        let local = |counter: Counter| match counter {
            Counter::Counter(id) if (first..first.saturating_add(len)).contains(&id) => {
                Counter::Counter(id - first)
            }
            Counter::Counter(id) => {
                Counter::Counter(len + foreign.binary_search(&id).unwrap() as u32)
            }
            other => other,
        };
        for (region, _) in &mut mapped {
            region.count = local(region.count);
            region.false_count = local(region.false_count);
        }
        for expression in &mut expressions {
            expression.lhs = local(expression.lhs);
            expression.rhs = local(expression.rhs);
        }
        let counts = (first..first.saturating_add(len))
            .chain(foreign)
            .map(|counter| self.hits(counter))
            .collect();

        // Sort like LLVM's writer: by file, location, kind, and wider regions first.
        let kind_key = |kind: RegionKind| match kind {
            RegionKind::McdcDecision => 2 * RegionKind::Expansion as u32 - 1,
            kind => 2 * kind as u32,
        };
        mapped.sort_by(|(lhs, _), (rhs, _)| {
            lhs.file
                .cmp(&rhs.file)
                .then(lhs.start.cmp(&rhs.start))
                .then(kind_key(lhs.kind).cmp(&kind_key(rhs.kind)))
                .then(rhs.end.cmp(&lhs.end))
        });

        let bitmap = self.assign_bitmap(function, &mut mapped)?;
        Some(FunctionInput {
            name: function.name.clone(),
            hash: function.hash.max(1),
            filenames,
            expressions,
            regions: mapped.into_iter().map(|(region, _)| region).collect(),
            counts,
            bitmap,
        })
    }

    /// Lays out the MC/DC bitmap of a function: assigns each decision the bits of its test
    /// vectors, and returns the bitmap bytes with the executed test vectors set.
    ///
    /// Like LLVM, it pairs a decision region with the condition regions that follow it.
    fn assign_bitmap(
        &self,
        function: &FunctionCoverage,
        regions: &mut [(MappingRegion, Option<usize>)],
    ) -> Option<Vec<u8>> {
        let mut layout = Vec::new();
        // The decisions that wait for conditions: the region index, the `[false, true]` next
        // condition of each condition by ID, and how many conditions were found.
        let mut open = Vec::<(usize, Vec<[i32; 2]>, usize)>::new();
        for (index, (region, _)) in regions.iter().enumerate() {
            match (region.kind, region.mcdc) {
                (RegionKind::McdcDecision, McdcParams::Decision { num_conditions, .. }) => {
                    open.push((index, vec![[-1; 2]; num_conditions as usize], 0));
                }
                (RegionKind::McdcBranch, McdcParams::Branch { id, false_next, true_next }) => {
                    let (_, next_ids, found) = open.last_mut()?;
                    *next_ids.get_mut(usize::try_from(id).ok()?)? = [false_next, true_next];
                    *found += 1;
                    if *found == next_ids.len() {
                        let (decision, next_ids, _) = open.pop()?;
                        layout.push((decision, next_ids));
                    }
                }
                _ => {}
            }
        }
        if !open.is_empty() {
            return None;
        }

        let mut bits = 0usize;
        let mut executed = Vec::new();
        for (index, next_ids) in layout {
            let (region, decision) = &mut regions[index];
            let McdcParams::Decision { num_conditions, .. } = region.mcdc else { return None };
            let decision = function.decisions.get((*decision)?)?;
            let vectors = decision_vectors(&next_ids)?;
            let start = bits;
            bits += vectors.len();
            // The index is that of the bit after the decision's test vectors.
            region.mcdc = McdcParams::Decision { bitmap_idx: bits as u32, num_conditions };

            // The map's test vector index is the one the probes add up. LLVM numbers the vectors
            // by the condition graph, so find each of the map's vectors among LLVM's.
            let positions = decision
                .vectors
                .iter()
                .map(|vector| vectors.iter().position(|llvm| llvm == vector))
                .collect::<Option<Vec<_>>>()
                .filter(|positions| positions.len() == vectors.len());
            if positions.is_none() {
                tracing::warn!(
                    function = function.name,
                    decision = ?decision.runtime_ids().collect::<Vec<_>>(),
                    "the MC/DC test vectors of the map are not those of its condition graph"
                );
            }
            // The vectors that any of the decision's runtime IDs executed.
            let vectors_run = decision
                .runtime_ids()
                .flat_map(|id| self.mcdc.get(&id).into_iter().flatten().copied());
            for vector in vectors_run {
                let position = match &positions {
                    Some(positions) => positions.get(vector as usize).copied(),
                    None => Some(vector as usize),
                };
                if let Some(position) = position.filter(|&position| position < vectors.len()) {
                    executed.push(start + position);
                }
            }
        }
        let mut bitmap = vec![0u8; bits.div_ceil(8)];
        for bit in executed {
            bitmap[bit / 8] |= 1 << (bit % 8);
        }
        Some(bitmap)
    }
}

/// Builds the LLVM counters and expressions of a function record from its tagged counters, with
/// counter IDs as in the map.
///
/// LLVM gives an expression its kind in the references to it, so an expression that the record
/// refers to as both a sum and a difference becomes two expressions.
struct ExpressionBuilder<'a> {
    function: &'a FunctionCoverage,
    expressions: Vec<Expression>,
    /// The expression of an input expression and kind, by `(index, subtract)`.
    built: FxHashMap<(u32, bool), u32>,
}

impl ExpressionBuilder<'_> {
    fn counter(&mut self, tagged: u32) -> Counter {
        match TaggedCounter::decode(tagged) {
            TaggedCounter::Zero => Counter::Zero,
            TaggedCounter::Counter(id) => Counter::Counter(id),
            TaggedCounter::Subtract(id) => self.expression(id, true),
            TaggedCounter::Add(id) => self.expression(id, false),
        }
    }

    fn expression(&mut self, id: u32, subtract: bool) -> Counter {
        if let Some(&index) = self.built.get(&(id, subtract)) {
            return Counter::Expression(index);
        }
        let Some(&[lhs, rhs]) = self.function.expressions.get(id as usize) else {
            return Counter::Zero;
        };
        let (lhs, rhs) = (self.counter(lhs), self.counter(rhs));
        let index = self.expressions.len() as u32;
        self.expressions.push(Expression { subtract, lhs, rhs });
        self.built.insert((id, subtract), index);
        Counter::Expression(index)
    }
}

/// Splits a PGO name `path:Contract.function(args)` into the contract and the function name.
fn parse_name(name: &str) -> (Option<&str>, &str) {
    let rest = name.split_once(':').map_or(name, |(_, rest)| rest);
    let head = &rest[..rest.find('(').unwrap_or(rest.len())];
    match head.rsplit_once('.') {
        Some((contract, function)) => (Some(contract), function),
        None => (None, head),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoverageReport, llvm::tests::build};
    use serde_json::json;

    fn item_summary(report: &CoverageReport) -> Vec<String> {
        let (_, items) = report.items_by_file().next().unwrap();
        items
            .iter()
            .map(|item| {
                let kind = match &item.kind {
                    CoverageItemKind::Line => "line".to_string(),
                    CoverageItemKind::Statement => "statement".to_string(),
                    CoverageItemKind::Function { name } => format!("function {name}"),
                    CoverageItemKind::Branch { branch_id, path_id, .. } => {
                        format!("branch {branch_id}.{path_id}")
                    }
                };
                format!("{kind} {:?} {}", item.loc.lines, item.hits)
            })
            .collect()
    }

    #[test]
    fn items_follow_the_coverage_map_and_llvm_line_rules() {
        let mut report = CoverageReport::default();
        report.add_source("b".into(), 0, "src/C.sol".into());
        report.add_instrumented_build("b".into(), build());
        report.finalize_instrumented();
        let summary = item_summary(&report);
        // The functions and statements of the file's own regions, and both sides of the two
        // conditions of the decision. The modifier's counts are the sum of its expansions'.
        for expected in [
            "function f 3..11 3",
            "function m 11..12 3",
            "function g 12..13 2",
            "function g2 13..14 1",
            "function k 14..15 1",
            "function h 15..16 0",
            "branch 0.0 4..5 2",
            "branch 0.1 4..5 1",
            "branch 1.0 4..5 1",
            "branch 1.1 4..5 1",
        ] {
            assert!(summary.iter().any(|item| item == expected), "{expected} in {summary:#?}");
        }
        // `llvm-cov` maps lines 3 to 15. The modifier's line counts its own record and the
        // expansions that repeat it.
        let lines = summary.iter().filter(|item| item.starts_with("line")).collect::<Vec<_>>();
        let expected = [3, 3, 1, 2, 2, 2, 3, 3, 6, 2, 1, 1, 0];
        assert_eq!(lines.len(), expected.len());
        for (index, hits) in expected.iter().enumerate() {
            let line = index + 3;
            assert_eq!(*lines[index], format!("line {line}..{} {hits}", line + 1));
        }
    }

    #[test]
    fn dropped_counters_take_the_hits_of_their_replacement() {
        let mut build = build();
        // Counter 20 stands in for the dropped counter 21 in one code object only.
        build.add_hits([(20, 4)].into_iter(), std::iter::empty(), &[[21, 20]]);
        build.add_hits([(21, 1)].into_iter(), std::iter::empty(), &[]);
        assert_eq!((build.hits(20), build.hits(21)), (4, 5));
        // A dropped counter whose replacement did not run gets none, even if it counted.
        build.add_hits([(22, 9)].into_iter(), std::iter::empty(), &[[22, 23]]);
        assert_eq!(build.hits(22), 0);
    }

    #[test]
    fn an_expression_used_as_a_sum_and_a_difference_becomes_two() {
        let source = InstrumentedSource::new(
            "src/A.sol".into(),
            "contract A { function f() public { a(); b(); } }".into(),
            true,
            serde_json::from_value(json!({
                "version": 2,
                "functions": [{
                    "name": "src/A.sol:A.f()",
                    "hash": "0x1",
                    // Expression 0 is counter 1 + counter 2. The regions refer to it as a sum and as
                    // a difference.
                    "counters": [1, 2],
                    "files": [0],
                    "expressions": [[5, 9]],
                    "regions": [
                        [0, 0, 13, 33, 3],
                        [0, 0, 36, 4, 2],
                        [0, 0, 41, 4, 3],
                    ],
                }]
            }))
            .unwrap(),
        );
        let input = InstrumentedBuild::new(BTreeMap::from([(0, source)])).function_inputs();
        let expressions = &input[0].expressions;
        assert_eq!(expressions.len(), 2);
        assert!(expressions[0].subtract != expressions[1].subtract);
        assert_eq!(
            (expressions[0].lhs, expressions[0].rhs),
            (expressions[1].lhs, expressions[1].rhs)
        );
        assert_eq!(expressions[0].lhs, Counter::Counter(0));
        assert_eq!(expressions[0].rhs, Counter::Counter(1));
    }

    #[test]
    fn a_modifier_in_an_excluded_source_is_dropped_from_its_users() {
        let text = "contract A { modifier m() { x; _; } function f() m { y; } }";
        let span = |needle: &str| (text.find(needle).unwrap(), needle.len());
        let region = |kind: u32, file: u32, needle: &str, rest: &[u32]| {
            let (start, len) = span(needle);
            let mut region = vec![kind, file, start as u32, len as u32];
            region.extend(rest);
            json!(region)
        };
        let map = |files: serde_json::Value, regions: Vec<serde_json::Value>| {
            serde_json::from_value::<SourceCoverage>(json!({
                "version": 2,
                "functions": [{
                    "name": "src/A.sol:A.f()", "hash": "0x1", "counters": [1, 2],
                    "files": files, "regions": regions,
                }]
            }))
            .unwrap()
        };
        let users = map(
            json!([0, 1]),
            vec![
                region(0, 0, "function f() m { y; }", &[5]),
                region(1, 0, "m {", &[0, 1]),
                region(0, 1, "modifier m() { x; _; }", &[9]),
                region(0, 0, "y;", &[9]),
            ],
        );
        let sources = |reportable| {
            BTreeMap::from([
                (0, InstrumentedSource::new("src/A.sol".into(), text.into(), true, users.clone())),
                (
                    1,
                    InstrumentedSource::new(
                        "lib/M.sol".into(),
                        text.into(),
                        reportable,
                        SourceCoverage::default(),
                    ),
                ),
            ])
        };
        let inputs = InstrumentedBuild::new(sources(true)).function_inputs();
        assert_eq!(inputs[0].filenames, ["src/A.sol", "lib/M.sol"]);
        assert_eq!(inputs[0].regions.len(), 4);
        // The library is not in the report, so the modifier's regions are gone and the user's
        // function keeps its own.
        let inputs = InstrumentedBuild::new(sources(false)).function_inputs();
        assert_eq!(inputs[0].filenames, ["src/A.sol"]);
        assert_eq!(
            inputs[0].regions.iter().map(|region| region.kind).collect::<Vec<_>>(),
            [RegionKind::Code, RegionKind::Code]
        );
    }

    #[test]
    fn mcdc_vectors_land_on_llvms_indices_whatever_order_the_map_numbers_them() {
        let text = "contract A { function f() public { if (x && y) { c(); } } }";
        let region = |kind: u32, needle: &str, rest: &[i64]| {
            let start = text.find(needle).unwrap();
            let mut region = vec![i64::from(kind), 0, start as i64, needle.len() as i64];
            region.extend(rest);
            json!(region)
        };
        let source = |vectors: serde_json::Value| {
            InstrumentedSource::new(
                "src/A.sol".into(),
                text.into(),
                true,
                serde_json::from_value(json!({
                    "version": 2,
                    "functions": [{
                        "name": "src/A.sol:A.f()", "hash": "0x1", "counters": [1, 1], "files": [0],
                        "regions": [
                            region(0, "function f() public { if (x && y) { c(); } }", &[5]),
                            region(5, "x && y", &[0]),
                            region(6, "x", &[5, 0, 0, -1, 1]),
                            region(6, "y", &[5, 0, 1, -1, -1]),
                        ],
                        "decisions": [{"id": 3, "conditions": 2, "vectors": vectors}],
                    }]
                }))
                .unwrap(),
            )
        };
        let bitmap = |vectors: serde_json::Value, executed: [(u32, u32); 2]| {
            let mut build = InstrumentedBuild::new(BTreeMap::from([(0, source(vectors))]));
            build.add_hits(std::iter::empty(), executed.into_iter(), &[]);
            build.function_inputs().remove(0).bitmap
        };
        // In LLVM's order, `F-F`, `TFF`, and `TTT` are 0, 1, and 2.
        assert_eq!(bitmap(json!(["F-F", "TFF", "TTT"]), [(3, 1), (3, 2)]), [0b110]);
        // The map numbers them the other way round: indices 0 and 1 are `TTT` and `TFF`.
        assert_eq!(bitmap(json!(["TTT", "TFF", "F-F"]), [(3, 0), (3, 1)]), [0b110]);
        // A map that does not describe the condition graph keeps its own indices.
        assert_eq!(bitmap(json!(["TTT"]), [(3, 0), (3, 1)]), [0b011]);
    }
}
