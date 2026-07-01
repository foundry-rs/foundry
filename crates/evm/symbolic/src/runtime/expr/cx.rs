use super::{super::hashcons::HashCons, *};

pub(crate) struct SymCx {
    words: HashCons<SymExprKind>,
    bools: HashCons<SymBoolExprKind>,
    bytes: HashCons<SymBytesKind>,
    symbols: HashMap<Arc<str>, Symbol>,
    cache: SymCxCache,
}

#[derive(Default)]
struct SymCxCache {
    zero: Option<SymExpr>,
    one: Option<SymExpr>,
    bool_true: Option<SymBoolExpr>,
    bool_false: Option<SymBoolExpr>,
    bytes_empty: Option<SymBytes>,
}

impl SymCx {
    pub(crate) fn new() -> Self {
        Self {
            words: HashCons::new(),
            bools: HashCons::new(),
            bytes: HashCons::new(),
            symbols: HashMap::default(),
            cache: SymCxCache::default(),
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
            let table = &mut self.bytes;
            return self
                .cache
                .bytes_empty
                .get_or_insert_with(|| SymBytes { kind: table.make(bytes) })
                .clone();
        }
        SymBytes { kind: self.bytes.make(bytes) }
    }

    pub(in crate::runtime::expr) fn cached_zero(&mut self) -> SymExpr {
        let table = &mut self.words;
        self.cache
            .zero
            .get_or_insert_with(|| SymExpr { kind: table.make(SymExprKind::Const(U256::ZERO)) })
            .clone()
    }

    pub(in crate::runtime::expr) fn cached_one(&mut self) -> SymExpr {
        let table = &mut self.words;
        self.cache
            .one
            .get_or_insert_with(|| SymExpr { kind: table.make(SymExprKind::Const(U256::from(1))) })
            .clone()
    }

    pub(in crate::runtime::expr) fn cached_bool(&mut self, value: bool) -> SymBoolExpr {
        let table = &mut self.bools;
        if value {
            self.cache
                .bool_true
                .get_or_insert_with(|| SymBoolExpr {
                    kind: table.make(SymBoolExprKind::Const(true)),
                })
                .clone()
        } else {
            self.cache
                .bool_false
                .get_or_insert_with(|| SymBoolExpr {
                    kind: table.make(SymBoolExprKind::Const(false)),
                })
                .clone()
        }
    }

    pub(crate) fn intern_expr(&mut self, expr: SymExpr) -> SymExpr {
        match expr.into_kind() {
            SymExprKind::Const(value) => SymExpr::constant(self, value),
            SymExprKind::Var(name) => SymExpr::var_symbol(self, name),
            SymExprKind::GasLeft(id) => SymExpr::gas_left(self, id),
            SymExprKind::Keccak { name, len, bytes } => {
                SymExpr::keccak_symbol(self, name, len, bytes.iter().cloned().collect())
            }
            SymExprKind::Hash { name, algorithm, bytes } => {
                SymExpr::hash_symbol(self, name, algorithm, bytes.iter().cloned().collect())
            }
            SymExprKind::Not(value) => SymExpr::not(self, value),
            SymExprKind::Op(op, left, right) => SymExpr::op(self, op, left, right),
            SymExprKind::AddMod { left, right, modulus } => {
                SymExpr::addmod(self, left, right, modulus)
            }
            SymExprKind::MulMod { left, right, modulus } => {
                SymExpr::mulmod(self, left, right, modulus)
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                SymExpr::ite(self, condition, then_expr, else_expr)
            }
        }
    }

    pub(crate) fn intern_bool(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.into_kind() {
            SymBoolExprKind::Const(value) => SymBoolExpr::constant(self, value),
            SymBoolExprKind::Not(value) => SymBoolExpr::not_bool(self, value),
            SymBoolExprKind::And(values) => {
                SymBoolExpr::and(self, values.iter().cloned().collect())
            }
            SymBoolExprKind::Eq(left, right) => SymBoolExpr::eq(self, left, right),
            SymBoolExprKind::Cmp(op, left, right) => SymBoolExpr::cmp(self, op, left, right),
        }
    }

    pub(crate) fn zero(&mut self) -> SymExpr {
        SymExpr::zero(self)
    }

    pub(crate) fn one(&mut self) -> SymExpr {
        SymExpr::one(self)
    }

    pub(crate) fn constant(&mut self, value: U256) -> SymExpr {
        SymExpr::constant(self, value)
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

    pub(crate) fn var(&mut self, name: &str) -> SymExpr {
        SymExpr::var(self, name)
    }

    pub(crate) fn var_symbol(&mut self, name: Symbol) -> SymExpr {
        SymExpr::var_symbol(self, name)
    }

    pub(crate) fn gas_left(&mut self, id: usize) -> SymExpr {
        SymExpr::gas_left(self, id)
    }

    pub(crate) fn bool_constant(&mut self, value: bool) -> SymBoolExpr {
        SymBoolExpr::constant(self, value)
    }

    pub(crate) fn not(&mut self, value: SymExpr) -> SymExpr {
        SymExpr::not(self, value)
    }

    pub(crate) fn op(&mut self, op: SymExprOp, left: SymExpr, right: SymExpr) -> SymExpr {
        SymExpr::op(self, op, left, right)
    }

    pub(crate) fn addmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        SymExpr::addmod(self, left, right, modulus)
    }

    pub(crate) fn mulmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        SymExpr::mulmod(self, left, right, modulus)
    }

    pub(crate) fn ite(
        &mut self,
        condition: SymBoolExpr,
        then_expr: SymExpr,
        else_expr: SymExpr,
    ) -> SymExpr {
        SymExpr::ite(self, condition, then_expr, else_expr)
    }

    pub(crate) fn bool_word(&mut self, value: SymBoolExpr) -> SymExpr {
        SymExpr::bool_word(self, value)
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn from_bytes(&mut self, bytes: impl IntoIterator<Item = SymExpr>) -> SymExpr {
        SymExpr::from_bytes(self, bytes)
    }

    pub(crate) fn keccak_symbol(
        &mut self,
        name: Symbol,
        len: SymExpr,
        bytes: Vec<SymExpr>,
    ) -> SymExpr {
        SymExpr::keccak_symbol(self, name, len, bytes)
    }

    pub(crate) fn hash_symbol(
        &mut self,
        name: Symbol,
        algorithm: &'static str,
        bytes: Vec<SymExpr>,
    ) -> SymExpr {
        SymExpr::hash_symbol(self, name, algorithm, bytes)
    }

    pub(crate) fn cmp_word_const(
        &mut self,
        op: SymBoolExprOp,
        word: &SymExpr,
        value: U256,
    ) -> SymBoolExpr {
        SymBoolExpr::cmp_word_const(self, op, word, value)
    }

    pub(crate) fn eq_word_const(&mut self, word: &SymExpr, value: U256) -> SymBoolExpr {
        SymBoolExpr::eq_word_const(self, word, value)
    }

    pub(crate) fn eq(&mut self, left: SymExpr, right: SymExpr) -> SymBoolExpr {
        SymBoolExpr::eq(self, left, right)
    }

    pub(crate) fn cmp(&mut self, op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> SymBoolExpr {
        SymBoolExpr::cmp(self, op, left, right)
    }

    pub(crate) fn and(&mut self, values: Vec<SymBoolExpr>) -> SymBoolExpr {
        SymBoolExpr::and(self, values)
    }

    pub(crate) fn or(&mut self, values: Vec<SymBoolExpr>) -> SymBoolExpr {
        SymBoolExpr::or(self, values)
    }

    pub(crate) fn not_bool(&mut self, value: SymBoolExpr) -> SymBoolExpr {
        SymBoolExpr::not_bool(self, value)
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
        let first = cx.constant(U256::from(42));
        let second = cx.constant(U256::from(42));

        assert!(first.ptr_eq(&second));
    }

    #[test]
    fn hashconses_word_expressions() {
        let mut cx = SymCx::new();
        let x = cx.var("x");
        let y = cx.var("y");

        let first = cx.op(SymExprOp::Add, x.clone(), y.clone());
        let second = cx.op(SymExprOp::Add, x, y);

        assert!(first.ptr_eq(&second));
    }

    #[test]
    fn hashconses_bool_expressions() {
        let mut cx = SymCx::new();
        let x = cx.var("x");

        let upper = cx.constant(U256::from(7));
        let first = cx.cmp(SymBoolExprOp::Ult, x.clone(), upper.clone());
        let second = cx.cmp(SymBoolExprOp::Ult, x, upper);

        assert!(first.ptr_eq(&second));
    }
}
