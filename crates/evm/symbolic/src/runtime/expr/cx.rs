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

    pub(in crate::runtime::expr) fn make_expr(&mut self, expr: SymExprKind) -> SymExpr {
        SymExpr { kind: self.words.make(expr) }
    }

    pub(in crate::runtime) fn make_bytes(&mut self, bytes: SymBytesKind) -> SymBytes {
        if matches!(&bytes, SymBytesKind::Concrete(bytes) if bytes.is_empty()) {
            if let Some(bytes) = &self.cache.bytes_empty {
                return bytes.clone();
            }
            let bytes = SymBytes { kind: self.bytes.make(bytes) };
            self.cache.bytes_empty = Some(bytes.clone());
            return bytes;
        }
        SymBytes { kind: self.bytes.make(bytes) }
    }

    pub(crate) fn intern_expr(&mut self, expr: SymExpr) -> SymExpr {
        match expr.into_kind() {
            SymExprKind::Const(value) => self.constant(value),
            SymExprKind::Var(name) => self.var_symbol(name),
            SymExprKind::GasLeft(id) => self.gas_left(id),
            SymExprKind::Keccak { name, len, bytes } => {
                self.keccak_symbol(name, len, bytes.iter().cloned().collect())
            }
            SymExprKind::Hash { name, algorithm, bytes } => {
                self.hash_symbol(name, algorithm, bytes.iter().cloned().collect())
            }
            SymExprKind::Not(value) => self.not(value),
            SymExprKind::Op(op, left, right) => self.op(op, left, right),
            SymExprKind::AddMod { left, right, modulus } => self.addmod(left, right, modulus),
            SymExprKind::MulMod { left, right, modulus } => self.mulmod(left, right, modulus),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                self.ite(condition, then_expr, else_expr)
            }
        }
    }

    pub(crate) fn intern_bool(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.into_kind() {
            SymBoolExprKind::Const(value) => self.bool_constant(value),
            SymBoolExprKind::Not(value) => self.not_bool(value),
            SymBoolExprKind::And(values) => self.and(values.iter().cloned().collect()),
            SymBoolExprKind::Eq(left, right) => self.eq(left, right),
            SymBoolExprKind::Cmp(op, left, right) => self.cmp(op, left, right),
        }
    }

    pub(crate) fn zero(&mut self) -> SymExpr {
        self.constant(U256::ZERO)
    }

    pub(crate) fn one(&mut self) -> SymExpr {
        self.constant(U256::from(1))
    }

    pub(crate) fn constant(&mut self, value: U256) -> SymExpr {
        if value.is_zero() {
            if let Some(value) = &self.cache.zero {
                return value.clone();
            }
            let value = self.make_expr(SymExprKind::Const(value));
            self.cache.zero = Some(value.clone());
            return value;
        }
        if value == U256::from(1) {
            if let Some(value) = &self.cache.one {
                return value.clone();
            }
            let value = self.make_expr(SymExprKind::Const(value));
            self.cache.one = Some(value.clone());
            return value;
        }
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
        if value {
            if let Some(value) = &self.cache.bool_true {
                return value.clone();
            }
            let value = self.bool_from_kind(SymBoolExprKind::Const(true));
            self.cache.bool_true = Some(value.clone());
            value
        } else {
            if let Some(value) = &self.cache.bool_false {
                return value.clone();
            }
            let value = self.bool_from_kind(SymBoolExprKind::Const(false));
            self.cache.bool_false = Some(value.clone());
            value
        }
    }

    pub(crate) fn not(&mut self, value: SymExpr) -> SymExpr {
        let value = self.intern_expr(value);
        match value.kind() {
            SymExprKind::Const(value) => self.constant(!*value),
            SymExprKind::Not(value) => value.clone(),
            _ => self.make_expr(SymExprKind::Not(value)),
        }
    }

    pub(crate) fn op(&mut self, op: SymExprOp, left: SymExpr, right: SymExpr) -> SymExpr {
        let left = self.intern_expr(left);
        let right = self.intern_expr(right);
        match op {
            SymExprOp::Add => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::Sub => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ if left == right => self.zero(),
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::Mul => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    self.zero()
                }
                (SymExprKind::Const(value), _) if *value == U256::from(1) => right,
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => left,
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::UDiv | SymExprOp::SDiv => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => self.zero(),
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => left,
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::URem | SymExprOp::SRem => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => self.zero(),
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => self.zero(),
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::And => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    self.zero()
                }
                (SymExprKind::Const(value), _) if *value == U256::MAX => right,
                (_, SymExprKind::Const(value)) if *value == U256::MAX => left,
                _ if left == right => left,
                (SymExprKind::Const(mask), _) => self.and_const(right, *mask),
                (_, SymExprKind::Const(mask)) => self.and_const(left, *mask),
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::Or | SymExprOp::Xor => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::Shl | SymExprOp::Shr => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                (SymExprKind::Const(value), _) if value.is_zero() => self.zero(),
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
            SymExprOp::Sar => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    self.constant(op.eval(*left_value, *right_value))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ => self.make_expr(SymExprKind::Op(op, left, right)),
            },
        }
    }

    pub(crate) fn addmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        let left = self.intern_expr(left);
        let right = self.intern_expr(right);
        let modulus = self.intern_expr(modulus);
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus))
                if modulus.is_zero() || *modulus == U256::from(1) =>
            {
                self.zero()
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                self.constant(left.add_mod(*right, *modulus))
            }
            _ => self.make_expr(SymExprKind::AddMod { left, right, modulus }),
        }
    }

    pub(crate) fn mulmod(&mut self, left: SymExpr, right: SymExpr, modulus: SymExpr) -> SymExpr {
        let left = self.intern_expr(left);
        let right = self.intern_expr(right);
        let modulus = self.intern_expr(modulus);
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus))
                if modulus.is_zero() || *modulus == U256::from(1) =>
            {
                self.zero()
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                self.constant(left.mul_mod(*right, *modulus))
            }
            _ => self.make_expr(SymExprKind::MulMod { left, right, modulus }),
        }
    }

    pub(crate) fn ite(
        &mut self,
        condition: SymBoolExpr,
        then_expr: SymExpr,
        else_expr: SymExpr,
    ) -> SymExpr {
        let condition = self.intern_bool(condition);
        let then_expr = self.intern_expr(then_expr);
        let else_expr = self.intern_expr(else_expr);
        match condition.as_const() {
            Some(true) => then_expr,
            Some(false) => else_expr,
            None if then_expr == else_expr => then_expr,
            None => self.make_expr(SymExprKind::Ite(condition, then_expr, else_expr)),
        }
    }

    pub(crate) fn bool_word(&mut self, value: SymBoolExpr) -> SymExpr {
        let one = self.one();
        let zero = self.zero();
        self.ite(value, one, zero)
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
        let len = self.intern_expr(len);
        let bytes = bytes.into_iter().map(|byte| self.intern_expr(byte)).collect::<Vec<_>>();
        self.make_expr(SymExprKind::Keccak { name, len, bytes: bytes.into() })
    }

    pub(crate) fn hash_symbol(
        &mut self,
        name: Symbol,
        algorithm: &'static str,
        bytes: Vec<SymExpr>,
    ) -> SymExpr {
        let bytes = bytes.into_iter().map(|byte| self.intern_expr(byte)).collect::<Vec<_>>();
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
        let left = self.intern_expr(left);
        let right = self.intern_expr(right);
        match (left.kind(), right.kind()) {
            _ if left == right => self.bool_constant(true),
            (SymExprKind::Const(left), SymExprKind::Const(right)) => {
                self.bool_constant(left == right)
            }
            (_, SymExprKind::Const(right_value)) => {
                if let Some(condition) = self.bool_word_eq_const(&left, *right_value) {
                    return condition;
                }
                if let Some(left_value) = left.known_word() {
                    return self.bool_constant(left_value == *right_value);
                }
                self.bool_from_kind(SymBoolExprKind::Eq(left, right))
            }
            (SymExprKind::Const(left_value), _) => {
                if let Some(condition) = self.bool_word_eq_const(&right, *left_value) {
                    return condition;
                }
                if let Some(right_value) = right.known_word() {
                    return self.bool_constant(*left_value == right_value);
                }
                self.bool_from_kind(SymBoolExprKind::Eq(left, right))
            }
            (
                SymExprKind::Keccak { len: left_len, bytes: left_bytes, .. },
                SymExprKind::Keccak { len: right_len, bytes: right_bytes, .. },
            ) if left_bytes.len() == right_bytes.len() => {
                let mut conditions = vec![self.eq(left_len.clone(), right_len.clone())];
                conditions.extend(
                    left_bytes
                        .iter()
                        .cloned()
                        .zip(right_bytes.iter().cloned())
                        .map(|(left, right)| self.eq(left, right)),
                );
                self.and(conditions)
            }
            (
                SymExprKind::Hash { algorithm: left_algorithm, bytes: left_bytes, .. },
                SymExprKind::Hash { algorithm: right_algorithm, bytes: right_bytes, .. },
            ) if left_algorithm == right_algorithm && left_bytes.len() == right_bytes.len() => {
                let conditions = left_bytes
                    .iter()
                    .cloned()
                    .zip(right_bytes.iter().cloned())
                    .map(|(left, right)| self.eq(left, right))
                    .collect();
                self.and(conditions)
            }
            _ => self.bool_from_kind(SymBoolExprKind::Eq(left, right)),
        }
    }

    pub(crate) fn cmp(&mut self, op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> SymBoolExpr {
        let left = self.intern_expr(left);
        let right = self.intern_expr(right);
        match (op, left.kind(), right.kind()) {
            (op, _, _) if left == right => {
                self.bool_constant(matches!(op, SymBoolExprOp::Ule | SymBoolExprOp::Uge))
            }
            (op, SymExprKind::Const(left), SymExprKind::Const(right)) => {
                self.bool_constant(op.eval(*left, *right))
            }
            (SymBoolExprOp::Ugt, SymExprKind::Const(value), _) if value.is_zero() => {
                self.bool_constant(false)
            }
            (SymBoolExprOp::Ule, SymExprKind::Const(value), _) if value.is_zero() => {
                self.bool_constant(true)
            }
            (SymBoolExprOp::Ult, _, SymExprKind::Const(value)) if value.is_zero() => {
                self.bool_constant(false)
            }
            (SymBoolExprOp::Uge, _, SymExprKind::Const(value)) if value.is_zero() => {
                self.bool_constant(true)
            }
            (SymBoolExprOp::Ult, SymExprKind::Const(value), _) if *value == U256::MAX => {
                self.bool_constant(false)
            }
            (SymBoolExprOp::Uge, SymExprKind::Const(value), _) if *value == U256::MAX => {
                self.bool_constant(true)
            }
            (SymBoolExprOp::Ugt, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                self.bool_constant(false)
            }
            (SymBoolExprOp::Ule, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                self.bool_constant(true)
            }
            _ => self.bool_from_kind(SymBoolExprKind::Cmp(op, left, right)),
        }
    }

    pub(crate) fn and(&mut self, values: Vec<SymBoolExpr>) -> SymBoolExpr {
        let mut out = Vec::new();
        for value in values {
            let value = self.intern_bool(value);
            match value.kind() {
                SymBoolExprKind::Const(true) => {}
                SymBoolExprKind::Const(false) => return self.bool_constant(false),
                SymBoolExprKind::And(values) => out.extend(values.iter().cloned()),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            self.bool_constant(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            self.bool_from_kind(SymBoolExprKind::And(out.into()))
        }
    }

    pub(crate) fn or(&mut self, values: Vec<SymBoolExpr>) -> SymBoolExpr {
        let mut out = Vec::new();
        for value in values {
            let value = self.intern_bool(value);
            match value.kind() {
                SymBoolExprKind::Const(false) => {}
                SymBoolExprKind::Const(true) => return self.bool_constant(true),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            self.bool_constant(false)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            let values = out.into_iter().map(|value| self.not_bool(value)).collect();
            let and = self.and(values);
            self.not_bool(and)
        }
    }

    pub(crate) fn not_bool(&mut self, value: SymBoolExpr) -> SymBoolExpr {
        let value = self.intern_bool(value);
        match value.kind() {
            SymBoolExprKind::Const(value) => self.bool_constant(!*value),
            SymBoolExprKind::Not(value) => value.clone(),
            _ => self.bool_from_kind(SymBoolExprKind::Not(value)),
        }
    }

    fn and_const(&mut self, expr: SymExpr, mask: U256) -> SymExpr {
        if mask.is_zero() {
            return self.zero();
        }
        if mask == U256::MAX {
            return expr;
        }

        match expr.kind() {
            SymExprKind::Op(SymExprOp::And, left, right) => match (left.kind(), right.kind()) {
                (SymExprKind::Const(value), _) if *value == mask => {
                    self.and_const(right.clone(), mask)
                }
                (_, SymExprKind::Const(value)) if *value == mask => {
                    self.and_const(left.clone(), mask)
                }
                _ if left == right => self.and_const(left.clone(), mask),
                _ => {
                    let mask = self.constant(mask);
                    self.make_expr(SymExprKind::Op(SymExprOp::And, expr, mask))
                }
            },
            _ => {
                let mask = self.constant(mask);
                self.make_expr(SymExprKind::Op(SymExprOp::And, expr, mask))
            }
        }
    }

    fn bool_word_eq_const(&mut self, word: &SymExpr, value: U256) -> Option<SymBoolExpr> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = word.kind() else { return None };
        match (then_expr.as_const(), else_expr.as_const()) {
            (Some(then_value), Some(else_value))
                if then_value == U256::from(1) && else_value.is_zero() =>
            {
                Some(if value.is_zero() {
                    self.not_bool(condition.clone())
                } else if value == U256::from(1) {
                    condition.clone()
                } else {
                    self.bool_constant(false)
                })
            }
            (Some(then_value), Some(else_value))
                if then_value.is_zero() && else_value == U256::from(1) =>
            {
                Some(if value.is_zero() {
                    condition.clone()
                } else if value == U256::from(1) {
                    self.not_bool(condition.clone())
                } else {
                    self.bool_constant(false)
                })
            }
            _ => None,
        }
    }

    pub(in crate::runtime::expr) fn bool_from_kind(
        &mut self,
        expr: SymBoolExprKind,
    ) -> SymBoolExpr {
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
