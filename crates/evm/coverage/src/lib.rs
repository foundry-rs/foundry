//! # foundry-evm-coverage
//!
//! EVM bytecode coverage analysis.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use alloy_primitives::{
    Bytes,
    map::{B256HashMap, HashMap, rustc_hash::FxHashMap},
};
use analysis::SourceAnalysis;
use eyre::Result;
use foundry_compilers::artifacts::sourcemap::SourceMap;
use semver::Version;
use std::{
    collections::BTreeMap,
    fmt::Display,
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
    /// A map of source IDs to the source path.
    pub source_paths: HashMap<(Version, usize), PathBuf>,
    /// A map of source paths to source IDs.
    pub source_paths_to_ids: HashMap<(Version, PathBuf), usize>,
    /// All coverage items for the codebase, keyed by the compiler version.
    pub analyses: HashMap<Version, SourceAnalysis>,
    /// All item anchors for the codebase, keyed by their contract ID.
    ///
    /// `(id, (creation, runtime))`
    pub anchors: HashMap<ContractId, (Vec<ItemAnchor>, Vec<ItemAnchor>)>,
    /// All the bytecode hits for the codebase.
    pub bytecode_hits: HashMap<ContractId, HitMap>,
    /// The bytecode -> source mappings.
    pub source_maps: HashMap<ContractId, (SourceMap, SourceMap)>,
}

impl CoverageReport {
    /// Add a source file path.
    pub fn add_source(&mut self, version: Version, source_id: usize, path: PathBuf) {
        self.source_paths.insert((version.clone(), source_id), path.clone());
        self.source_paths_to_ids.insert((version, path), source_id);
    }

    /// Get the source ID for a specific source file path.
    pub fn get_source_id(&self, version: Version, path: PathBuf) -> Option<usize> {
        self.source_paths_to_ids.get(&(version, path)).copied()
    }

    /// Add the source maps.
    pub fn add_source_maps(
        &mut self,
        source_maps: impl IntoIterator<Item = (ContractId, (SourceMap, SourceMap))>,
    ) {
        self.source_maps.extend(source_maps);
    }

    /// Add a [`SourceAnalysis`] to this report.
    pub fn add_analysis(&mut self, version: Version, analysis: SourceAnalysis) {
        self.analyses.insert(version, analysis);
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

    /// Returns an iterator over coverage summaries by source file path.
    pub fn summary_by_file(&self) -> impl Iterator<Item = (&Path, CoverageSummary)> {
        self.by_file(|summary: &mut CoverageSummary, item| summary.add_item(item))
    }

    /// Returns an iterator over coverage items by source file path.
    pub fn items_by_file(&self) -> impl Iterator<Item = (&Path, Vec<&CoverageItem>)> {
        self.by_file(|list: &mut Vec<_>, item| list.push(item))
    }

    fn by_file<'a, T: Default>(
        &'a self,
        mut f: impl FnMut(&mut T, &'a CoverageItem),
    ) -> impl Iterator<Item = (&'a Path, T)> {
        let mut by_file: BTreeMap<&Path, T> = BTreeMap::new();
        for (version, items) in &self.analyses {
            for item in items.all_items() {
                let key = (version.clone(), item.loc.source_id);
                let Some(path) = self.source_paths.get(&key) else { continue };
                f(by_file.entry(path).or_default(), item);
            }
        }
        by_file.into_iter()
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
                        .get_mut(&contract_id.version)
                        .and_then(|items| items.all_items_mut().get_mut(anchor.item_id as usize))
                        .expect("Anchor refers to non-existent coverage item")
                        .hits += hits.get();
                }
            }
        }

        Ok(())
    }

    /// Retains all the coverage items specified by `predicate`.
    ///
    /// This function should only be called after all the sources were used, otherwise, the output
    /// will be missing the ones that are dependent on them.
    pub fn retain_sources(&mut self, mut predicate: impl FnMut(&Path) -> bool) {
        self.analyses.retain(|version, analysis| {
            analysis.all_items_mut().retain(|item| {
                self.source_paths
                    .get(&(version.clone(), item.loc.source_id))
                    .map(|path| predicate(path))
                    .unwrap_or(false)
            });
            !analysis.all_items().is_empty()
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

/// Hit data for an address.
///
/// Contains low-level data about hit counters for the instructions in the bytecode of a contract.
#[derive(Clone, Debug)]
pub struct HitMap {
    hits: FxHashMap<u32, u32>,
    bytecode: Bytes,
}

impl HitMap {
    /// Create a new hitmap with the given bytecode.
    #[inline]
    pub fn new(bytecode: Bytes) -> Self {
        Self { bytecode, hits: HashMap::with_capacity_and_hasher(1024, Default::default()) }
    }

    /// Returns the bytecode.
    #[inline]
    pub fn bytecode(&self) -> &Bytes {
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

    /// Merge another hitmap into this, assuming the bytecode is consistent
    pub fn merge(&mut self, other: &Self) {
        self.hits.reserve(other.len());
        for (pc, hits) in other.iter() {
            self.hits(pc, hits);
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

/// A unique identifier for a contract
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContractId {
    pub version: Version,
    pub source_id: usize,
    pub contract_name: Arc<str>,
}

impl Display for ContractId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl Display for ItemAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IC {} -> Item {}", self.instruction, self.item_id)
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

#[derive(Clone, Debug)]
pub struct CoverageItem {
    /// The coverage item kind.
    pub kind: CoverageItemKind,
    /// The location of the item in the source code.
    pub loc: SourceLocation,
    /// The number of times this item was hit.
    pub hits: u32,
}

impl Display for CoverageItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
        write!(f, " (location: ({}), hits: {})", self.loc, self.hits)
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

impl Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "source ID: {}, lines: {:?}, bytes: {:?}", self.source_id, self.lines, self.bytes)
    }
}

impl SourceLocation {
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
    pub fn merge(&mut self, other: &Self) {
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
    pub fn add_item(&mut self, item: &CoverageItem) {
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
