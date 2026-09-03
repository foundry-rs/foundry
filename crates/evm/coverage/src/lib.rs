//! # foundry-evm-coverage
//!
//! EVM bytecode coverage analysis.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

use alloy_primitives::{
    Bytes,
    map::{
        B256HashMap, HashMap,
        rustc_hash::{FxHashMap, FxHashSet},
    },
};
use analysis::SourceAnalysis;
use eyre::Result;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use semver::Version;
use std::{
    collections::BTreeMap,
    fmt,
    num::NonZeroU32,
    ops::{Deref, DerefMut, Range},
    path::{Path, PathBuf},
    sync::Arc,
};

pub mod analysis;
pub mod anchors;

mod inspector;
pub use inspector::LineCoverageCollector;

/// A coverage report.
///
/// A coverage report contains coverage items and opcodes corresponding to those items (called
/// "anchors"). A single coverage item may be referred to by multiple anchors.
#[derive(Clone, Debug, Default)]
pub struct CoverageReport {
    /// A map of compiler build IDs and source IDs to source paths.
    pub source_paths: HashMap<String, HashMap<usize, PathBuf>>,
    /// A map of compiler build IDs and source paths to source IDs.
    pub source_paths_to_ids: HashMap<String, HashMap<PathBuf, usize>>,
    /// All coverage items for the codebase, keyed by the compiler build ID.
    pub analyses: HashMap<String, SourceAnalysis>,
    /// All item anchors for the codebase, keyed by their contract ID.
    ///
    /// `(id, (creation, runtime))`
    pub anchors: HashMap<ContractId, (Vec<ItemAnchor>, Vec<ItemAnchor>)>,
    /// Execution-based anchors for coverage items without source-mapped bytecode.
    execution_anchors: HashMap<ContractId, ContractExecutionAnchors>,
    /// All the bytecode hits for the codebase.
    pub bytecode_hits: HashMap<ContractId, HitMap>,
    /// The bytecode -> source mappings.
    pub source_maps: HashMap<ContractId, (SourceMap, SourceMap)>,
}

impl CoverageReport {
    /// Add a source file path.
    pub fn add_source(&mut self, build_id: String, source_id: usize, path: PathBuf) {
        self.source_paths.entry(build_id.clone()).or_default().insert(source_id, path.clone());
        self.source_paths_to_ids.entry(build_id).or_default().insert(path, source_id);
    }

    /// Get the source ID for a specific source file path.
    pub fn get_source_id(&self, build_id: &str, path: &Path) -> Option<usize> {
        self.source_paths_to_ids.get(build_id)?.get(path).copied()
    }

    /// Get the source path for a source ID in a compiler build.
    pub fn get_source_path(&self, build_id: &str, source_id: usize) -> Option<&Path> {
        self.source_paths.get(build_id)?.get(&source_id).map(PathBuf::as_path)
    }

    /// Add the source maps.
    pub fn add_source_maps(
        &mut self,
        source_maps: impl IntoIterator<Item = (ContractId, (SourceMap, SourceMap))>,
    ) {
        self.source_maps.extend(source_maps);
    }

    /// Add a [`SourceAnalysis`] to this report.
    pub fn add_analysis(&mut self, build_id: String, analysis: SourceAnalysis) {
        self.analyses.insert(build_id, analysis);
    }

    /// Add anchors to this report.
    ///
    /// `(id, (creation, runtime))`
    pub fn add_anchors(
        &mut self,
        anchors: impl IntoIterator<Item = (ContractId, (Vec<ItemAnchor>, Vec<ItemAnchor>))>,
    ) {
        self.anchors.extend(anchors);
    }

    /// Adds execution-based anchors for a contract.
    pub fn add_execution_anchors(
        &mut self,
        contract_id: ContractId,
        anchors: Vec<ExecutionAnchor>,
        function_selectors: impl IntoIterator<Item = [u8; 4]>,
        has_receive: bool,
        fallback_payable: bool,
    ) {
        if anchors.is_empty() {
            return;
        }
        self.execution_anchors.insert(
            contract_id,
            ContractExecutionAnchors {
                anchors,
                function_selectors: function_selectors.into_iter().collect(),
                has_receive,
                fallback_payable,
            },
        );
    }

    /// Returns an iterator over coverage summaries by source file path.
    pub fn summary_by_file(&self) -> impl Iterator<Item = (&Path, CoverageSummary)> {
        self.items_by_file().map(|(path, items)| {
            let summary = CoverageSummary::from_items(&items);
            (path, summary)
        })
    }

    /// Returns coverage items by source file path, merging duplicate items from compiler builds.
    pub fn items_by_file(&self) -> impl Iterator<Item = (&Path, Vec<CoverageItem>)> {
        let mut by_file = BTreeMap::<&Path, BTreeMap<CoverageItemKey<'_>, CoverageItem>>::new();
        for (build_id, items) in &self.analyses {
            for item in items.all_items() {
                let Some(path) = self.get_source_path(build_id, item.loc.source_id) else {
                    continue;
                };
                by_file
                    .entry(path)
                    .or_default()
                    .entry(CoverageItemKey::new(item))
                    .and_modify(|merged| merged.hits = merged.hits.saturating_add(item.hits))
                    .or_insert_with(|| item.clone());
            }
        }
        by_file.into_iter().map(|(path, items)| (path, items.into_values().collect()))
    }

    /// Processes data from a [`HitMap`] and sets hit counts for coverage items in this coverage
    /// map.
    ///
    /// This function should only be called *after* all the relevant sources have been processed and
    /// added to the map (see [`add_source`](Self::add_source)).
    pub fn add_hit_map(
        &mut self,
        contract_id: &ContractId,
        hit_map: &HitMap,
        is_deployed_code: bool,
    ) -> Result<()> {
        // Add bytecode level hits.
        self.bytecode_hits
            .entry(contract_id.clone())
            .and_modify(|m| m.merge(hit_map))
            .or_insert_with(|| hit_map.clone());

        // Add source level hits.
        if let Some(anchors) = self.anchors.get(contract_id) {
            let anchors = if is_deployed_code { &anchors.1 } else { &anchors.0 };
            for anchor in anchors {
                if let Some(hits) = hit_map.get(anchor.instruction) {
                    self.analyses
                        .get_mut(&contract_id.build_id)
                        .and_then(|items| items.all_items_mut().get_mut(anchor.item_id as usize))
                        .expect("Anchor refers to non-existent coverage item")
                        .hits += hits.get();
                }
            }
        }
        if let Some(anchors) = self.execution_anchors.get(contract_id) {
            for anchor in &anchors.anchors {
                let hits = anchors.hits(hit_map, anchor.kind, is_deployed_code);
                self.analyses
                    .get_mut(&contract_id.build_id)
                    .and_then(|items| items.all_items_mut().get_mut(anchor.item_id as usize))
                    .expect("Anchor refers to non-existent coverage item")
                    .hits += hits;
            }
        }

        Ok(())
    }

    /// Returns the coverage items hit by a [`HitMap`] without mutating this report.
    pub fn hit_items_for_hit_map<'a>(
        &'a self,
        contract_id: &ContractId,
        hit_map: &HitMap,
        is_deployed_code: bool,
    ) -> Vec<(&'a CoverageItem, u32)> {
        let Some(anchors) = self.anchors.get(contract_id) else { return Vec::new() };
        let anchors = if is_deployed_code { &anchors.1 } else { &anchors.0 };

        let mut hits_by_item = BTreeMap::<u32, u32>::new();
        for anchor in anchors {
            if let Some(hits) = hit_map.get(anchor.instruction) {
                *hits_by_item.entry(anchor.item_id).or_default() += hits.get();
            }
        }
        if let Some(anchors) = self.execution_anchors.get(contract_id) {
            for anchor in &anchors.anchors {
                let hits = anchors.hits(hit_map, anchor.kind, is_deployed_code);
                if hits > 0 {
                    *hits_by_item.entry(anchor.item_id).or_default() += hits;
                }
            }
        }

        let Some(items) = self.analyses.get(&contract_id.build_id) else {
            return Vec::new();
        };
        hits_by_item
            .into_iter()
            .filter_map(|(item_id, hits)| {
                let item = items.get(item_id)?;
                Some((item, hits))
            })
            .collect()
    }

    /// Retains all the sources specified by `predicate`.
    ///
    /// This function should only be called after all the sources were used, otherwise, the output
    /// will be missing the ones that are dependent on them.
    pub fn retain_sources(&mut self, mut predicate: impl FnMut(&Path) -> bool) {
        self.source_paths.retain(|_, paths| {
            paths.retain(|_, path| predicate(path));
            !paths.is_empty()
        });

        let source_paths = &self.source_paths;
        self.source_paths_to_ids.retain(|build_id, paths| {
            paths.retain(|_, source_id| {
                source_paths.get(build_id).is_some_and(|paths| paths.contains_key(source_id))
            });
            !paths.is_empty()
        });
    }
}

/// A collection of [`HitMap`]s.
#[derive(Clone, Debug, Default)]
pub struct HitMaps(pub B256HashMap<HitMap>);

impl HitMaps {
    /// Merges two `Option<HitMaps>`.
    pub fn merge_opt(a: &mut Option<Self>, b: Option<Self>) {
        match (a, b) {
            (_, None) => {}
            (a @ None, Some(b)) => *a = Some(b),
            (Some(a), Some(b)) => a.merge(b),
        }
    }

    /// Merges two `HitMaps`.
    pub fn merge(&mut self, other: Self) {
        self.reserve(other.len());
        for (code_hash, other) in other.0 {
            self.entry(code_hash).and_modify(|e| e.merge(&other)).or_insert(other);
        }
    }

    /// Merges two `HitMaps`.
    pub fn merged(mut self, other: Self) -> Self {
        self.merge(other);
        self
    }
}

impl Deref for HitMaps {
    type Target = B256HashMap<HitMap>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HitMaps {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Copy, Debug)]
enum CallData {
    Empty,
    Short,
    Selector([u8; 4]),
}

impl CallData {
    fn new(input: &[u8]) -> Self {
        if input.is_empty() {
            Self::Empty
        } else if let Some(selector) = input.get(..4) {
            Self::Selector(selector.try_into().unwrap())
        } else {
            Self::Short
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CallHits {
    without_value: u32,
    with_value: u32,
}

impl CallHits {
    const fn hit(&mut self, with_value: bool) {
        if with_value {
            self.with_value += 1;
        } else {
            self.without_value += 1;
        }
    }

    const fn merge(&mut self, other: Self) {
        self.without_value += other.without_value;
        self.with_value += other.with_value;
    }

    const fn total(self, payable: bool) -> u32 {
        self.without_value + if payable { self.with_value } else { 0 }
    }
}

/// Hit data for an address.
///
/// Contains low-level data about hit counters for the instructions in the bytecode of a contract.
#[derive(Clone, Debug)]
pub struct HitMap {
    hits: FxHashMap<u32, u32>,
    bytecode: Bytes,
    creations: u32,
    empty_calls: CallHits,
    short_calls: CallHits,
    selector_calls: FxHashMap<[u8; 4], CallHits>,
}

impl HitMap {
    /// Create a new hitmap with the given bytecode.
    #[inline]
    pub fn new(bytecode: Bytes) -> Self {
        Self {
            bytecode,
            hits: HashMap::with_capacity_and_hasher(1024, Default::default()),
            creations: 0,
            empty_calls: Default::default(),
            short_calls: Default::default(),
            selector_calls: Default::default(),
        }
    }

    /// Returns the bytecode.
    #[inline]
    pub const fn bytecode(&self) -> &Bytes {
        &self.bytecode
    }

    /// Returns the number of hits for the given program counter.
    #[inline]
    pub fn get(&self, pc: u32) -> Option<NonZeroU32> {
        NonZeroU32::new(self.hits.get(&pc).copied().unwrap_or(0))
    }

    /// Increase the hit counter by 1 for the given program counter.
    #[inline]
    pub fn hit(&mut self, pc: u32) {
        self.hits(pc, 1)
    }

    /// Increase the hit counter by `hits` for the given program counter.
    #[inline]
    pub fn hits(&mut self, pc: u32, hits: u32) {
        *self.hits.entry(pc).or_default() += hits;
    }

    fn call(&mut self, call: CallData, with_value: bool) {
        let hits = match call {
            CallData::Empty => &mut self.empty_calls,
            CallData::Short => &mut self.short_calls,
            CallData::Selector(selector) => self.selector_calls.entry(selector).or_default(),
        };
        hits.hit(with_value);
    }

    const fn creation(&mut self) {
        self.creations += 1;
    }

    /// Reserve space for additional hits.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.hits.reserve(additional);
    }

    /// Merge another hitmap into this, assuming the bytecode is consistent
    pub fn merge(&mut self, other: &Self) {
        self.reserve(other.len());
        for (pc, hits) in other.iter() {
            self.hits(pc, hits);
        }
        self.creations += other.creations;
        self.empty_calls.merge(other.empty_calls);
        self.short_calls.merge(other.short_calls);
        for (&selector, &hits) in &other.selector_calls {
            self.selector_calls.entry(selector).or_default().merge(hits);
        }
    }

    /// Returns an iterator over all the program counters and their hit counts.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.hits.iter().map(|(&pc, &hits)| (pc, hits))
    }

    /// Returns the number of program counters hit in the hitmap.
    #[inline]
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// Returns `true` if the hitmap is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }
}

/// A unique identifier for a contract.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContractId {
    pub version: Version,
    pub build_id: String,
    pub source_id: usize,
    pub contract_name: Arc<str>,
}

impl fmt::Display for ContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Contract \"{}\" (solc {}, source ID {})",
            self.contract_name, self.version, self.source_id
        )
    }
}

/// An item anchor describes what instruction marks a [CoverageItem] as covered.
#[derive(Clone, Debug)]
pub struct ItemAnchor {
    /// The program counter for the opcode of this anchor.
    pub instruction: u32,
    /// The item ID this anchor points to.
    pub item_id: u32,
}

impl fmt::Display for ItemAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IC {} -> Item {}", self.instruction, self.item_id)
    }
}

/// An execution-based anchor for a coverage item without source-mapped bytecode.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionAnchor {
    /// The item ID this anchor points to.
    pub item_id: u32,
    /// The execution path that marks the item as covered.
    pub kind: ExecutionAnchorKind,
}

/// The execution path associated with an execution-based anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionAnchorKind {
    /// A successful contract creation.
    Constructor,
    /// An empty calldata call routed to `receive`.
    Receive,
    /// A call routed to `fallback`.
    Fallback,
}

#[derive(Clone, Debug)]
struct ContractExecutionAnchors {
    anchors: Vec<ExecutionAnchor>,
    function_selectors: FxHashSet<[u8; 4]>,
    has_receive: bool,
    fallback_payable: bool,
}

impl ContractExecutionAnchors {
    fn hits(&self, hit_map: &HitMap, kind: ExecutionAnchorKind, is_deployed_code: bool) -> u32 {
        match (kind, is_deployed_code) {
            (ExecutionAnchorKind::Constructor, false) => hit_map.creations,
            (ExecutionAnchorKind::Receive, true) => hit_map.empty_calls.total(true),
            (ExecutionAnchorKind::Fallback, true) => {
                let empty_calls = if self.has_receive {
                    0
                } else {
                    hit_map.empty_calls.total(self.fallback_payable)
                };
                empty_calls
                    + hit_map.short_calls.total(self.fallback_payable)
                    + hit_map
                        .selector_calls
                        .iter()
                        .filter(|(selector, _)| !self.function_selectors.contains(*selector))
                        .map(|(_, hits)| hits.total(self.fallback_payable))
                        .sum::<u32>()
            }
            _ => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum CoverageItemKind {
    /// An executable line in the code.
    Line,
    /// A statement in the code.
    Statement,
    /// A branch in the code.
    Branch {
        /// The ID that identifies the branch.
        ///
        /// There may be multiple items with the same branch ID - they belong to the same branch,
        /// but represent different paths.
        branch_id: u32,
        /// The path ID for this branch.
        ///
        /// The first path has ID 0, the next ID 1, and so on.
        path_id: u32,
        /// If true, then the branch anchor is the first opcode within the branch source range.
        is_first_opcode: bool,
    },
    /// A function in the code.
    Function {
        /// The name of the function.
        name: Box<str>,
    },
}

impl PartialEq for CoverageItemKind {
    fn eq(&self, other: &Self) -> bool {
        self.ord_key() == other.ord_key()
    }
}

impl Eq for CoverageItemKind {}

impl PartialOrd for CoverageItemKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CoverageItemKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ord_key().cmp(&other.ord_key())
    }
}

impl CoverageItemKind {
    fn ord_key(&self) -> impl Ord + use<> {
        match *self {
            Self::Line => 0,
            Self::Statement => 1,
            Self::Branch { .. } => 2,
            Self::Function { .. } => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CoverageItem {
    /// The coverage item kind.
    pub kind: CoverageItemKind,
    /// The location of the item in the source code.
    pub loc: SourceLocation,
    /// An alternative source location used only to find the item's bytecode anchor.
    pub anchor_loc: Option<SourceLocation>,
    /// The number of times this item was hit.
    pub hits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CoverageItemKindKey<'a> {
    Line,
    Statement,
    Branch { branch_id: u32, path_id: u32 },
    Function(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CoverageItemKey<'a> {
    line_start: u32,
    line_end: u32,
    kind_order: u8,
    byte_start: u32,
    byte_end: u32,
    contract_name: &'a str,
    kind: CoverageItemKindKey<'a>,
}

impl<'a> CoverageItemKey<'a> {
    fn new(item: &'a CoverageItem) -> Self {
        let (kind_order, kind) = match &item.kind {
            CoverageItemKind::Line => (0, CoverageItemKindKey::Line),
            CoverageItemKind::Statement => (1, CoverageItemKindKey::Statement),
            CoverageItemKind::Branch { branch_id, path_id, .. } => {
                (2, CoverageItemKindKey::Branch { branch_id: *branch_id, path_id: *path_id })
            }
            CoverageItemKind::Function { name } => {
                (3, CoverageItemKindKey::Function(name.as_ref()))
            }
        };

        Self {
            line_start: item.loc.lines.start,
            line_end: item.loc.lines.end,
            kind_order,
            byte_start: item.loc.bytes.start,
            byte_end: item.loc.bytes.end,
            contract_name: item.loc.contract_name.as_ref(),
            kind,
        }
    }
}

impl PartialEq for CoverageItem {
    fn eq(&self, other: &Self) -> bool {
        self.ord_key() == other.ord_key()
    }
}

impl Eq for CoverageItem {}

impl PartialOrd for CoverageItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CoverageItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ord_key().cmp(&other.ord_key())
    }
}

impl fmt::Display for CoverageItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_source(None).fmt(f)
    }
}

impl CoverageItem {
    fn ord_key(&self) -> impl Ord + use<> {
        (
            self.loc.source_id,
            self.loc.lines.start,
            self.loc.lines.end,
            self.kind.ord_key(),
            self.loc.bytes.start,
            self.loc.bytes.end,
        )
    }

    pub fn fmt_with_source(&self, src: Option<&str>) -> impl fmt::Display {
        solar::data_structures::fmt::from_fn(move |f| {
            match &self.kind {
                CoverageItemKind::Line => {
                    write!(f, "Line")?;
                }
                CoverageItemKind::Statement => {
                    write!(f, "Statement")?;
                }
                CoverageItemKind::Branch { branch_id, path_id, .. } => {
                    write!(f, "Branch (branch: {branch_id}, path: {path_id})")?;
                }
                CoverageItemKind::Function { name } => {
                    write!(f, r#"Function "{name}""#)?;
                }
            }
            write!(f, " (location: ({}), hits: {})", self.loc, self.hits)?;

            if let Some(src) = src
                && let Some(src) = src.get(self.loc.bytes())
            {
                write!(f, " -> ")?;

                let max_len = 64;
                let max_half = max_len / 2;

                if src.len() > max_len {
                    write!(f, "\"{}", src[..max_half].escape_debug())?;
                    write!(f, "...")?;
                    write!(f, "{}\"", src[src.len() - max_half..].escape_debug())?;
                } else {
                    write!(f, "{src:?}")?;
                }
            }

            Ok(())
        })
    }
}

/// A source location.
#[derive(Clone, Debug)]
pub struct SourceLocation {
    /// The source ID.
    pub source_id: usize,
    /// The contract this source range is in.
    pub contract_name: Arc<str>,
    /// Byte range.
    pub bytes: Range<u32>,
    /// Line range. Indices are 1-based.
    pub lines: Range<u32>,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "source ID: {}, lines: {:?}, bytes: {:?}", self.source_id, self.lines, self.bytes)
    }
}

impl SourceLocation {
    /// Returns the byte range as usize.
    pub const fn bytes(&self) -> Range<usize> {
        self.bytes.start as usize..self.bytes.end as usize
    }

    /// Returns the length of the byte range.
    pub fn len(&self) -> u32 {
        self.bytes.len() as u32
    }

    /// Returns true if the byte range is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Coverage summary for a source file.
#[derive(Clone, Debug, Default)]
pub struct CoverageSummary {
    /// The number of executable lines in the source file.
    pub line_count: usize,
    /// The number of lines that were hit.
    pub line_hits: usize,
    /// The number of statements in the source file.
    pub statement_count: usize,
    /// The number of statements that were hit.
    pub statement_hits: usize,
    /// The number of branches in the source file.
    pub branch_count: usize,
    /// The number of branches that were hit.
    pub branch_hits: usize,
    /// The number of functions in the source file.
    pub function_count: usize,
    /// The number of functions hit.
    pub function_hits: usize,
}

impl CoverageSummary {
    /// Creates a new, empty coverage summary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a coverage summary from a collection of coverage items.
    pub fn from_items<'a>(items: impl IntoIterator<Item = &'a CoverageItem>) -> Self {
        let mut summary = Self::default();
        summary.add_items(items);
        summary
    }

    /// Adds another coverage summary to this one.
    pub const fn merge(&mut self, other: &Self) {
        let Self {
            line_count,
            line_hits,
            statement_count,
            statement_hits,
            branch_count,
            branch_hits,
            function_count,
            function_hits,
        } = self;
        *line_count += other.line_count;
        *line_hits += other.line_hits;
        *statement_count += other.statement_count;
        *statement_hits += other.statement_hits;
        *branch_count += other.branch_count;
        *branch_hits += other.branch_hits;
        *function_count += other.function_count;
        *function_hits += other.function_hits;
    }

    /// Adds a coverage item to this summary.
    pub const fn add_item(&mut self, item: &CoverageItem) {
        match item.kind {
            CoverageItemKind::Line => {
                self.line_count += 1;
                if item.hits > 0 {
                    self.line_hits += 1;
                }
            }
            CoverageItemKind::Statement => {
                self.statement_count += 1;
                if item.hits > 0 {
                    self.statement_hits += 1;
                }
            }
            CoverageItemKind::Branch { .. } => {
                self.branch_count += 1;
                if item.hits > 0 {
                    self.branch_hits += 1;
                }
            }
            CoverageItemKind::Function { .. } => {
                self.function_count += 1;
                if item.hits > 0 {
                    self.function_hits += 1;
                }
            }
        }
    }

    /// Adds multiple coverage items to this summary.
    pub fn add_items<'a>(&mut self, items: impl IntoIterator<Item = &'a CoverageItem>) {
        for item in items {
            self.add_item(item);
        }
    }
}
