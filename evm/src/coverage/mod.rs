pub mod analysis;
pub mod anchors;

use ethers::types::Address;
use semver::Version;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    ops::AddAssign,
};

/// A coverage report.
///
/// A coverage report contains coverage items and opcodes corresponding to those items (called
/// "anchors"). A single coverage item may be referred to by multiple anchors.
#[derive(Default, Debug, Clone)]
pub struct CoverageReport {
    /// A map of source IDs to the source path
    pub source_paths: HashMap<(Version, usize), String>,
    /// A map of source paths to source IDs
    pub source_paths_to_ids: HashMap<(Version, String), usize>,
    /// All coverage items for the codebase, keyed by the compiler version.
    pub items: HashMap<Version, Vec<CoverageItem>>,
    /// All item anchors for the codebase, keyed by their contract ID.
    pub anchors: HashMap<ContractId, Vec<ItemAnchor>>,
}

impl CoverageReport {
    /// Add a source file path.
    pub fn add_source(&mut self, version: Version, source_id: usize, path: String) {
        self.source_paths.insert((version.clone(), source_id), path.clone());
        self.source_paths_to_ids.insert((version, path), source_id);
    }

    /// Get the source ID for a specific source file path.
    pub fn get_source_id(&self, version: Version, path: String) -> Option<&usize> {
        self.source_paths_to_ids.get(&(version, path))
    }

    /// Add coverage items to this report
    pub fn add_items(&mut self, version: Version, items: Vec<CoverageItem>) {
        self.items.entry(version).or_default().extend(items);
    }

    /// Add anchors to this report
    pub fn add_anchors(&mut self, anchors: HashMap<ContractId, Vec<ItemAnchor>>) {
        self.anchors.extend(anchors);
    }

    /// Get coverage summaries by source file path
    pub fn summary_by_file(&self) -> impl Iterator<Item = (String, CoverageSummary)> {
        let mut summaries: BTreeMap<String, CoverageSummary> = BTreeMap::new();

        for (version, items) in self.items.iter() {
            for item in items {
                let mut summary = summaries
                    .entry(
                        self.source_paths
                            .get(&(version.clone(), item.loc.source_id))
                            .cloned()
                            .unwrap_or_else(|| {
                                format!("Unknown (ID: {}, solc: {})", item.loc.source_id, version)
                            }),
                    )
                    .or_default();
                summary += item;
            }
        }

        summaries.into_iter()
    }

    /// Get coverage items by source file path
    pub fn items_by_source(&self) -> impl Iterator<Item = (String, Vec<CoverageItem>)> {
        let mut items_by_source: BTreeMap<String, Vec<CoverageItem>> = BTreeMap::new();

        for (version, items) in self.items.iter() {
            for item in items {
                items_by_source
                    .entry(
                        self.source_paths
                            .get(&(version.clone(), item.loc.source_id))
                            .cloned()
                            .unwrap_or_else(|| {
                                format!("Unknown (ID: {}, solc: {})", item.loc.source_id, version)
                            }),
                    )
                    .or_default()
                    .push(item.clone());
            }
        }

        items_by_source.into_iter()
    }

    /// Processes data from a [HitMap] and sets hit counts for coverage items in this coverage map.
    ///
    /// This function should only be called *after* all the relevant sources have been processed and
    /// added to the map (see [add_source]).
    pub fn add_hit_map(&mut self, contract_id: &ContractId, hit_map: &HitMap) {
        if let Some(anchors) = self.anchors.get(contract_id) {
            for anchor in anchors {
                if let Some(hits) = hit_map.hits.get(&anchor.instruction) {
                    self.items
                        .get_mut(&contract_id.version)
                        .and_then(|items| items.get_mut(anchor.item_id))
                        .expect("Anchor refers to non-existent coverage item")
                        .hits += hits;
                }
            }
        }
    }
}

/// A collection of [HitMap]s
pub type HitMaps = HashMap<Address, HitMap>;

/// Hit data for an address.
///
/// Contains low-level data about hit counters for the instructions in the bytecode of a contract.
#[derive(Debug, Clone, Default)]
pub struct HitMap {
    hits: BTreeMap<usize, u64>,
}

impl HitMap {
    /// Increase the hit counter for the given instruction counter.
    pub fn hit(&mut self, ic: usize) {
        *self.hits.entry(ic).or_default() += 1;
    }
}

/// A unique identifier for a contract
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ContractId {
    pub version: Version,
    pub source_id: usize,
    pub contract_name: String,
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
    /// The instruction counter for the opcode of this anchor
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
            CoverageItemKind::Branch { branch_id, path_id } => {
                write!(f, "Branch (branch: {branch_id}, path: {path_id})")?;
            }
            CoverageItemKind::Function { name } => {
                write!(f, r#"Function "{name}""#)?;
            }
        }
        write!(f, " (location: {}, hits: {})", self.loc, self.hits)
    }
}

#[derive(Debug, Clone)]
pub struct SourceLocation {
    /// The source ID.
    pub source_id: usize,
    /// The contract this source range is in.
    pub contract_name: String,
    /// Start byte in the source code.
    pub start: usize,
    /// Number of bytes in the source code.
    pub length: Option<usize>,
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
#[derive(Default, Debug, Clone)]
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

impl AddAssign<&CoverageItem> for &mut CoverageSummary {
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
