use super::{hashcons::HashCons, *};
use alloy_primitives::map::DefaultHashBuilder;
use inturn::unsync::Interner;
use std::collections::BTreeMap;

pub(crate) struct SymCx {
    words: HashCons<SymExprKind>,
    bools: HashCons<SymBoolExprKind>,
    bytes: HashCons<SymBytesKind>,
    symbols: Interner<Symbol, DefaultHashBuilder>,
    replayable_inputs: SymbolicVars,
    /// Ordered so that the closest hash below a constant is a range lookup.
    concrete_keccak_preimages: BTreeMap<U256, Arc<[SymExpr]>>,
    cache: SymCxCache,
    /// Solver normalization results for `ite`-containing words, keyed by the input word.
    ///
    /// Keys are strong hash-consed handles, so the map is bounded like the SAT cache.
    solver_normalized_words: HashMap<SymExpr, SymExpr>,
    /// The `PUSH` immediates and the candidate mapping keys of each deployed contract with a
    /// storage layout.
    folded_slot_keys: Vec<(HashSet<U256>, Arc<[U256]>)>,
    /// Outcomes of searching a constant slot among the values of a mapping entry, keyed by both.
    folded_slot_searches: HashMap<(U256, SymExpr), Option<bool>>,
}

struct SymCxCache {
    zero: SymExpr,
    one: SymExpr,
    bool_true: SymBoolExpr,
    bool_false: SymBoolExpr,
    bytes_empty: SymBytes,
}

impl SymCx {
    pub(crate) fn new() -> Self {
        let mut words = HashCons::new();
        let zero = SymExpr { kind: words.make(SymExprKind::Const(U256::ZERO)) };
        let one = SymExpr { kind: words.make(SymExprKind::Const(U256::ONE)) };

        let mut bools = HashCons::new();
        let bool_true = SymBoolExpr { kind: bools.make(SymBoolExprKind::Const(true)) };
        let bool_false = SymBoolExpr { kind: bools.make(SymBoolExprKind::Const(false)) };

        let mut bytes = HashCons::new();
        let bytes_empty = SymBytes { kind: bytes.make(SymBytesKind::Concrete(Vec::new())) };

        Self {
            words,
            bools,
            bytes,
            symbols: Interner::with_hasher(DefaultHashBuilder::default()),
            replayable_inputs: SymbolicVars::default(),
            concrete_keccak_preimages: BTreeMap::new(),
            cache: SymCxCache { zero, one, bool_true, bool_false, bytes_empty },
            solver_normalized_words: HashMap::default(),
            folded_slot_keys: Vec::new(),
            folded_slot_searches: HashMap::default(),
        }
    }

    pub(in crate::runtime) fn mk_expr_kind(&mut self, expr: SymExprKind) -> SymExpr {
        SymExpr { kind: self.words.make(expr) }
    }

    pub(in crate::runtime) fn mk_bool_kind(&mut self, expr: SymBoolExprKind) -> SymBoolExpr {
        SymBoolExpr { kind: self.bools.make(expr) }
    }

    pub(in crate::runtime) fn mk_bytes_kind(&mut self, bytes: SymBytesKind) -> SymBytes {
        if matches!(&bytes, SymBytesKind::Concrete(bytes) if bytes.is_empty()) {
            return self.cache.bytes_empty.clone();
        }
        SymBytes { kind: self.bytes.make(bytes) }
    }

    pub(in crate::runtime::expr) fn cached_zero(&self) -> SymExpr {
        self.cache.zero.clone()
    }

    pub(in crate::runtime::expr) fn cached_one(&self) -> SymExpr {
        self.cache.one.clone()
    }

    pub(in crate::runtime::expr) fn cached_bool(&self, value: bool) -> SymBoolExpr {
        if value { self.cache.bool_true.clone() } else { self.cache.bool_false.clone() }
    }

    pub(crate) fn intern(&mut self, name: &str) -> Symbol {
        self.symbols.intern_mut(name)
    }

    pub(crate) fn symbol_name(&self, symbol: Symbol) -> &str {
        self.symbols.resolve(symbol)
    }

    pub(crate) fn mark_replayable_input(&mut self, symbol: Symbol) {
        self.replayable_inputs.insert(symbol);
    }

    pub(crate) fn is_replayable_input(&self, symbol: Symbol) -> bool {
        self.replayable_inputs.contains(&symbol)
    }

    pub(in crate::runtime) fn record_concrete_keccak_preimage(
        &mut self,
        hash: U256,
        bytes: Arc<[SymExpr]>,
    ) {
        self.concrete_keccak_preimages.entry(hash).or_insert(bytes);
    }

    pub(in crate::runtime::expr) fn concrete_keccak_preimage(
        &self,
        hash: U256,
    ) -> Option<Arc<[SymExpr]>> {
        self.concrete_keccak_preimages.get(&hash).cloned()
    }

    pub(in crate::runtime) fn add_folded_slot_keys(
        &mut self,
        constants: HashSet<U256>,
        keys: Vec<U256>,
    ) {
        self.folded_slot_keys.push((constants, keys.into()));
    }

    /// Returns the candidate keys of the contracts whose code pushes `slot`, or `None` when no
    /// such contract is known.
    pub(in crate::runtime::expr) fn folded_slot_keys(&self, slot: U256) -> Option<Vec<U256>> {
        let mut keys = self
            .folded_slot_keys
            .iter()
            .filter(|(constants, _)| constants.contains(&slot))
            .flat_map(|(_, keys)| keys.iter().copied())
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return None;
        }
        keys.sort_unstable();
        keys.dedup();
        Some(keys)
    }

    pub(in crate::runtime::expr) fn folded_slot_search(
        &self,
        slot: U256,
        entry: &SymExpr,
    ) -> Option<Option<bool>> {
        self.folded_slot_searches.get(&(slot, entry.clone())).copied()
    }

    pub(in crate::runtime::expr) fn record_folded_slot_search(
        &mut self,
        slot: U256,
        entry: SymExpr,
        found: Option<bool>,
    ) {
        self.folded_slot_searches.insert((slot, entry), found);
    }

    /// Returns the closest recorded concrete hash below `value` that `value` is a storage layout
    /// offset from, so a folded `keccak(..) + offset` constant keeps its hash base.
    pub(in crate::runtime) fn concrete_keccak_base(
        &self,
        value: U256,
        offset_bits: usize,
    ) -> Option<U256> {
        let (&hash, _) = self.concrete_keccak_preimages.range(..=value).next_back()?;
        (value - hash < U256::ONE << offset_bits).then_some(hash)
    }

    /// Returns the memoized solver normalization of `expr`.
    pub(in crate::runtime) fn solver_normalized_word(&self, expr: &SymExpr) -> Option<SymExpr> {
        self.solver_normalized_words.get(expr).cloned()
    }

    /// Memoizes the solver normalization of `expr`.
    ///
    /// A full map is cleared rather than frozen: a miss on a nested `ite` chain re-traverses it
    /// once per nesting level, so later words must stay cacheable.
    pub(in crate::runtime) fn record_solver_normalized_word(
        &mut self,
        expr: SymExpr,
        normalized: SymExpr,
    ) {
        if self.solver_normalized_words.len() >= SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES {
            self.solver_normalized_words.clear();
        }
        self.solver_normalized_words.insert(expr, normalized);
    }
}

impl fmt::Debug for SymCx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymCx").finish_non_exhaustive()
    }
}

impl Default for SymCx {
    fn default() -> Self {
        Self::new()
    }
}
