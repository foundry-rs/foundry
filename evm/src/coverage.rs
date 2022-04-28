use ethers::{
    prelude::{sourcemap::SourceMap, sources::VersionedSourceFile},
    types::Address,
};
use semver::Version;
use std::{
    collections::{BTreeMap, HashMap},
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
    sources: BTreeMap<(Version, u32), SourceFile>,
}

impl CoverageMap {
    pub fn new() -> Self {
        Default::default()
    }

    /// Adds coverage items and a source map for the given source.
    ///
    /// Sources are identified by path, and then by source ID and version.
    ///
    /// We need both the version and the source ID in case the project has distinct file
    /// hierarchies that use different compiler versions, as that will result in multiple compile
    /// jobs, and source IDs are only guaranteed to be stable across a single compile job.
    pub fn add_source(
        &mut self,
        path: String,
        source: VersionedSourceFile,
        items: Vec<CoverageItem>,
    ) {
        let VersionedSourceFile { version, source_file: source } = source;

        self.sources.insert((version, source.id), SourceFile { path: PathBuf::from(path), items });
    }

    pub fn add_hit_map(
        &mut self,
        source_version: Version,
        source_map: &SourceMap,
        hit_map: HitMap,
    ) {
        // NOTE(onbjerg): I've made an assumption here that the coverage items are laid out in
        // sorted order, ordered by their offset in the source code.
        //
        // This assumption is based on the way we process the AST - we start at the root node,
        // and work our way down. If we change up how we process the AST, we *have* to either
        // change this logic to work with unsorted data, or sort the data prior to calling
        // this function.
        for (ic, instruction_hits) in hit_map.hits.into_iter() {
            if instruction_hits == 0 {
                continue
            }

            // Get the instruction offset and the source ID in the source map.
            let (instruction_offset, source_id) = if let Some((instruction_offset, source_id)) =
                source_map
                    .get(ic)
                    .and_then(|source_element| Some((source_element.offset, source_element.index?)))
            {
                (instruction_offset, source_id)
            } else {
                continue
            };

            // Get the coverage items corresponding to the source ID in the source map.
            if let Some(source) = self.sources.get_mut(&(source_version.clone(), source_id)) {
                for item in source.items.iter_mut() {
                    match item {
                        CoverageItem::Line { offset, hits } |
                        CoverageItem::Statement { offset, hits } |
                        CoverageItem::Branch { offset, hits, .. } |
                        CoverageItem::Function { offset, hits, .. } => {
                            // We've reached a point where we will no longer be able to map this
                            // instruction to coverage items
                            if instruction_offset < *offset {
                                break
                            }

                            // We found a matching coverage item, but there may be more
                            if instruction_offset == *offset {
                                *hits += instruction_hits;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl IntoIterator for CoverageMap {
    type Item = SourceFile;
    type IntoIter = std::collections::btree_map::IntoValues<(Version, u32), Self::Item>;

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

#[derive(Clone, Debug)]
pub enum CoverageItem {
    /// An executable line in the code.
    Line {
        /// The byte offset in the source file for the start of the line.
        offset: usize,
        /// The number of times this item was hit.
        hits: u64,
    },

    /// A statement in the code.
    Statement {
        /// The byte offset in the source file for the start of the statement.
        offset: usize,
        /// The number of times this statement was hit.
        hits: u64,
    },

    /// A branch in the code.
    Branch {
        /// The ID that identifies the branch.
        ///
        /// There are two branches with the same ID,
        /// one for each outcome (true and false).
        id: usize,
        /// The branch kind.
        kind: BranchKind,
        /// The byte offset which the branch is on in the source file.
        offset: usize,
        /// The number of times this item was hit.
        hits: u64,
    },

    /// A function in the code.
    Function {
        /// The name of the function.
        name: String,
        /// The byte offset which the function is on in the source file.
        offset: usize,
        /// The number of times this item was hit.
        hits: u64,
    },
}

#[derive(Debug, Clone)]
pub enum BranchKind {
    /// A false branch
    True,
    /// A true branch
    False,
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

impl CoverageSummary {
    /// Add the data of another [CoverageSummary] to this one.
    pub fn add(&mut self, other: &CoverageSummary) {
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
