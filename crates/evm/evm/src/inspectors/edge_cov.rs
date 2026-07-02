use alloy_primitives::{
    Address, U256,
    map::{DefaultHashBuilder, Entry, HashMap},
};
use core::{
    fmt,
    hash::{BuildHasher, Hash, Hasher},
};
use revm::{
    Inspector,
    bytecode::opcode,
    context::{ContextTr, JournalTr},
    interpreter::{
        Interpreter,
        interpreter_types::{InputsTr, Jumps},
    },
};

// Default capacity for the hitcount buffer.
pub(crate) const MAX_EDGE_COUNT: usize = 65536;

// Maximum number of unique comparison sites to track for CmpLog-style feedback.
const MAX_CMP_LOG_SITES: usize = 1024;

// Maximum number of comparison operand pairs to track per site. This matches the downstream loop
// detection threshold so a hot loop can be classified without filling the whole log.
const MAX_CMP_OBSERVATIONS_PER_SITE: u8 = 8;

/// Edge coverage collection kind.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EdgeCovKind {
    /// Assign dense monotonically-increasing indices to unique edges.
    #[default]
    CollisionFree,
    /// Preserve the legacy fixed-size hash ID calculation.
    Hash,
}

/// Configuration for [`EdgeCovInspector`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EdgeCovConfig {
    /// Which edge coverage representation should be collected.
    pub kind: EdgeCovKind,
    /// Whether call-frame depth should be included in the edge identity.
    pub include_call_depth: bool,
}

impl EdgeCovConfig {
    /// Creates a new edge coverage configuration.
    pub const fn new(kind: EdgeCovKind, include_call_depth: bool) -> Self {
        Self { kind, include_call_depth }
    }

    /// Legacy fixed-size hash ID configuration.
    pub const fn legacy_hash_ids() -> Self {
        Self::new(EdgeCovKind::Hash, false)
    }
}

impl Default for EdgeCovConfig {
    fn default() -> Self {
        Self::new(EdgeCovKind::CollisionFree, false)
    }
}

impl From<&foundry_config::FuzzCorpusConfig> for EdgeCovConfig {
    fn from(corpus: &foundry_config::FuzzCorpusConfig) -> Self {
        let kind = if corpus.evm_edge_coverage_collision_free() {
            EdgeCovKind::CollisionFree
        } else {
            EdgeCovKind::Hash
        };
        Self::new(kind, corpus.evm_edge_coverage_include_call_depth())
    }
}

/// A comparison operand pair captured during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CmpOperands {
    /// First operand of the comparison.
    pub op1: U256,
    /// Second operand of the comparison.
    pub op2: U256,
    /// Program counter where the comparison occurred.
    pub pc: usize,
    /// Contract address where the comparison occurred.
    pub address: Address,
    /// EVM opcode that performed the comparison.
    pub opcode: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EdgeKey {
    pub address: Address,
    pub depth: Option<usize>,
    pub pc: usize,
    pub jump_dest: U256,
}

impl EdgeKey {
    fn new(
        address: Address,
        depth: usize,
        pc: usize,
        jump_dest: U256,
        include_depth: bool,
    ) -> Self {
        Self { address, depth: include_depth.then_some(depth), pc, jump_dest }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EdgeIndexMap {
    edge_indices: HashMap<EdgeKey, usize>,
    next_index: usize,
}

impl EdgeIndexMap {
    #[inline]
    pub fn edge_index(&mut self, edge: EdgeKey) -> usize {
        match self.edge_indices.entry(edge) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.next_index;
                self.next_index += 1;
                entry.insert(index);
                index
            }
        }
    }

    pub const fn edge_count(&self) -> usize {
        self.next_index
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EdgeCovHit {
    pub edge: EdgeKey,
    pub count: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EdgeCoverage {
    Hash(Vec<u8>),
    CollisionFree(Vec<EdgeCovHit>),
}

impl EdgeCoverage {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Hash(hitcount) => hitcount.iter().all(|&count| count == 0),
            Self::CollisionFree(hits) => hits.is_empty(),
        }
    }
}

/// An `Inspector` that tracks [edge coverage](https://clang.llvm.org/docs/SanitizerCoverage.html#edge-coverage).
/// Covered edges will not wrap to zero e.g. a loop edge hit more than 255 will still be retained.
///
/// Also tracks comparison operands for CmpLog-style guided fuzzing.
// see https://github.com/AFLplusplus/AFLplusplus/blob/5777ceaf23f48ae4ceae60e4f3a79263802633c6/instrumentation/afl-llvm-pass.so.cc#L810-L829
#[derive(Clone)]
pub struct EdgeCovInspector {
    /// Map of hitcounts that can be diffed against to determine if new coverage was reached.
    hitcount: Vec<u8>,
    /// Configuration for edge ID generation.
    config: EdgeCovConfig,
    /// Whether to collect edge hits. Comparison-only consumers keep this off to avoid affecting
    /// the active coverage guidance source.
    collect_edges: bool,
    /// Per-execution dense edge hitcounts. Stable IDs are assigned by the corpus history owner.
    dense_hitcount: HashMap<EdgeKey, u8>,
    hash_builder: DefaultHashBuilder,
    /// Comparison operand log for CmpLog-style guided fuzzing.
    cmp_log: Option<Vec<CmpOperands>>,
    cmp_site_counts: HashMap<CmpSiteKey, u8>,
}

impl fmt::Debug for EdgeCovInspector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EdgeCovInspector")
            .field("capacity", &self.hitcount.len())
            .field("edges", &self.edge_count())
            .field("config", &self.config)
            .field("collect_edges", &self.collect_edges)
            .finish()
    }
}

impl EdgeCovInspector {
    /// Create a new `EdgeCovInspector` with default configuration and capacity.
    pub fn new() -> Self {
        Self::with_config(EdgeCovConfig::default())
    }

    /// Create a new `EdgeCovInspector` with comparison operand logging enabled.
    pub fn with_cmp_log() -> Self {
        let mut inspector = Self::new();
        inspector.enable_cmp_log(true);
        inspector
    }

    /// Create a comparison-operand inspector without collecting EVM edge hits.
    pub fn with_cmp_log_only() -> Self {
        let mut inspector = Self::new();
        inspector.collect_edges = false;
        inspector.enable_cmp_log(true);
        inspector
    }

    /// Create a new `EdgeCovInspector` with the given configuration.
    ///
    /// [`EdgeCovKind::Hash`] preallocates a fixed-size bitmap;
    /// [`EdgeCovKind::CollisionFree`] grows its dense map on demand.
    pub fn with_config(config: EdgeCovConfig) -> Self {
        let hitcount = match config.kind {
            EdgeCovKind::Hash => vec![0; MAX_EDGE_COUNT],
            EdgeCovKind::CollisionFree => Vec::new(),
        };
        Self {
            hitcount,
            config,
            collect_edges: true,
            dense_hitcount: HashMap::default(),
            hash_builder: DefaultHashBuilder::default(),
            cmp_log: None,
            cmp_site_counts: HashMap::default(),
        }
    }

    /// Set whether to collect comparison operand logs.
    pub fn enable_cmp_log(&mut self, yes: bool) {
        if yes {
            self.cmp_log.get_or_insert_with(|| Vec::with_capacity(MAX_CMP_LOG_SITES));
        } else {
            self.cmp_log = None;
            self.cmp_site_counts.clear();
        }
    }

    /// Reset the hitcount to zero and clear the comparison log.
    pub fn reset(&mut self) {
        match self.config.kind {
            EdgeCovKind::CollisionFree => self.dense_hitcount.clear(),
            EdgeCovKind::Hash => self.hitcount.fill(0),
        }
        if let Some(cmp_log) = &mut self.cmp_log {
            cmp_log.clear();
        }
        self.cmp_site_counts.clear();
    }

    /// Get an immutable reference to the comparison operand log.
    pub const fn get_cmp_log(&self) -> &[CmpOperands] {
        match &self.cmp_log {
            Some(cmp_log) => cmp_log.as_slice(),
            None => &[],
        }
    }

    /// Consume the inspector and take ownership of both the hitcount and comparison log.
    pub fn into_parts(mut self) -> (EdgeCoverage, Vec<CmpOperands>) {
        let cmp_log = self.cmp_log.take().unwrap_or_default();
        (self.into(), cmp_log)
    }

    /// Number of unique collision-free edges discovered so far.
    pub fn edge_count(&self) -> usize {
        self.dense_hitcount.len()
    }

    /// Mark the edge `(address, depth, pc, jump_dest)` as hit.
    fn store_hit(&mut self, address: Address, depth: usize, pc: usize, jump_dest: U256) {
        if !self.collect_edges {
            return;
        }

        let edge_id = match self.config.kind {
            EdgeCovKind::CollisionFree => {
                self.store_dense_hit(address, depth, pc, jump_dest);
                return;
            }
            EdgeCovKind::Hash => self.hash_edge_id(address, depth, pc, jump_dest),
        };
        self.hitcount[edge_id] = self.hitcount[edge_id].wrapping_add(1).max(1);
    }

    fn store_dense_hit(&mut self, address: Address, depth: usize, pc: usize, jump_dest: U256) {
        let key = EdgeKey::new(address, depth, pc, jump_dest, self.config.include_call_depth);
        let count = self.dense_hitcount.entry(key).or_default();
        *count = count.wrapping_add(1).max(1);
    }

    fn hash_edge_id(
        &mut self,
        address: Address,
        depth: usize,
        pc: usize,
        jump_dest: U256,
    ) -> usize {
        let mut hasher = self.hash_builder.build_hasher();
        address.hash(&mut hasher);
        if self.config.include_call_depth {
            depth.hash(&mut hasher);
        }
        pc.hash(&mut hasher);
        jump_dest.hash(&mut hasher);
        // The hash is used to index into the hitcount array,
        // so it must be modulo the map size.
        (hasher.finish() % self.hitcount.len() as u64) as usize
    }

    #[cfg(test)]
    fn dense_hits(&self) -> Vec<EdgeCovHit> {
        let mut hits = self
            .dense_hitcount
            .iter()
            .map(|(&edge, &count)| EdgeCovHit { edge, count })
            .collect::<Vec<_>>();
        hits.sort_by_key(|hit| hit.edge);
        hits
    }

    /// Store comparison operands for CmpLog-style guided fuzzing.
    fn store_cmp(&mut self, cmp: CmpOperands) {
        let Some(cmp_log) = &mut self.cmp_log else {
            return;
        };

        let site = CmpSiteKey::new(&cmp);
        if let Some(count) = self.cmp_site_counts.get_mut(&site) {
            if *count >= MAX_CMP_OBSERVATIONS_PER_SITE {
                return;
            }
            *count += 1;
            cmp_log.push(cmp);
        } else if self.cmp_site_counts.len() < MAX_CMP_LOG_SITES {
            self.cmp_site_counts.insert(site, 1);
            cmp_log.push(cmp);
        }
    }

    #[cold]
    fn do_step<CTX>(&mut self, interp: &mut Interpreter, context: &mut CTX)
    where
        CTX: ContextTr,
    {
        let address = interp.input.target_address();
        let depth = context.journal_ref().depth();
        let current_pc = interp.bytecode.pc();

        match interp.bytecode.opcode() {
            opcode::JUMP => {
                // unconditional jump
                if let Ok(jump_dest) = interp.stack.peek(0) {
                    self.store_hit(address, depth, current_pc, jump_dest);
                }
            }
            opcode::JUMPI => {
                if let Ok(stack_value) = interp.stack.peek(1) {
                    let jump_dest = if stack_value.is_zero() {
                        // fall through
                        Ok(U256::from(current_pc + 1))
                    } else {
                        // branch taken
                        interp.stack.peek(0)
                    };

                    if let Ok(jump_dest) = jump_dest {
                        self.store_hit(address, depth, current_pc, jump_dest);
                    }
                }
            }
            _ => {
                // no-op
            }
        }
    }

    #[cold]
    fn do_cmp_step(&mut self, interp: &mut Interpreter) {
        if self.cmp_log.is_none() {
            return;
        }

        let address = interp.input.target_address();
        let current_pc = interp.bytecode.pc();

        match interp.bytecode.opcode() {
            op @ (opcode::EQ | opcode::LT | opcode::SLT | opcode::GT | opcode::SGT) => {
                if let (Ok(op1), Ok(op2)) = (interp.stack.peek(0), interp.stack.peek(1)) {
                    self.store_cmp(CmpOperands { op1, op2, pc: current_pc, address, opcode: op });
                }
            }
            op @ opcode::ISZERO => {
                if let Ok(op1) = interp.stack.peek(0) {
                    self.store_cmp(CmpOperands {
                        op1,
                        op2: U256::ZERO,
                        pc: current_pc,
                        address,
                        opcode: op,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Default for EdgeCovInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl From<EdgeCovInspector> for EdgeCoverage {
    fn from(inspector: EdgeCovInspector) -> Self {
        let EdgeCovInspector { hitcount, config, dense_hitcount, .. } = inspector;
        match config.kind {
            // Hits are deliberately not sorted here — this is the per-call drain
            // path and `merge_edge_coverage` doesn't care about order. Consumers
            // that need a deterministic order (e.g. `snapshot_edge_fingerprint`)
            // sort locally.
            EdgeCovKind::CollisionFree => Self::CollisionFree(
                dense_hitcount
                    .into_iter()
                    .map(|(edge, count)| EdgeCovHit { edge, count })
                    .collect(),
            ),
            EdgeCovKind::Hash => Self::Hash(hitcount),
        }
    }
}

impl<CTX> Inspector<CTX> for EdgeCovInspector
where
    CTX: ContextTr,
{
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, context: &mut CTX) {
        let op = interp.bytecode.opcode();
        if self.collect_edges && matches!(op, opcode::JUMP | opcode::JUMPI) {
            self.do_step(interp, context);
        }
        if self.cmp_log.is_some()
            && matches!(
                op,
                opcode::EQ | opcode::LT | opcode::GT | opcode::SLT | opcode::SGT | opcode::ISZERO
            )
        {
            self.do_cmp_step(interp);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CmpSiteKey {
    address: Address,
    pc: u32,
    opcode: u8,
}

impl CmpSiteKey {
    fn new(cmp: &CmpOperands) -> Self {
        debug_assert!(cmp.pc <= u32::MAX as usize);
        Self { address: cmp.address, pc: cmp.pc as u32, opcode: cmp.opcode }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dense_counts(inspector: &EdgeCovInspector) -> Vec<u8> {
        inspector.dense_hits().into_iter().map(|hit| hit.count).collect()
    }

    #[test]
    fn cmp_operands_defaults_and_clones() {
        let cmp = CmpOperands {
            op1: U256::from(123),
            op2: U256::from(456),
            pc: 42,
            address: Address::repeat_byte(0xaa),
            opcode: opcode::EQ,
        };

        assert_eq!(cmp.op1, U256::from(123));
        assert_eq!(cmp.op2, U256::from(456));
        assert_eq!(cmp.pc, 42);

        assert_eq!(CmpOperands::default(), CmpOperands::default());
        let cloned = cmp;
        assert_eq!(cloned, cmp);
    }

    #[test]
    fn cmp_log_starts_empty_and_is_returned_by_into_parts() {
        let inspector = EdgeCovInspector::new();

        assert!(inspector.get_cmp_log().is_empty());

        let (coverage, cmp_log) = inspector.into_parts();
        assert_eq!(coverage, EdgeCoverage::CollisionFree(Vec::new()));
        assert!(cmp_log.is_empty());
    }

    #[test]
    fn cmp_log_only_does_not_collect_edges() {
        let mut inspector = EdgeCovInspector::with_cmp_log_only();
        let addr = Address::ZERO;

        inspector.store_hit(addr, 0, 0, U256::from(1));
        inspector.store_cmp(CmpOperands {
            op1: U256::from(123),
            op2: U256::from(456),
            pc: 42,
            address: addr,
            opcode: opcode::EQ,
        });

        let (coverage, cmp_log) = inspector.into_parts();
        assert_eq!(coverage, EdgeCoverage::CollisionFree(Vec::new()));
        assert_eq!(cmp_log.len(), 1);
    }

    #[test]
    fn collision_free_ids() {
        let mut inspector = EdgeCovInspector::new();
        let addr = Address::ZERO;

        inspector.store_hit(addr, 0, 0, U256::from(10));
        inspector.store_hit(addr, 0, 0, U256::from(20));
        inspector.store_hit(addr, 0, 1, U256::from(10));

        assert_eq!(inspector.edge_count(), 3);
        assert_eq!(dense_counts(&inspector), [1, 1, 1]);
    }

    #[test]
    fn same_edge_increments_same_dense_slot() {
        let mut inspector = EdgeCovInspector::new();
        let addr = Address::ZERO;

        for _ in 0..5 {
            inspector.store_hit(addr, 0, 42, U256::from(100));
        }

        assert_eq!(inspector.edge_count(), 1);
        assert_eq!(dense_counts(&inspector), [5]);
    }

    #[test]
    fn edge_index_map_keeps_indices_stable() {
        let addr = Address::ZERO;
        let mut indices = EdgeIndexMap::default();
        let first = EdgeKey::new(addr, 0, 0, U256::from(10), false);
        let second = EdgeKey::new(addr, 0, 0, U256::from(20), false);

        assert_eq!(indices.edge_index(first), 0);
        assert_eq!(indices.edge_index(second), 1);
        assert_eq!(indices.edge_index(first), 0);
        assert_eq!(indices.edge_count(), 2);
    }

    #[test]
    fn hitcount_neverzero_on_wrap() {
        let mut inspector = EdgeCovInspector::new();
        let addr = Address::ZERO;

        for _ in 0..256 {
            inspector.store_hit(addr, 0, 0, U256::from(1));
        }

        assert_eq!(inspector.edge_count(), 1);
        assert_eq!(dense_counts(&inspector), [1]);
    }

    #[test]
    fn reset_clears_dense_hitcounts() {
        let mut inspector = EdgeCovInspector::new();
        let addr = Address::ZERO;

        inspector.store_hit(addr, 0, 0, U256::from(1));
        inspector.store_hit(addr, 0, 0, U256::from(2));
        assert_eq!(inspector.edge_count(), 2);
        assert_eq!(dense_counts(&inspector), [1, 1]);

        inspector.reset();
        assert_eq!(inspector.edge_count(), 0);
        assert!(inspector.dense_hits().is_empty());

        inspector.store_hit(addr, 0, 0, U256::from(1));
        assert_eq!(inspector.edge_count(), 1);
        assert_eq!(dense_counts(&inspector), [1]);
    }

    #[test]
    fn legacy_hash_ids_match_old_calculation() {
        let mut inspector = EdgeCovInspector::with_config(EdgeCovConfig::legacy_hash_ids());
        let addr = Address::ZERO;
        let pc = 42;
        let jump_dest = U256::from(100);

        let mut hasher = inspector.hash_builder.build_hasher();
        addr.hash(&mut hasher);
        pc.hash(&mut hasher);
        jump_dest.hash(&mut hasher);
        let expected_id = (hasher.finish() % MAX_EDGE_COUNT as u64) as usize;

        inspector.store_hit(addr, 0, pc, jump_dest);

        assert_eq!(inspector.hitcount[expected_id], 1);
        assert_eq!(inspector.hitcount.iter().filter(|&&count| count != 0).count(), 1);
    }

    #[test]
    fn call_depth_option_delineates_same_edge() {
        let addr = Address::ZERO;

        let mut without_depth = EdgeCovInspector::new();
        without_depth.store_hit(addr, 0, 0, U256::from(1));
        without_depth.store_hit(addr, 1, 0, U256::from(1));
        assert_eq!(without_depth.edge_count(), 1);
        assert_eq!(dense_counts(&without_depth), [2]);

        let mut with_depth =
            EdgeCovInspector::with_config(EdgeCovConfig::new(EdgeCovKind::CollisionFree, true));
        with_depth.store_hit(addr, 0, 0, U256::from(1));
        with_depth.store_hit(addr, 1, 0, U256::from(1));
        assert_eq!(with_depth.edge_count(), 2);
        assert_eq!(dense_counts(&with_depth), [1, 1]);
    }

    #[test]
    fn reset_clears_hitcount_and_cmp_log() {
        let mut inspector = EdgeCovInspector::with_cmp_log();

        inspector.store_hit(Address::ZERO, 0, 0, U256::from(1));
        inspector.store_cmp(CmpOperands {
            op1: U256::from(123),
            op2: U256::from(456),
            pc: 42,
            address: Address::ZERO,
            opcode: opcode::EQ,
        });

        inspector.reset();

        assert!(inspector.dense_hits().is_empty());
        assert!(inspector.get_cmp_log().is_empty());
    }

    #[test]
    fn cmp_log_is_capped_per_site() {
        let mut inspector = EdgeCovInspector::with_cmp_log();

        for i in 0..usize::from(MAX_CMP_OBSERVATIONS_PER_SITE) + 1 {
            inspector.store_cmp(CmpOperands {
                op1: U256::from(i),
                op2: U256::from(i + 1),
                pc: 42,
                address: Address::ZERO,
                opcode: opcode::EQ,
            });
        }

        assert_eq!(inspector.get_cmp_log().len(), usize::from(MAX_CMP_OBSERVATIONS_PER_SITE));
    }

    #[test]
    fn cmp_log_caps_sites_without_starving_later_observations() {
        let mut inspector = EdgeCovInspector::with_cmp_log();

        for i in 0..usize::from(MAX_CMP_OBSERVATIONS_PER_SITE) * 2 {
            inspector.store_cmp(CmpOperands {
                op1: U256::from(i),
                op2: U256::from(i + 1),
                pc: 1,
                address: Address::ZERO,
                opcode: opcode::EQ,
            });
        }
        inspector.store_cmp(CmpOperands {
            op1: U256::from(123),
            op2: U256::from(456),
            pc: 2,
            address: Address::ZERO,
            opcode: opcode::EQ,
        });

        assert_eq!(inspector.get_cmp_log().len(), usize::from(MAX_CMP_OBSERVATIONS_PER_SITE) + 1);
        assert_eq!(inspector.get_cmp_log().last().unwrap().pc, 2);
    }
}
