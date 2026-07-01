use super::{hashcons::HashCons, *};

pub(crate) struct SymCx {
    words: HashCons<SymExprKind>,
    bools: HashCons<SymBoolExprKind>,
    bytes: HashCons<SymBytesKind>,
    symbols: HashMap<Arc<str>, Symbol>,
    cache: SymCxCache,
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
        let one = SymExpr { kind: words.make(SymExprKind::Const(U256::from(1))) };

        let mut bools = HashCons::new();
        let bool_true = SymBoolExpr { kind: bools.make(SymBoolExprKind::Const(true)) };
        let bool_false = SymBoolExpr { kind: bools.make(SymBoolExprKind::Const(false)) };

        let mut bytes = HashCons::new();
        let bytes_empty = SymBytes { kind: bytes.make(SymBytesKind::Concrete(Vec::new())) };

        Self {
            words,
            bools,
            bytes,
            symbols: HashMap::default(),
            cache: SymCxCache { zero, one, bool_true, bool_false, bytes_empty },
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
        if let Some(symbol) = self.symbols.get(name) {
            return symbol.clone();
        }
        let name = Arc::<str>::from(name);
        let symbol = Symbol::new(name.clone());
        self.symbols.insert(name, symbol.clone());
        symbol
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashconses_word_constants() {
        let mut cx = SymCx::new();
        let first = SymExpr::constant(&mut cx, U256::from(42));
        let second = SymExpr::constant(&mut cx, U256::from(42));

        assert!(first.ptr_eq(&second));
    }

    #[test]
    fn hashconses_word_expressions() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");

        let first = SymExpr::binop(&mut cx, SymExprBinOp::Add, x.clone(), y.clone());
        let second = SymExpr::binop(&mut cx, SymExprBinOp::Add, x, y);

        assert!(first.ptr_eq(&second));
    }

    #[test]
    fn hashconses_bool_expressions() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");

        let upper = SymExpr::constant(&mut cx, U256::from(7));
        let first = SymBoolExpr::cmp(&mut cx, SymBoolExprOp::Ult, x.clone(), upper.clone());
        let second = SymBoolExpr::cmp(&mut cx, SymBoolExprOp::Ult, x, upper);

        assert!(first.ptr_eq(&second));
    }

    #[test]
    fn simplifies_shift_right_over_or_at_construction() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x").low_byte(&mut cx);
        let low = SymExpr::constant(&mut cx, U256::from(0xff));
        let shift = SymExpr::constant(&mut cx, U256::from(8));
        let high = SymExpr::binop(&mut cx, SymExprBinOp::Shl, x.clone(), shift.clone());
        let word = SymExpr::binop(&mut cx, SymExprBinOp::Or, high, low);
        let shifted = SymExpr::binop(&mut cx, SymExprBinOp::Shr, word, shift);

        assert_eq!(shifted, x);
    }

    #[test]
    fn simplifies_masked_or_at_construction() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x").low_byte(&mut cx);
        let y = SymExpr::var(&mut cx, "y").low_byte(&mut cx);
        let shift = SymExpr::constant(&mut cx, U256::from(8));
        let high = SymExpr::binop(&mut cx, SymExprBinOp::Shl, x, shift);
        let word = SymExpr::binop(&mut cx, SymExprBinOp::Or, high, y.clone());
        let mask = SymExpr::constant(&mut cx, U256::from(0xff));
        let masked = SymExpr::binop(&mut cx, SymExprBinOp::And, word, mask);

        assert_eq!(masked, y);
    }
}
