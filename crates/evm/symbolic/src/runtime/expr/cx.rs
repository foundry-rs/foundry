use super::{hashcons::HashCons, *};

pub(crate) struct SymCx {
    words: HashCons<SymExprKind>,
    bools: HashCons<SymBoolExprKind>,
    symbols: HashMap<Arc<str>, Symbol>,
}

impl SymCx {
    pub(crate) fn new() -> Self {
        Self { words: HashCons::new(), bools: HashCons::new(), symbols: HashMap::default() }
    }

    fn make_expr(&mut self, expr: SymExprKind) -> SymExpr {
        SymExpr { kind: self.words.make(expr) }
    }

    pub(crate) fn intern_expr(&mut self, expr: SymExpr) -> SymExpr {
        match expr.into_kind() {
            SymExprKind::Const(value) => self.constant(value),
            SymExprKind::Var(name) => self.var_symbol(name),
            SymExprKind::GasLeft(id) => self.gas_left(id),
            SymExprKind::Keccak { name, len, bytes } => {
                let len = self.intern_expr(len);
                let bytes = bytes.iter().cloned().map(|byte| self.intern_expr(byte)).collect();
                self.keccak_symbol(name, len, bytes)
            }
            SymExprKind::Hash { name, algorithm, bytes } => {
                let bytes = bytes.iter().cloned().map(|byte| self.intern_expr(byte)).collect();
                self.hash_symbol(name, algorithm, bytes)
            }
            SymExprKind::Not(value) => {
                let value = self.intern_expr(value);
                self.make_expr(SymExprKind::Not(value))
            }
            SymExprKind::Op(op, left, right) => {
                let left = self.intern_expr(left);
                let right = self.intern_expr(right);
                self.make_expr(SymExprKind::Op(op, left, right))
            }
            SymExprKind::AddMod { left, right, modulus } => {
                let left = self.intern_expr(left);
                let right = self.intern_expr(right);
                let modulus = self.intern_expr(modulus);
                self.make_expr(SymExprKind::AddMod { left, right, modulus })
            }
            SymExprKind::MulMod { left, right, modulus } => {
                let left = self.intern_expr(left);
                let right = self.intern_expr(right);
                let modulus = self.intern_expr(modulus);
                self.make_expr(SymExprKind::MulMod { left, right, modulus })
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                let condition = self.intern_bool(condition);
                let then_expr = self.intern_expr(then_expr);
                let else_expr = self.intern_expr(else_expr);
                self.make_expr(SymExprKind::Ite(condition, then_expr, else_expr))
            }
        }
    }

    pub(crate) fn intern_bool(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.into_kind() {
            SymBoolExprKind::Const(value) => self.bool_constant(value),
            SymBoolExprKind::Not(value) => {
                let value = self.intern_bool(value);
                self.intern_bool_peepholed(value.not(), |kind| {
                    matches!(kind, SymBoolExprKind::Not(_))
                })
            }
            SymBoolExprKind::And(values) => {
                let values = values.iter().cloned().map(|value| self.intern_bool(value)).collect();
                self.intern_bool_peepholed(SymBoolExpr::and(values), |kind| {
                    matches!(kind, SymBoolExprKind::And(_))
                })
            }
            SymBoolExprKind::Eq(left, right) => {
                let left = self.intern_expr(left);
                let right = self.intern_expr(right);
                self.intern_bool_peepholed(SymBoolExpr::eq(left, right), |kind| {
                    matches!(kind, SymBoolExprKind::Eq(_, _))
                })
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let left = self.intern_expr(left);
                let right = self.intern_expr(right);
                self.intern_bool_peepholed(SymBoolExpr::cmp(op, left, right), |kind| {
                    matches!(kind, SymBoolExprKind::Cmp(_, _, _))
                })
            }
        }
    }

    fn intern_bool_peepholed(
        &mut self,
        expr: SymBoolExpr,
        terminal: impl FnOnce(&SymBoolExprKind) -> bool,
    ) -> SymBoolExpr {
        if terminal(expr.kind()) {
            self.bool_from_kind(expr.into_kind())
        } else {
            self.intern_bool(expr)
        }
    }

    pub(crate) fn zero(&mut self) -> SymExpr {
        self.constant(U256::ZERO)
    }

    pub(crate) fn constant(&mut self, value: U256) -> SymExpr {
        self.make_expr(SymExprKind::Const(value))
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
        let symbol = self.intern(name);
        self.var_symbol(symbol)
    }

    pub(crate) fn var_symbol(&mut self, name: Symbol) -> SymExpr {
        self.make_expr(SymExprKind::Var(name))
    }

    pub(crate) fn gas_left(&mut self, id: usize) -> SymExpr {
        self.make_expr(SymExprKind::GasLeft(id))
    }

    pub(crate) fn bool_constant(&mut self, value: bool) -> SymBoolExpr {
        self.bool_from_kind(SymBoolExprKind::Const(value))
    }

    pub(crate) fn not(&mut self, value: SymExpr) -> SymExpr {
        self.intern_expr(SymExpr::not(value))
    }

    pub(crate) fn op(&mut self, op: SymExprOp, left: SymExpr, right: SymExpr) -> SymExpr {
        self.intern_expr(SymExpr::op(op, left, right))
    }

    pub(crate) fn addmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        self.intern_expr(SymExpr::addmod(left, right, modulus))
    }

    pub(crate) fn mulmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        self.intern_expr(SymExpr::mulmod(left, right, modulus))
    }

    pub(crate) fn ite(
        &mut self,
        condition: SymBoolExpr,
        then_expr: SymExpr,
        else_expr: SymExpr,
    ) -> SymExpr {
        self.intern_expr(SymExpr::ite(condition, then_expr, else_expr))
    }

    pub(crate) fn bool_word(&mut self, value: SymBoolExpr) -> SymExpr {
        let one = self.constant(U256::from(1));
        let zero = self.zero();
        self.ite(value, one, zero)
    }

    pub(crate) fn keccak_symbol(
        &mut self,
        name: Symbol,
        len: SymExpr,
        bytes: Vec<SymExpr>,
    ) -> SymExpr {
        self.make_expr(SymExprKind::Keccak { name, len, bytes: bytes.into() })
    }

    pub(crate) fn hash_symbol(
        &mut self,
        name: Symbol,
        algorithm: &'static str,
        bytes: Vec<SymExpr>,
    ) -> SymExpr {
        self.make_expr(SymExprKind::Hash { name, algorithm, bytes: bytes.into() })
    }

    pub(crate) fn cmp_word_const(
        &mut self,
        op: SymBoolExprOp,
        word: &SymExpr,
        value: U256,
    ) -> SymBoolExpr {
        if let Some(word) = word.as_const() {
            self.bool_constant(op.eval(word, value))
        } else {
            let value = self.constant(value);
            self.cmp(op, word.clone(), value)
        }
    }

    pub(crate) fn eq_word_const(&mut self, word: &SymExpr, value: U256) -> SymBoolExpr {
        if let Some(word) = word.as_const() {
            self.bool_constant(word == value)
        } else {
            let value = self.constant(value);
            self.eq(word.clone(), value)
        }
    }

    pub(crate) fn eq(&mut self, left: SymExpr, right: SymExpr) -> SymBoolExpr {
        self.intern_bool(SymBoolExpr::eq(left, right))
    }

    pub(crate) fn cmp(&mut self, op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> SymBoolExpr {
        self.intern_bool(SymBoolExpr::cmp(op, left, right))
    }

    fn bool_from_kind(&mut self, expr: SymBoolExprKind) -> SymBoolExpr {
        SymBoolExpr { kind: self.bools.make(expr) }
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
