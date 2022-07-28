mod visitor;
pub use visitor::Visitor;

use ethers::{
    prelude::{sourcemap::SourceMap, sources::VersionedSourceFile},
    types::Address,
};
use semver::Version;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    ops::AddAssign,
    path::PathBuf,
    usize,
};

/// A coverage map.
///
/// The coverage map collects coverage items for sources. It also converts hit data from
/// [HitMap]s into their appropriate coverage items.
///
/// You **MUST** add all the sources before you start adding hit data.
#[derive(Default, Debug, Clone)]
pub struct CoverageMap {
    /// A map of `(version, source id)` -> `source file`
    sources: HashMap<(Version, u32), SourceFile>,
}

impl CoverageMap {
    /// Adds coverage items and a source map for the given source.
    ///
    /// Sources are identified by path, and then by source ID and version.
    ///
    /// We need both the version and the source ID in case the project has distinct file
    /// hierarchies that use different compiler versions, as that will result in multiple compile
    /// jobs, and source IDs are only guaranteed to be stable across a single compile job.
    pub fn add_source(
        &mut self,
        path: impl Into<PathBuf>,
        source: VersionedSourceFile,
        items: Vec<CoverageItem>,
    ) {
        let VersionedSourceFile { version, source_file: source } = source;

        self.sources.insert((version, source.id), SourceFile { path: path.into(), items });
    }

    /// Processes data from a [HitMap] and sets hit counts for coverage items in this coverage map.
    ///
    /// This function should only be called *after* all the relevant sources have been processed and
    /// added to the map (see [add_source]).
    ///
    /// NOTE(onbjerg): I've made an assumption here that the coverage items are laid out in
    /// sorted order, ordered by their anchors.
    ///
    /// This assumption is based on the way we process the AST - we start at the root node,
    /// and work our way down. If we change up how we process the AST, we *have* to either
    /// change this logic to work with unsorted data, or sort the data prior to calling
    /// this function.
    pub fn add_hit_map(
        &mut self,
        source_version: Version,
        source_map: &SourceMap,
        contract_name: &str,
        hit_map: HitMap,
    ) {
        for (ic, instruction_hits) in hit_map.hits.into_iter() {
            if instruction_hits == 0 {
                continue
            }

            // Get the source ID in the source map.
            let source_id =
                if let Some(source_id) = source_map.get(ic).and_then(|element| element.index) {
                    source_id
                } else {
                    continue
                };

            // Get the coverage items corresponding to the source ID in the source map.
            if let Some(source) = self.sources.get_mut(&(source_version.clone(), source_id)) {
                for item in source.items.iter_mut() {
                    // We found a matching coverage item, but there may be more
                    let anchor = item.anchor();
                    if ic == anchor.instruction && contract_name == anchor.contract {
                        item.increment_hits(instruction_hits);
                    }
                }
            }
        }
    }
}

impl IntoIterator for CoverageMap {
    type Item = SourceFile;
    type IntoIter = std::collections::hash_map::IntoValues<(Version, u32), Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.sources.into_values()
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

/// A source file.
#[derive(Default, Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub items: Vec<CoverageItem>,
}

impl SourceFile {
    /// Get a simple summary of the coverage for the file.
    pub fn summary(&self) -> CoverageSummary {
        self.items.iter().fold(CoverageSummary::default(), |mut summary, item| match item {
            CoverageItem::Line { hits, .. } => {
                summary.line_count += 1;
                if *hits > 0 {
                    summary.line_hits += 1;
                }
                summary
            }
            CoverageItem::Statement { hits, .. } => {
                summary.statement_count += 1;
                if *hits > 0 {
                    summary.statement_hits += 1;
                }
                summary
            }
            CoverageItem::Branch { hits, .. } => {
                summary.branch_count += 1;
                if *hits > 0 {
                    summary.branch_hits += 1;
                }
                summary
            }
            CoverageItem::Function { hits, .. } => {
                summary.function_count += 1;
                if *hits > 0 {
                    summary.function_hits += 1;
                }
                summary
            }
        })
    }
}

/// An item anchor describes what instruction (and what contract) marks a [CoverageItem] as covered.
#[derive(Clone, Debug)]
pub struct ItemAnchor {
    /// The instruction counter that constitutes this anchor
    pub instruction: usize,
    /// The contract in which the instruction is in
    pub contract: String,
}

impl Display for ItemAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.contract, self.instruction)
    }
}

#[derive(Clone, Debug)]
pub enum CoverageItem {
    /// An executable line in the code.
    Line {
        /// The location of the line in the source code.
        loc: SourceLocation,
        /// The instruction counter that covers this line.
        anchor: ItemAnchor,
        /// The number of times this item was hit.
        hits: u64,
    },

    /// A statement in the code.
    Statement {
        /// The location of the statement in the source code.
        loc: SourceLocation,
        /// The instruction counter that covers this statement.
        anchor: ItemAnchor,
        /// The number of times this statement was hit.
        hits: u64,
    },

    /// A branch in the code.
    Branch {
        /// The location of the branch in the source code.
        loc: SourceLocation,
        /// The instruction counter that covers this branch.
        anchor: ItemAnchor,
        /// The ID that identifies the branch.
        ///
        /// There may be multiple items with the same branch ID - they belong to the same branch,
        /// but represent different paths.
        branch_id: usize,
        /// The path ID for this branch.
        ///
        /// The first path has ID 0, the next ID 1, and so on.
        path_id: usize,
        /// The number of times this item was hit.
        hits: u64,
    },

    /// A function in the code.
    Function {
        /// The location of the function in the source code.
        loc: SourceLocation,
        /// The instruction counter that covers this function.
        anchor: ItemAnchor,
        /// The name of the function.
        name: String,
        /// The number of times this item was hit.
        hits: u64,
    },
}

impl CoverageItem {
    pub fn source_location(&self) -> &SourceLocation {
        match self {
            Self::Line { loc, .. } |
            Self::Statement { loc, .. } |
            Self::Branch { loc, .. } |
            Self::Function { loc, .. } => loc,
        }
    }

    pub fn anchor(&self) -> &ItemAnchor {
        match self {
            Self::Line { anchor, .. } |
            Self::Statement { anchor, .. } |
            Self::Branch { anchor, .. } |
            Self::Function { anchor, .. } => anchor,
        }
    }

    pub fn increment_hits(&mut self, delta: u64) {
        match self {
            Self::Line { hits, .. } |
            Self::Statement { hits, .. } |
            Self::Branch { hits, .. } |
            Self::Function { hits, .. } => *hits += delta,
        }
    }

    pub fn hits(&self) -> u64 {
        match self {
            Self::Line { hits, .. } |
            Self::Statement { hits, .. } |
            Self::Branch { hits, .. } |
            Self::Function { hits, .. } => *hits,
        }
    }
}

impl Display for CoverageItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            CoverageItem::Line { loc, anchor, hits } => {
                write!(f, "Line (location: {loc}, anchor: {anchor}, hits: {hits})")
            }
            CoverageItem::Statement { loc, anchor, hits } => {
                write!(f, "Statement (location: {loc}, anchor: {anchor}, hits: {hits})")
            }
            CoverageItem::Branch { loc, anchor, hits, branch_id, path_id } => {
                write!(f, "Branch (branch: {branch_id}, path: {path_id}) (location: {loc}, anchor: {anchor}, hits: {hits})")
            }
            CoverageItem::Function { loc, anchor, hits, name } => {
                write!(f, r#"Function "{name}" (location: {loc}, anchor: {anchor}, hits: {hits})"#)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceLocation {
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
            "L{}, C{}-{}",
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
