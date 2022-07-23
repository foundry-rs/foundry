mod visitor;

use ethers::{
    prelude::{
        artifacts::{Ast, Node, NodeType},
        sourcemap::SourceMap,
        Bytes,
    },
    types::Address,
};
use revm::{opcode, spec_opcode_gas, SpecId};
use semver::Version;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    ops::AddAssign,
};
use visitor::ContractVisitor;

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
    // TODO: Doc
    pub fn add_source(&mut self, version: Version, source_id: usize, path: String) {
        self.source_paths.insert((version.clone(), source_id), path.clone());
        self.source_paths_to_ids.insert((version, path), source_id);
    }

    // TODO: Doc
    pub fn get_source_id(&self, version: Version, path: String) -> Option<&usize> {
        self.source_paths_to_ids.get(&(version, path))
    }

    // TODO: Doc
    pub fn add_items(&mut self, version: Version, items: Vec<CoverageItem>) {
        self.items.entry(version).or_default().extend(items);
    }

    // TODO: Doc
    pub fn add_anchors(&mut self, anchors: HashMap<ContractId, Vec<ItemAnchor>>) {
        self.anchors.extend(anchors);
    }

    // TODO: Doc
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

    // TODO: Doc
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

// TODO: Docs
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

#[derive(Debug)]
pub struct SourceAnalysis {
    /// A collection of coverage items.
    pub items: Vec<CoverageItem>,
    /// A mapping of contract IDs to item IDs relevant to the contract (including items in base
    /// contracts).
    pub contract_items: HashMap<ContractId, Vec<usize>>,
}

/// Analyzes a set of sources to find coverage items.
#[derive(Default, Clone, Debug)]
pub struct SourceAnalyzer {
    /// A map of source IDs to their source code
    sources: HashMap<usize, String>,
    /// A map of AST node IDs of contracts to their contract IDs.
    contract_ids: HashMap<usize, ContractId>,
    /// A map of contract IDs to their AST nodes.
    contracts: HashMap<ContractId, Node>,
    /// A collection of coverage items.
    items: Vec<CoverageItem>,
    /// A map of contract IDs to item IDs.
    contract_items: HashMap<ContractId, Vec<usize>>,
    /// A map of contracts to their base contracts
    contract_bases: HashMap<ContractId, Vec<ContractId>>,
}

impl SourceAnalyzer {
    /// Creates a new source analyzer.
    ///
    /// The source analyzer expects all given sources to belong to the same compilation job
    /// (defined by `version`).
    pub fn new(
        version: Version,
        asts: HashMap<usize, Ast>,
        sources: HashMap<usize, String>,
    ) -> eyre::Result<Self> {
        let mut analyzer = SourceAnalyzer { sources, ..Default::default() };

        // TODO: Skip interfaces
        for (source_id, ast) in asts.into_iter() {
            for child in ast.nodes {
                if !matches!(child.node_type, NodeType::ContractDefinition) {
                    continue
                }

                let node_id =
                    child.id.ok_or_else(|| eyre::eyre!("The contract's AST node has no ID"))?;
                let contract_id = ContractId {
                    version: version.clone(),
                    source_id,
                    contract_name: child
                        .attribute("name")
                        .ok_or_else(|| eyre::eyre!("Contract has no name"))?,
                };
                analyzer.contract_ids.insert(node_id, contract_id.clone());
                analyzer.contracts.insert(contract_id, child);
            }
        }

        Ok(analyzer)
    }

    /// Analyzes contracts in the sources held by the source analyzer.
    ///
    /// Coverage items are found by:
    /// - Walking the AST of each contract (except interfaces)
    /// - Recording the items and base contracts of each contract
    ///
    /// Finally, the item IDs of each contract and its base contracts are flattened, and the return
    /// value represents all coverage items in the project, along with a mapping of contract IDs to
    /// item IDs.
    ///
    /// Each coverage item contains relevant information to find opcodes corresponding to them: the
    /// source ID the item is in, the source code range of the item, and the contract name the item
    /// is in.
    ///
    /// Note: Source IDs are only unique per compilation job; that is, a code base compiled with
    /// two different solc versions will produce overlapping source IDs if the compiler version is
    /// not taken into account.
    pub fn analyze(mut self) -> eyre::Result<SourceAnalysis> {
        // Analyze the contracts
        self.analyze_contracts()?;

        // Flatten the data
        let mut flattened: HashMap<ContractId, Vec<usize>> = HashMap::new();
        for (contract_id, own_item_ids) in &self.contract_items {
            let mut item_ids = own_item_ids.clone();
            if let Some(base_contract_ids) = self.contract_bases.get(contract_id) {
                for item_id in
                    base_contract_ids.iter().filter_map(|id| self.contract_items.get(id)).flatten()
                {
                    item_ids.push(*item_id);
                }
            }

            // If there are no items for this contract, then it was most likely filtered
            if !item_ids.is_empty() {
                flattened.insert(contract_id.clone(), item_ids);
            }
        }

        Ok(SourceAnalysis { items: self.items.clone(), contract_items: flattened })
    }

    fn analyze_contracts(&mut self) -> eyre::Result<()> {
        for (contract_id, contract) in &self.contracts {
            let base_contract_node_ids: Vec<usize> =
                contract.attribute("linearizedBaseContracts").ok_or_else(|| {
                    eyre::eyre!(
                        "The contract's AST node is missing a list of linearized base contracts"
                    )
                })?;

            // Find this contract's coverage items if we haven't already
            if self.contract_items.get(contract_id).is_none() {
                let items = ContractVisitor::new(
                    contract_id.source_id,
                    self.sources.get(&contract_id.source_id).unwrap_or_else(|| {
                        panic!(
                            "We should have the source code for source ID {}",
                            contract_id.source_id
                        )
                    }),
                    contract_id.contract_name.clone(),
                )
                .visit(
                    self.contracts
                        .get(contract_id)
                        .unwrap_or_else(|| {
                            panic!("We should have the AST of contract: {:?}", contract_id)
                        })
                        .clone(),
                )?;
                let is_test = items.iter().any(|item| {
                    if let CoverageItemKind::Function { name } = &item.kind {
                        name.starts_with("test")
                    } else {
                        false
                    }
                });

                // Record this contract's base contracts
                // We don't do this for test contracts because we don't care about them
                if !is_test {
                    self.contract_bases.insert(
                        contract_id.clone(),
                        base_contract_node_ids[1..]
                            .iter()
                            .filter_map(|base_contract_node_id| {
                                self.contract_ids.get(base_contract_node_id).cloned()
                            })
                            .collect(),
                    );
                }

                // For tests and contracts with no items we still record an empty Vec so we don't
                // end up here again
                if items.is_empty() || is_test {
                    self.contract_items.insert(contract_id.clone(), Vec::new());
                } else {
                    let item_ids: Vec<usize> =
                        (self.items.len()..self.items.len() + items.len()).collect();
                    self.items.extend(items);
                    self.contract_items.insert(contract_id.clone(), item_ids.clone());
                }
            }
        }

        Ok(())
    }
}

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &Bytes,
    source_map: &SourceMap,
    item_ids: &[usize],
    items: &[CoverageItem],
) -> Vec<ItemAnchor> {
    item_ids
        .iter()
        .filter_map(|item_id| {
            let item = items.get(*item_id)?;

            match item.kind {
                CoverageItemKind::Branch { path_id, .. } => {
                    match find_anchor_branch(bytecode, source_map, *item_id, &item.loc) {
                        Ok(anchors) => match path_id {
                            0 => Some(anchors.0),
                            1 => Some(anchors.1),
                            _ => panic!("Too many paths for branch"),
                        },
                        Err(e) => {
                            tracing::warn!("Could not find anchor for item: {}, error: {e}", item);
                            None
                        }
                    }
                }
                _ => match find_anchor_simple(source_map, *item_id, &item.loc) {
                    Ok(anchor) => Some(anchor),
                    Err(e) => {
                        tracing::warn!("Could not find anchor for item: {}, error: {e}", item);
                        None
                    }
                },
            }
        })
        .collect()
}

/// Find an anchor representing the first opcode within the given source range.
pub fn find_anchor_simple(
    source_map: &SourceMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<ItemAnchor> {
    let instruction = source_map
        .iter()
        .enumerate()
        .find_map(|(ic, element)| {
            if element.index? as usize == loc.source_id &&
                loc.start.max(element.offset) <
                    (element.offset + element.length)
                        .min(loc.start + loc.length.unwrap_or_default())
            {
                return Some(ic)
            }

            None
        })
        .ok_or_else(|| {
            eyre::eyre!("Could not find anchor: No matching instruction in range {}", loc)
        })?;

    Ok(ItemAnchor { instruction, item_id })
}

/// Finds the anchor corresponding to a branch item.
///
/// This finds the relevant anchors for a branch coverage item. These anchors
/// are found using the bytecode of the contract in the range of the branching node.
///
/// For `IfStatement` nodes, the template is generally:
/// ```text
/// <condition>
/// PUSH <ic if false>
/// JUMPI
/// <true branch>
/// <...>
/// <false branch>
/// ```
///
/// For `assert` and `require`, the template is generally:
///
/// ```text
/// PUSH <ic if true>
/// JUMPI
/// <revert>
/// <...>
/// <true branch>
/// ```
///
/// This function will look for the last JUMPI instruction, backtrack to find the instruction
/// counter of the first branch, and return an item for that instruction counter, and the
/// instruction counter immediately after the JUMPI instruction.
pub fn find_anchor_branch(
    bytecode: &Bytes,
    source_map: &SourceMap,
    item_id: usize,
    loc: &SourceLocation,
) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
    // NOTE(onbjerg): We use `SpecId::LATEST` here since it does not matter; the only difference
    // is the gas cost.
    let opcode_infos = spec_opcode_gas(SpecId::LATEST);

    let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();
    let mut first_branch_ic = None;
    let mut second_branch_pc = None;
    let mut pc = 0;
    let mut cumulative_push_size = 0;
    while pc < bytecode.0.len() {
        let op = bytecode.0[pc];
        ic_map.insert(pc, pc - cumulative_push_size);

        // We found a push, so we do some PC -> IC translation accounting, but we also check if
        // this push is coupled with the JUMPI we are interested in.
        if opcode_infos[op as usize].is_push() {
            let element = if let Some(element) = source_map.get(pc - cumulative_push_size) {
                element
            } else {
                // NOTE(onbjerg): For some reason the last few bytes of the bytecode do not have
                // a source map associated, so at that point we just stop searching
                break
            };

            // Do push byte accounting
            let push_size = (op - opcode::PUSH1 + 1) as usize;
            pc += push_size;
            cumulative_push_size += push_size;

            // Check if we are in the source range we are interested in, and if the next opcode
            // is a JUMPI
            let source_ids_match = element.index.map_or(false, |a| a as usize == loc.source_id);
            let is_in_source_range = loc.start.max(element.offset) <
                (element.offset + element.length).min(loc.start + loc.length.unwrap_or_default());
            if source_ids_match && is_in_source_range && bytecode.0[pc + 1] == opcode::JUMPI {
                // We do not support program counters bigger than usize. This is also an
                // assumption in REVM, so this is just a sanity check.
                if push_size > 8 {
                    panic!("We found the anchor for the branch, but it refers to a program counter bigger than 64 bits.");
                }

                // The first branch is the opcode directly after JUMPI
                first_branch_ic = Some(pc + 2 - cumulative_push_size);

                // Convert the push bytes for the second branch's PC to a usize
                let push_bytes_start = pc - push_size + 1;
                let mut pc_bytes: [u8; 8] = [0; 8];
                for (i, push_byte) in
                    bytecode.0[push_bytes_start..push_bytes_start + push_size].iter().enumerate()
                {
                    pc_bytes[8 - push_size + i] = *push_byte;
                }
                second_branch_pc = Some(usize::from_be_bytes(pc_bytes));
            }
        }
        pc += 1;
    }

    match (first_branch_ic, second_branch_pc) {
            (Some(first_branch_ic), Some(second_branch_pc)) => Ok((
                    ItemAnchor {
                        item_id,
                        instruction: first_branch_ic,
                    },
                    ItemAnchor {
                        item_id,
                        instruction: *ic_map.get(&second_branch_pc).expect("Could not translate the program counter of the second branch to an instruction counter"),
                    }
            )),
            _ => eyre::bail!("Could not detect branches in source: {}", loc)
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
