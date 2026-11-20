//! LLVM's coverage-mapping model: function records with their counted regions, MC/DC records,
//! and the lookups `llvm-cov` builds its reports from.
//!
//! This is a port of the parts of `CoverageMapping.cpp` that read a function's regions and
//! counters and turn them into counted regions, so that the reports match what `llvm-cov` prints
//! for the same coverage mapping and profile.

use super::segments::{CoverageData, build_segments};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A 1-based line and column.
pub type LineCol = (u32, u32);

/// The kind of a mapping region, numbered like LLVM's `CounterMappingRegion::RegionKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegionKind {
    /// Code with an execution count.
    Code = 0,
    /// The expansion of a virtual file.
    Expansion = 1,
    /// Code that was skipped.
    Skipped = 2,
    /// A gap between code regions.
    Gap = 3,
    /// A condition with a true and a false count.
    Branch = 4,
    /// An MC/DC decision.
    McdcDecision = 5,
    /// A condition of an MC/DC decision.
    McdcBranch = 6,
}

impl RegionKind {
    /// Returns whether the region has a true and a false count.
    pub const fn is_branch(self) -> bool {
        matches!(self, Self::Branch | Self::McdcBranch)
    }
}

/// A counter reference, like LLVM's `Counter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Counter {
    /// The constant zero.
    #[default]
    Zero,
    /// The profile counter with this index.
    Counter(u32),
    /// The expression with this index.
    Expression(u32),
}

impl Counter {
    /// Returns whether the counter is the constant zero.
    pub const fn is_zero(self) -> bool {
        matches!(self, Self::Zero)
    }
}

/// A counter expression, like LLVM's `CounterExpression`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Expression {
    /// Whether the expression subtracts its right operand. It adds otherwise.
    pub subtract: bool,
    /// The left operand.
    pub lhs: Counter,
    /// The right operand.
    pub rhs: Counter,
}

/// The MC/DC parameters of a region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McdcParams {
    /// A region without MC/DC parameters.
    #[default]
    None,
    /// An MC/DC decision.
    Decision {
        /// The index in the function's bitmap, in bits, just past the decision's test vectors.
        bitmap_idx: u32,
        /// The number of conditions.
        num_conditions: u32,
    },
    /// A condition of an MC/DC decision.
    Branch {
        /// The condition ID.
        id: i32,
        /// The condition evaluated next when this one is false, or `-1` at the end of the
        /// decision.
        false_next: i32,
        /// The condition evaluated next when this one is true, or `-1` at the end of the decision.
        true_next: i32,
    },
}

/// A region of a function record, like LLVM's `CounterMappingRegion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingRegion {
    /// The kind of region.
    pub kind: RegionKind,
    /// The count, or the true count of a branch.
    pub count: Counter,
    /// The false count of a branch.
    pub false_count: Counter,
    /// The MC/DC parameters.
    pub mcdc: McdcParams,
    /// The index in the function's file table.
    pub file: u32,
    /// The expanded file of an expansion.
    pub expanded_file: u32,
    /// The start line and column.
    pub start: LineCol,
    /// The end line and column, exclusive.
    pub end: LineCol,
}

impl MappingRegion {
    /// The order of LLVM's writer: by file, location, kind, and wider regions first.
    pub fn writer_order(&self, other: &Self) -> std::cmp::Ordering {
        let kind_key = |kind: RegionKind| match kind {
            RegionKind::McdcDecision => 2 * RegionKind::Expansion as u32 - 1,
            kind => 2 * kind as u32,
        };
        (self.file, self.start, kind_key(self.kind))
            .cmp(&(other.file, other.start, kind_key(other.kind)))
            .then(other.end.cmp(&self.end))
    }
}

/// A function record as stored in the coverage mapping, with its profile data.
#[derive(Clone, Debug)]
pub struct FunctionInput {
    /// The PGO name of the function.
    pub name: String,
    /// The structural hash of the function.
    pub hash: u64,
    /// The names of the files of the function's file table.
    pub filenames: Vec<String>,
    /// The counter expressions.
    pub expressions: Vec<Expression>,
    /// The regions, sorted by file and location like LLVM's writer does.
    pub regions: Vec<MappingRegion>,
    /// The profile counters, by index.
    pub counts: Vec<u64>,
    /// The profile's MC/DC bitmap bytes.
    pub bitmap: Vec<u8>,
    /// Whether the record has no counters of its own, such as a modifier's, so that its counts are
    /// the sums of the counters of the records that expand it.
    pub summed: bool,
}

/// A mapping region with its execution counts.
#[derive(Clone, Copy, Debug)]
pub struct CountedRegion {
    /// The region.
    pub region: MappingRegion,
    /// The execution count, or the true count of a branch.
    pub count: u64,
    /// The false count of a branch.
    pub false_count: u64,
    /// Whether the true side of a branch is constant folded.
    pub true_folded: bool,
    /// Whether the false side of a branch is constant folded.
    pub false_folded: bool,
}

impl CountedRegion {
    /// The start of the region.
    pub const fn start(&self) -> LineCol {
        self.region.start
    }

    /// The end of the region, exclusive.
    pub const fn end(&self) -> LineCol {
        self.region.end
    }

    /// The index in the file table.
    pub const fn file(&self) -> u32 {
        self.region.file
    }
}

/// The state of a condition in a test vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CondState {
    /// The condition was not evaluated.
    DontCare,
    /// The condition was false.
    False,
    /// The condition was true.
    True,
}

/// A test vector: the state of each condition by condition ID, and the outcome.
type TestVector = Vec<CondState>;

/// An MC/DC record: a decision with its executed and not executed test vectors.
#[derive(Clone, Debug)]
pub struct McdcRecord {
    /// The decision region.
    pub region: MappingRegion,
    /// The executed test vectors with their outcome.
    pub executed: Vec<(Vec<CondState>, CondState)>,
    /// The not executed test vectors with their outcome.
    pub not_executed: Vec<(Vec<CondState>, CondState)>,
    /// Whether each condition's false and true side is constant folded, by ordinal.
    folded: [Vec<bool>; 2],
    /// The condition ID of each condition, by ordinal.
    pos_to_id: Vec<usize>,
    /// The condition IDs with an independence pair.
    independence_pairs: HashSet<usize>,
}

impl McdcRecord {
    /// The number of conditions.
    pub const fn num_conditions(&self) -> usize {
        self.pos_to_id.len()
    }

    /// Returns whether the condition at ordinal `condition` is constant folded.
    pub fn is_cond_folded(&self, condition: usize) -> bool {
        self.folded[0][condition] || self.folded[1][condition]
    }

    /// Returns whether an independence pair covers the condition at ordinal `condition`.
    pub fn is_independence_pair_covered(&self, condition: usize) -> bool {
        self.independence_pairs.contains(&self.pos_to_id[condition])
    }

    /// The state of the condition at ordinal `condition` in a test vector.
    pub fn condition(&self, vector: &[CondState], condition: usize) -> CondState {
        vector[self.pos_to_id[condition]]
    }

    /// The number of executed test vectors with a true and with a false outcome.
    pub fn decisions(&self) -> (usize, usize) {
        let true_decisions =
            self.executed.iter().filter(|(_, result)| *result == CondState::True).count();
        (true_decisions, self.executed.len() - true_decisions)
    }
}

/// The coverage of one function, like LLVM's `FunctionRecord`.
#[derive(Clone, Debug)]
pub struct FunctionRecord {
    /// The function name without its file prefix.
    pub name: String,
    /// The names of the files in the function's file table.
    pub filenames: Vec<String>,
    /// The non-branch regions, in region order.
    pub regions: Vec<CountedRegion>,
    /// The branch regions, in region order.
    pub branch_regions: Vec<CountedRegion>,
    /// The MC/DC records.
    pub mcdc_records: Vec<McdcRecord>,
    /// The number of times the function was entered.
    pub execution_count: u64,
}

/// An expansion of a virtual file in a function.
#[derive(Clone, Copy, Debug)]
pub struct Expansion {
    /// The index of the function in the mapping.
    pub function: usize,
    /// The index of the expansion region in the function's `regions`.
    pub region: usize,
}

/// The loaded coverage mapping of all functions.
#[derive(Debug, Default)]
pub struct CoverageMapping {
    pub(super) functions: Vec<FunctionRecord>,
    by_file: HashMap<String, Vec<usize>>,
}

/// Functions that share a definition location.
#[derive(Clone, Debug)]
pub struct InstantiationGroup {
    /// The line of the definition.
    pub line: u32,
    /// The column of the definition.
    pub col: u32,
    /// The indices of the functions in the mapping.
    pub functions: Vec<usize>,
}

impl CoverageMapping {
    /// Loads the function records with their profile data, skipping a function whose counters
    /// `llvm-cov` could not evaluate.
    pub fn new(inputs: impl IntoIterator<Item = FunctionInput>) -> Self {
        let mut mapping = Self::default();
        let mut seen = HashSet::new();
        for input in inputs {
            let Some(function) = load_function(input) else { continue };
            // A function with the same file table and name is a duplicate.
            if !seen.insert((function.filenames.clone(), function.name.clone())) {
                continue;
            }
            let index = mapping.functions.len();
            for filename in &function.filenames {
                let indices = mapping.by_file.entry(filename.clone()).or_default();
                if indices.last() != Some(&index) {
                    indices.push(index);
                }
            }
            mapping.functions.push(function);
        }
        mapping
    }

    /// The loaded functions.
    pub fn functions(&self) -> &[FunctionRecord] {
        &self.functions
    }

    /// The unique source files of all functions, sorted.
    pub fn unique_source_files(&self) -> Vec<&str> {
        let mut files = self.by_file.keys().map(String::as_str).collect::<Vec<_>>();
        files.sort_unstable();
        files
    }

    /// The indices of the functions with `filename` in their file table.
    pub fn functions_in_file(&self, filename: &str) -> &[usize] {
        self.by_file.get(filename).map_or(&[], Vec::as_slice)
    }

    /// The coverage data of a file: its segments, branches, expansions, and MC/DC records.
    pub fn coverage_for_file(&self, filename: &str) -> CoverageData {
        let mut data = CoverageData::new(filename);
        let mut code_regions = Vec::new();
        for &index in self.functions_in_file(filename) {
            let function = &self.functions[index];
            let main_file = find_main_view_file_id(function)
                .filter(|&file| function.filenames[file as usize] == filename);
            let in_file = |file: u32| function.filenames[file as usize] == filename;
            for (region_index, region) in function.regions.iter().enumerate() {
                if !in_file(region.file()) {
                    continue;
                }
                code_regions.push(*region);
                if main_file.is_some_and(|file| is_expansion(region, file)) {
                    data.expansions.push(Expansion { function: index, region: region_index });
                }
            }
            data.branches
                .extend(function.branch_regions.iter().filter(|r| in_file(r.file())).copied());
            data.mcdc_records
                .extend(function.mcdc_records.iter().filter(|r| in_file(r.region.file)).cloned());
        }
        data.segments = build_segments(&mut code_regions);
        data
    }

    /// The coverage data of a function's main file.
    pub fn coverage_for_function(&self, index: usize) -> CoverageData {
        let function = &self.functions[index];
        let Some(main_file) = find_main_view_file_id(function) else {
            return CoverageData::default();
        };
        let mut data = CoverageData::new(&function.filenames[main_file as usize]);
        let mut code_regions = Vec::new();
        for (region_index, region) in function.regions.iter().enumerate() {
            if region.file() != main_file {
                continue;
            }
            code_regions.push(*region);
            if is_expansion(region, main_file) {
                data.expansions.push(Expansion { function: index, region: region_index });
            }
        }
        data.branches
            .extend(function.branch_regions.iter().filter(|r| r.file() == main_file).copied());
        data.mcdc_records
            .extend(function.mcdc_records.iter().filter(|r| r.region.file == main_file).cloned());
        data.segments = build_segments(&mut code_regions);
        data
    }

    /// The coverage data of an expansion's virtual file.
    pub fn coverage_for_expansion(&self, expansion: Expansion) -> CoverageData {
        let function = &self.functions[expansion.function];
        let file = function.regions[expansion.region].region.expanded_file;
        let mut data = CoverageData::new(&function.filenames[file as usize]);
        let mut code_regions = Vec::new();
        for (region_index, region) in function.regions.iter().enumerate() {
            if region.file() != file {
                continue;
            }
            code_regions.push(*region);
            if is_expansion(region, file) {
                data.expansions
                    .push(Expansion { function: expansion.function, region: region_index });
            }
        }
        data.branches.extend(function.branch_regions.iter().filter(|r| r.file() == file).copied());
        data.segments = build_segments(&mut code_regions);
        data
    }

    /// The functions defined in `filename`, grouped by definition location in source order.
    pub fn instantiation_groups(&self, filename: &str) -> Vec<InstantiationGroup> {
        let mut groups = BTreeMap::<LineCol, Vec<usize>>::new();
        for &index in self.functions_in_file(filename) {
            let function = &self.functions[index];
            let Some(main_file) = find_main_view_file_id(function)
                .filter(|&file| function.filenames[file as usize] == filename)
            else {
                continue;
            };
            let Some(region) = function.regions.iter().find(|r| r.file() == main_file) else {
                continue;
            };
            groups.entry(region.start()).or_default().push(index);
        }
        groups
            .into_iter()
            .map(|((line, col), functions)| InstantiationGroup { line, col, functions })
            .collect()
    }
}

impl CoverageMapping {
    /// The expansion's region.
    pub fn expansion_region(&self, expansion: Expansion) -> &CountedRegion {
        &self.functions[expansion.function].regions[expansion.region]
    }

    /// The function the expansion is in.
    pub fn expansion_function(&self, expansion: Expansion) -> &FunctionRecord {
        &self.functions[expansion.function]
    }
}

/// The first file of the function's file table that no expansion expands: the file the function
/// is defined in.
fn find_main_view_file_id(function: &FunctionRecord) -> Option<u32> {
    if function.regions.is_empty() {
        return None;
    }
    let mut is_expanded = vec![false; function.filenames.len()];
    for region in &function.regions {
        if region.region.kind == RegionKind::Expansion {
            is_expanded[region.region.expanded_file as usize] = true;
        }
    }
    is_expanded.iter().position(|&expanded| !expanded).map(|file| file as u32)
}

fn is_expansion(region: &CountedRegion, file: u32) -> bool {
    region.region.kind == RegionKind::Expansion && region.file() == file
}

/// Merges two records of the same function without counters of its own, which were built with
/// different expansions and so differ in their counters, hash, and the regions they take from the
/// expansions. A region that both have takes the sum of its counts, and a code, gap, skipped, or
/// expansion region that only one has stays, as do the files that only one has after those of
/// the other. Returns `None` if the other regions or files differ.
pub fn merge_summed(a: &FunctionInput, b: &FunctionInput) -> Option<FunctionInput> {
    let key = |r: &MappingRegion| (r.kind, r.file, r.expanded_file, r.start, r.end, r.mcdc);
    let from_expansions = |r: &MappingRegion| {
        matches!(
            r.kind,
            RegionKind::Code | RegionKind::Gap | RegionKind::Skipped | RegionKind::Expansion
        )
    };
    let filenames = if a.filenames.len() < b.filenames.len() { &b.filenames } else { &a.filenames };
    if !filenames.starts_with(&a.filenames)
        || !filenames.starts_with(&b.filenames)
        || a.bitmap.len() != b.bitmap.len()
    {
        return None;
    }
    let mut pairs = vec![None; a.regions.len()];
    let mut paired = vec![false; b.regions.len()];
    for (index, region) in a.regions.iter().enumerate() {
        let other = (0..b.regions.len()).find(|&j| !paired[j] && key(region) == key(&b.regions[j]));
        if let Some(other) = other {
            pairs[index] = Some(other);
            paired[other] = true;
        }
    }
    let lone = a.regions.iter().zip(&pairs).filter(|(_, pair)| pair.is_none());
    let lone_b = b.regions.iter().zip(&paired).filter(|(_, paired)| !**paired);
    if lone.into_iter().any(|(region, _)| !from_expansions(region))
        || lone_b.clone().any(|(region, _)| !from_expansions(region))
    {
        return None;
    }

    let mut counts = Vec::new();
    let mut sum = |x: Option<(Counter, &FunctionInput)>, y: Option<(Counter, &FunctionInput)>| {
        if x.is_none_or(|(counter, _)| counter.is_zero())
            && y.is_none_or(|(counter, _)| counter.is_zero())
        {
            return Counter::Zero;
        }
        let value = |side: Option<(Counter, &FunctionInput)>| {
            side.map_or(0, |(counter, input)| {
                evaluate(counter, &input.expressions, &input.counts).unwrap_or(0).max(0) as u64
            })
        };
        counts.push(value(x).saturating_add(value(y)));
        Counter::Counter(counts.len() as u32 - 1)
    };
    let mut regions = Vec::with_capacity(a.regions.len());
    for (region, pair) in a.regions.iter().zip(&pairs) {
        let other = pair.map(|index| &b.regions[index]);
        let mut merged = *region;
        merged.count = sum(Some((region.count, a)), other.map(|o| (o.count, b)));
        if region.kind.is_branch() {
            merged.false_count =
                sum(Some((region.false_count, a)), other.map(|o| (o.false_count, b)));
        }
        regions.push(merged);
    }
    for (region, _) in lone_b {
        let mut merged = *region;
        merged.count = sum(None, Some((region.count, b)));
        regions.push(merged);
    }
    regions.sort_by(MappingRegion::writer_order);
    Some(FunctionInput {
        name: a.name.clone(),
        hash: a.hash,
        filenames: filenames.clone(),
        expressions: Vec::new(),
        regions,
        counts,
        bitmap: a.bitmap.iter().zip(&b.bitmap).map(|(x, y)| x | y).collect(),
        summed: true,
    })
}

/// Removes the file prefix from the PGO name, like `getFuncNameWithoutPrefix`.
fn name_without_prefix<'a>(name: &'a str, filename: &str) -> &'a str {
    if filename.is_empty() {
        return name;
    }
    name.strip_prefix(filename).map_or(name, |rest| rest.get(1..).unwrap_or(""))
}

/// Evaluates a counter like LLVM's `CounterMappingContext::evaluate`.
fn evaluate(counter: Counter, expressions: &[Expression], counts: &[u64]) -> Option<i64> {
    enum Frame {
        Visit(Counter),
        Combine(Expression),
    }
    let mut stack = vec![Frame::Visit(counter)];
    let mut values = Vec::new();
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Visit(Counter::Zero) => values.push(0),
            Frame::Visit(Counter::Counter(id)) => values.push(*counts.get(id as usize)? as i64),
            Frame::Visit(Counter::Expression(id)) => {
                let expression = *expressions.get(id as usize)?;
                stack.push(Frame::Combine(expression));
                stack.push(Frame::Visit(expression.rhs));
                stack.push(Frame::Visit(expression.lhs));
            }
            Frame::Combine(expression) => {
                let rhs = values.pop()?;
                let lhs = values.pop()?;
                values.push(if expression.subtract {
                    lhs.wrapping_sub(rhs)
                } else {
                    lhs.wrapping_add(rhs)
                });
            }
        }
    }
    values.pop()
}

/// The largest counter ID a counter refers to, like `CounterMappingContext::getMaxCounterID`.
fn max_counter_id(counter: Counter, expressions: &[Expression]) -> u32 {
    let mut max = 0;
    let mut stack = vec![counter];
    while let Some(counter) = stack.pop() {
        match counter {
            Counter::Zero => {}
            Counter::Counter(id) => max = max.max(id),
            Counter::Expression(id) => {
                if let Some(expression) = expressions.get(id as usize) {
                    stack.extend([expression.lhs, expression.rhs]);
                }
            }
        }
    }
    max
}

/// Loads a function record: evaluates the counters of its regions and its MC/DC records.
fn load_function(mut input: FunctionInput) -> Option<FunctionRecord> {
    if input.name.is_empty() || input.regions.is_empty() {
        return None;
    }
    // The counts of the expansion regions are the counts of the first region of the expanded file.
    propagate_expansion_counts(&mut input.regions);

    let mut counts = input.counts.clone();
    let max_id = input
        .regions
        .iter()
        .flat_map(|region| {
            let false_count = region.kind.is_branch().then_some(region.false_count);
            std::iter::once(region.count).chain(false_count)
        })
        .map(|counter| max_counter_id(counter, &input.expressions))
        .max()
        .unwrap_or(0);
    if counts.len() <= max_id as usize {
        counts.resize(max_id as usize + 1, 0);
    }

    // A zero region of a function that is used elsewhere is a placeholder.
    if input.regions.len() == 1 && input.regions[0].count.is_zero() && counts[0] > 0 {
        return None;
    }

    let filename0 = input.filenames.first().map_or("", String::as_str);
    let name = name_without_prefix(&input.name, filename0).to_string();
    let mut function = FunctionRecord {
        name,
        filenames: input.filenames.clone(),
        regions: Vec::new(),
        branch_regions: Vec::new(),
        mcdc_records: Vec::new(),
        execution_count: 0,
    };
    let emitter = RegionEmitter {
        input: &input,
        counts: &counts,
        values: HashMap::new(),
        decisions: Vec::new(),
    };
    emitter.emit(&mut function)?;
    Some(function)
}

/// Sets each expansion region's count to the count of the first region of its expanded file.
fn propagate_expansion_counts(regions: &mut [MappingRegion]) {
    let files = regions.iter().map(|r| r.file.max(r.expanded_file) as usize + 1).max().unwrap_or(0);
    let mut slots = vec![None; files];
    loop {
        for (index, region) in regions.iter().enumerate() {
            if region.kind == RegionKind::Expansion {
                slots[region.expanded_file as usize] = Some(index);
            }
        }
        let mut changed = false;
        for index in 0..regions.len() {
            if let Some(expansion) = slots[regions[index].file as usize].take() {
                let count = regions[index].count;
                if regions[expansion].count != count {
                    regions[expansion].count = count;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// A decision being collected, with the condition regions found so far.
struct DecisionRecord {
    region: MappingRegion,
    num_conditions: usize,
    branches: Vec<MappingRegion>,
}

/// Walks a function's regions along its expansions and emits counted regions, like LLVM's
/// `CountedRegionEmitter`.
struct RegionEmitter<'a> {
    input: &'a FunctionInput,
    counts: &'a [u64],
    values: HashMap<Counter, u64>,
    decisions: Vec<DecisionRecord>,
}

impl RegionEmitter<'_> {
    fn emit(mut self, function: &mut FunctionRecord) -> Option<()> {
        let regions = &self.input.regions;
        let files = self.input.filenames.len().max(
            regions.iter().map(|r| r.file.max(r.expanded_file) as usize + 1).max().unwrap_or(0),
        );
        // The end of each file's regions, and whether an expansion expands the file.
        let mut last_index = vec![0usize; files];
        let mut is_expanded = vec![false; files];
        for (index, region) in regions.iter().enumerate() {
            last_index[region.file as usize] = index + 1;
            if region.kind == RegionKind::Expansion {
                is_expanded[region.expanded_file as usize] = true;
            }
        }
        for (file, expanded) in is_expanded.iter().enumerate() {
            if !expanded {
                self.walk(file, &last_index, function)?;
            }
        }
        for region in regions {
            if region.kind == RegionKind::McdcDecision {
                continue;
            }
            let count = self.values[&region.count];
            let false_count = self.values[&region.false_count];
            let mut counted = CountedRegion {
                region: *region,
                count,
                false_count,
                true_folded: false,
                false_folded: false,
            };
            if region.kind.is_branch() {
                counted.true_folded = region.count.is_zero();
                counted.false_folded = region.false_count.is_zero();
                function.branch_regions.push(counted);
            } else {
                if function.regions.is_empty() {
                    function.execution_count = count;
                }
                function.regions.push(counted);
            }
        }
        Some(())
    }

    fn evaluate_and_cache(&mut self, counter: Counter) -> Option<()> {
        if self.values.contains_key(&counter) {
            return Some(());
        }
        let value = evaluate(counter, &self.input.expressions, self.counts)?;
        self.values.insert(counter, value as u64);
        Some(())
    }

    fn walk(
        &mut self,
        file: usize,
        last_index: &[usize],
        function: &mut FunctionRecord,
    ) -> Option<()> {
        let begin = if file == 0 { 0 } else { last_index[file - 1] };
        let end = last_index[file];
        for index in begin..end {
            let region = self.input.regions[index];
            if region.file as usize != file {
                break;
            }
            if region.kind == RegionKind::Expansion {
                self.walk(region.expanded_file as usize, last_index, function)?;
            }
            self.evaluate_and_cache(region.count)?;
            match (region.kind, region.mcdc) {
                (RegionKind::McdcDecision, McdcParams::Decision { num_conditions, .. }) => {
                    self.decisions.push(DecisionRecord {
                        region,
                        num_conditions: num_conditions as usize,
                        branches: Vec::new(),
                    });
                }
                (RegionKind::McdcBranch, _) => {
                    let decision = self.decisions.last_mut()?;
                    decision.branches.push(region);
                    if decision.branches.len() == decision.num_conditions {
                        let decision = self.decisions.pop()?;
                        function.mcdc_records.push(self.mcdc_record(&decision)?);
                    }
                }
                _ => {}
            }
            self.evaluate_and_cache(region.false_count)?;
        }
        Some(())
    }

    fn mcdc_record(&self, decision: &DecisionRecord) -> Option<McdcRecord> {
        let McdcParams::Decision { bitmap_idx, .. } = decision.region.mcdc else { return None };
        let n = decision.num_conditions;
        let mut next_ids = vec![[-1i32; 2]; n];
        let mut seen = vec![false; n];
        for branch in &decision.branches {
            let McdcParams::Branch { id, false_next, true_next } = branch.mcdc else { return None };
            let slot = usize::try_from(id).ok().filter(|&id| id < n && !seen[id])?;
            seen[slot] = true;
            next_ids[slot] = [false_next, true_next];
        }
        let builder = TvIdxBuilder::new(&next_ids)?;

        let mut pos_to_id = Vec::with_capacity(n);
        let mut folded = [vec![false; n], vec![false; n]];
        for (position, branch) in decision.branches.iter().enumerate() {
            let McdcParams::Branch { id, .. } = branch.mcdc else { return None };
            pos_to_id.push(id as usize);
            folded[0][position] = branch.false_count.is_zero();
            folded[1][position] = branch.count.is_zero();
        }

        // The bitmap of the executed test vectors.
        let bit = |index: usize| {
            self.input.bitmap.get(index / 8).is_some_and(|byte| byte >> (index % 8) & 1 == 1)
        };
        let start = (bitmap_idx as usize).checked_sub(builder.num_test_vectors)?;
        let mut collector = VectorCollector {
            next_ids: &next_ids,
            builder: &builder,
            start,
            bit: &bit,
            exec: Vec::new(),
            not_exec: Vec::new(),
        };
        let mut vector = vec![CondState::DontCare; n];
        collector.build(&mut vector, 0, 0);
        let VectorCollector { mut exec, mut not_exec, .. } = collector;
        exec.sort_by_key(|&(cond, index, _, _)| (cond, index));
        not_exec.sort_by_key(|&(cond, index, _, _)| (cond, index));
        let executed =
            exec.into_iter().map(|(_, _, vector, result)| (vector, result)).collect::<Vec<_>>();
        let not_executed =
            not_exec.into_iter().map(|(_, _, vector, result)| (vector, result)).collect();

        let independence_pairs = independence_pairs(&executed);
        Some(McdcRecord {
            region: decision.region,
            executed,
            not_executed,
            folded,
            pos_to_id,
            independence_pairs,
        })
    }
}

/// Finds a condition's independence pair: a true and a false test vector where only that
/// condition differs and the outcome differs.
fn independence_pairs(executed: &[(TestVector, CondState)]) -> HashSet<usize> {
    let mut pairs = HashSet::new();
    let true_start = executed
        .iter()
        .position(|(_, result)| *result == CondState::True)
        .unwrap_or(executed.len());
    for (a, _) in &executed[true_start..] {
        for (b, _) in &executed[..true_start] {
            // The conditions evaluated in both vectors that differ.
            let mut differences = a.iter().zip(b).enumerate().filter(|(_, (a, b))| {
                **a != CondState::DontCare && **b != CondState::DontCare && a != b
            });
            if let (Some((condition, _)), None) = (differences.next(), differences.next()) {
                pairs.insert(condition);
            }
        }
    }
    pairs
}

/// Enumerates the test vectors of a decision from its condition graph.
struct VectorCollector<'a> {
    next_ids: &'a [[i32; 2]],
    builder: &'a TvIdxBuilder,
    /// The bit index of the decision's first test vector in the bitmap.
    start: usize,
    bit: &'a dyn Fn(usize) -> bool,
    /// Executed vectors with their condition state, index, vector, and result.
    exec: Vec<(CondState, usize, TestVector, CondState)>,
    not_exec: Vec<(CondState, usize, TestVector, CondState)>,
}

impl VectorCollector<'_> {
    /// Walks the decision diagram, trying false and true for each condition. A terminal node
    /// completes a vector.
    fn build(&mut self, vector: &mut TestVector, id: usize, tv_idx: usize) {
        for (side, state) in [CondState::False, CondState::True].into_iter().enumerate() {
            vector[id] = state;
            let next = self.next_ids[id][side];
            let next_tv_idx = tv_idx + self.builder.indices[id][side];
            if next >= 0 {
                self.build(vector, next as usize, next_tv_idx);
                continue;
            }
            let executed = (self.bit)(self.start + next_tv_idx);
            let entry = (state, next_tv_idx, vector.clone(), state);
            if executed { self.exec.push(entry) } else { self.not_exec.push(entry) }
        }
        vector[id] = CondState::DontCare;
    }
}

/// The test vectors of a decision in LLVM's test vector index order, as `T`, `F`, or `-` for each
/// condition ID followed by the outcome. `next_ids` is the `[false_next, true_next]` of each
/// condition, indexed by condition ID, and `-1` ends the decision.
pub fn decision_vectors(next_ids: &[[i32; 2]]) -> Option<Vec<String>> {
    let builder = TvIdxBuilder::new(next_ids)?;
    let all_executed = |_| true;
    let mut collector = VectorCollector {
        next_ids,
        builder: &builder,
        start: 0,
        bit: &all_executed,
        exec: Vec::new(),
        not_exec: Vec::new(),
    };
    collector.build(&mut vec![CondState::DontCare; next_ids.len()], 0, 0);
    let mut vectors = collector.exec;
    vectors.sort_by_key(|&(_, index, _, _)| index);
    let letter = |state: &CondState| match state {
        CondState::True => 'T',
        CondState::False => 'F',
        CondState::DontCare => '-',
    };
    Some(
        vectors
            .into_iter()
            .map(|(_, _, vector, result)| vector.iter().chain([&result]).map(letter).collect())
            .collect(),
    )
}

/// Assigns test vector indices over a decision's condition graph, like LLVM's `TVIdxBuilder`.
#[derive(Debug)]
pub struct TvIdxBuilder {
    /// The index added at each edge: `indices[condition][false or true]`.
    pub indices: Vec<[usize; 2]>,
    /// The number of test vectors.
    pub num_test_vectors: usize,
}

impl TvIdxBuilder {
    /// Builds the indices from the `[false_next, true_next]` of each condition, indexed by
    /// condition ID. `-1` ends the decision.
    pub fn new(next_ids: &[[i32; 2]]) -> Option<Self> {
        let n = next_ids.len();
        let mut in_count = vec![0usize; n];
        let mut width = vec![0usize; n];
        for ids in next_ids {
            for &next in ids {
                if next >= 0 {
                    *in_count.get_mut(next as usize)? += 1;
                }
            }
        }
        if n == 0 || in_count[0] != 0 {
            return None;
        }
        let mut indices = vec![[0usize; 2]; n];
        // (negated width, order, condition, side) of the edges that end the decision.
        let mut decisions = Vec::new();
        let mut queue = vec![0usize];
        width[0] = 1;
        let mut order = 0usize;
        while !queue.is_empty() {
            let id = queue.remove(0);
            for side in 0..2 {
                let next = next_ids[id][side];
                if next < 0 {
                    decisions.push((std::cmp::Reverse(width[id]), order, id, side));
                    order += 1;
                    continue;
                }
                let next = next as usize;
                if next == 0 {
                    return None;
                }
                indices[id][side] = width[next];
                width[next] += width[id];
                in_count[next] -= 1;
                if in_count[next] == 0 {
                    queue.push(next);
                }
            }
        }
        decisions.sort();
        let mut current = 0usize;
        for (std::cmp::Reverse(edge_width), _, id, side) in decisions {
            indices[id][side] = current;
            current += edge_width;
        }
        Some(Self { indices, num_test_vectors: current })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_indices_of_a_and_b_or_c() {
        // `a && b || c`: a(0) false -> c(2), true -> b(1); b false -> c, true -> end; c ends
        // either way. The five paths are F-F, F-T, TFF, TFT, and TT.
        let builder = TvIdxBuilder::new(&[[2, 1], [2, -1], [-1, -1]]).unwrap();
        assert_eq!(builder.num_test_vectors, 5);
        let index = |path: &[(usize, usize)]| {
            path.iter().map(|&(condition, side)| builder.indices[condition][side]).sum::<usize>()
        };
        let indices = [
            index(&[(0, 0), (2, 0)]),
            index(&[(0, 0), (2, 1)]),
            index(&[(0, 1), (1, 0), (2, 0)]),
            index(&[(0, 1), (1, 0), (2, 1)]),
            index(&[(0, 1), (1, 1)]),
        ];
        assert_eq!(indices, [0, 2, 1, 3, 4]);
    }

    #[test]
    fn decision_vectors_follow_the_test_vector_indices() {
        // `a && b`: the first condition ends the decision when false.
        assert_eq!(decision_vectors(&[[-1, 1], [-1, -1]]).unwrap(), ["F-F", "TFF", "TTT"]);
        // `a || b`: the first condition ends the decision when true.
        assert_eq!(decision_vectors(&[[1, -1], [-1, -1]]).unwrap(), ["T-T", "FFF", "FTT"]);
    }

    #[test]
    fn a_decision_without_a_start_has_no_indices() {
        // Condition 1 is not reachable from the root, and a condition cannot point back at it.
        assert!(TvIdxBuilder::new(&[[-1, -1], [-1, -1]]).is_some_and(|b| b.num_test_vectors == 2));
        assert!(TvIdxBuilder::new(&[[0, -1]]).is_none());
    }
}
