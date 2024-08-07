//! # foundry-evm-coverage
//!
//! EVM bytecode coverage analysis.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use alloy_primitives::{Bytes, B256};
use eyre::{Context, Result};
use foundry_compilers::artifacts::sourcemap::SourceMap;
use semver::Version;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    ops::{AddAssign, Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
};

pub mod analysis;
pub mod anchors;

mod inspector;
pub use inspector::CoverageCollector;

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
    pub items: HashMap<Version, Vec<CoverageItem>>,
    /// All item anchors for the codebase, keyed by their contract ID.
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

    /// Add coverage items to this report.
    pub fn add_items(&mut self, version: Version, items: impl IntoIterator<Item = CoverageItem>) {
        self.items.entry(version).or_default().extend(items);
    }

    /// Add anchors to this report.
    pub fn add_anchors(
        &mut self,
        anchors: impl IntoIterator<Item = (ContractId, (Vec<ItemAnchor>, Vec<ItemAnchor>))>,
    ) {
        self.anchors.extend(anchors);
    }

    /// Get coverage summaries by source file path.
    pub fn summary_by_file(&self) -> impl Iterator<Item = (PathBuf, CoverageSummary)> {
        let mut summaries = BTreeMap::new();

        for (version, items) in self.items.iter() {
            for item in items {
                let Some(path) =
                    self.source_paths.get(&(version.clone(), item.loc.source_id)).cloned()
                else {
                    continue;
                };
                *summaries.entry(path).or_default() += item;
            }
        }

        summaries.into_iter()
    }

    /// Get coverage items by source file path.
    pub fn items_by_source(&self) -> impl Iterator<Item = (PathBuf, Vec<CoverageItem>)> {
        let mut items_by_source: BTreeMap<_, Vec<_>> = BTreeMap::new();

        for (version, items) in self.items.iter() {
            for item in items {
                let Some(path) =
                    self.source_paths.get(&(version.clone(), item.loc.source_id)).cloned()
                else {
                    continue;
                };
                items_by_source.entry(path).or_default().push(item.clone());
            }
        }

        items_by_source.into_iter()
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
        // Add bytecode level hits
        let e = self
            .bytecode_hits
            .entry(contract_id.clone())
            .or_insert_with(|| HitMap::new(hit_map.bytecode.clone()));
        e.merge(hit_map).wrap_err_with(|| format!("{contract_id:?}"))?;

        // Add source level hits
        if let Some(anchors) = self.anchors.get(contract_id) {
            let anchors = if is_deployed_code { &anchors.1 } else { &anchors.0 };
            for anchor in anchors {
                if let Some(&hits) = hit_map.hits.get(&anchor.instruction) {
                    self.items
                        .get_mut(&contract_id.version)
                        .and_then(|items| items.get_mut(anchor.item_id))
                        .expect("Anchor refers to non-existent coverage item")
                        .hits += hits;
                }
            }
        }
        Ok(())
    }

    /// Removes all the coverage items that should be ignored by the filter.
    ///
    /// This function should only be called after all the sources were used, otherwise, the output
    /// will be missing the ones that are dependent on them.
    pub fn filter_out_ignored_sources(&mut self, filter: impl Fn(&Path) -> bool) {
        self.items.retain(|version, items| {
            items.retain(|item| {
                self.source_paths
                    .get(&(version.clone(), item.loc.source_id))
                    .map(|path| filter(path))
                    .unwrap_or(false)
            });
            !items.is_empty()
        });
    }
}

/// A collection of [`HitMap`]s.
#[derive(Clone, Debug, Default)]
pub struct HitMaps(pub HashMap<B256, HitMap>);

impl HitMaps {
    pub fn merge(&mut self, other: Self) {
        for (code_hash, hit_map) in other.0 {
            if let Some(HitMap { hits: extra_hits, .. }) = self.insert(code_hash, hit_map) {
                for (pc, hits) in extra_hits {
                    self.entry(code_hash)
                        .and_modify(|map| *map.hits.entry(pc).or_default() += hits);
                }
            }
        }
    }

    pub fn merged(mut self, other: Self) -> Self {
        self.merge(other);
        self
    }
}

impl Deref for HitMaps {
    type Target = HashMap<B256, HitMap>;

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
    pub bytecode: Bytes,
    pub hits: BTreeMap<usize, u64>,
}

impl HitMap {
    pub fn new(bytecode: Bytes) -> Self {
        Self { bytecode, hits: BTreeMap::new() }
    }

    /// Increase the hit counter for the given program counter.
    pub fn hit(&mut self, pc: usize) {
        *self.hits.entry(pc).or_default() += 1;
    }

    /// Merge another hitmap into this, assuming the bytecode is consistent
    pub fn merge(&mut self, other: &Self) -> Result<(), eyre::Report> {
        for (pc, hits) in &other.hits {
            *self.hits.entry(*pc).or_default() += hits;
        }
        Ok(())
    }

    pub fn consistent_bytecode(&self, hm1: &Self, hm2: &Self) -> bool {
        // Consider the bytecodes consistent if they are the same out as far as the
        // recorded hits
        let len1 = hm1.hits.last_key_value();
        let len2 = hm2.hits.last_key_value();
        if let (Some(len1), Some(len2)) = (len1, len2) {
            let len = std::cmp::max(len1.0, len2.0);
            let ok = hm1.bytecode.0[..*len] == hm2.bytecode.0[..*len];
            println!("consistent_bytecode: {}, {}, {}, {}", ok, len1.0, len2.0, len);
            return ok;
        }
        true
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
    /// The program counter for the opcode of this anchor
    pub instruction: usize,
    /// The item ID this anchor points to
    pub item_id: usize,
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
        branch_id: usize,
        /// The path ID for this branch.
        ///
        /// The first path has ID 0, the next ID 1, and so on.
        path_id: usize,
        /// If true, then the branch anchor is the first opcode within the branch source range.
        is_first_opcode: bool,
    },
    /// A function in the code.
    Function {
        /// The name of the function.
        name: String,
    },
}

#[derive(Clone, Debug)]
pub struct CoverageItem {
    /// The coverage item kind.
    pub kind: CoverageItemKind,
    /// The location of the item in the source code.
    pub loc: SourceLocation,
    /// The number of times this item was hit.
    pub hits: u64,
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
        write!(f, " (location: {}, hits: {})", self.loc, self.hits)
    }
}

#[derive(Clone, Debug)]
pub struct SourceLocation {
    /// The source ID.
    pub source_id: usize,
    /// The contract this source range is in.
    pub contract_name: Arc<str>,
    /// Start byte in the source code.
    pub start: u32,
    /// Number of bytes in the source code.
    pub length: Option<u32>,
    /// The line in the source code.
    pub line: usize,
}

impl Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "source ID {}, line {}, chars {}-{}",
            self.source_id,
            self.line,
            self.start,
            self.length.map_or(self.start, |length| self.start + length)
        )
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

impl AddAssign<&Self> for CoverageSummary {
    fn add_assign(&mut self, other: &Self) {
        self.line_count += other.line_count;
        self.line_hits += other.line_hits;
        self.statement_count += other.statement_count;
        self.statement_hits += other.statement_hits;
        self.branch_count += other.branch_count;
        self.branch_hits += other.branch_hits;
        self.function_count += other.function_count;
        self.function_hits += other.function_hits;
    }
}

impl AddAssign<&CoverageItem> for CoverageSummary {
    fn add_assign(&mut self, item: &CoverageItem) {
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
}
