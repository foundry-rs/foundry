use super::*;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
pub(crate) fn normalize_constraints_for_solver(constraints: &[SymBoolExpr]) -> Vec<SymBoolExpr> {
    let normalized = normalize_constraint_batch(
        constraints.iter().cloned().map(normalize_bool_for_solver),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)) {
        return normalized;
    }

    let context = ConstraintContext::new(&normalized);
    let normalized_len = normalized.len();
    normalize_constraint_batch(
        normalized.into_iter().map(|constraint| context.normalize_bool(constraint)),
        normalized_len,
    )
}

fn normalize_constraint_batch(
    constraints: impl IntoIterator<Item = SymBoolExpr>,
    capacity: usize,
) -> Vec<SymBoolExpr> {
    let mut normalized = Vec::with_capacity(capacity);
    for constraint in constraints {
        if constraint.as_const() == Some(false) {
            return vec![SymBoolExpr::constant(false)];
        }
        constraint.push_normalized_conjuncts(&mut normalized);
    }
    sort_dedup_bool_exprs(&mut normalized);
    normalized
}

fn sort_dedup_bool_exprs(exprs: &mut Vec<SymBoolExpr>) {
    exprs.sort();
    exprs.dedup();
}

/// Returns a structural key for normalized solver cache lookups.
pub(super) fn constraint_cache_key(constraints: &[SymBoolExpr]) -> Vec<SymBoolExpr> {
    let mut key = Vec::with_capacity(constraints.len());
    for constraint in constraints.iter().cloned() {
        constraint.cache_key().push_cache_key_conjuncts(&mut key);
    }
    sort_dedup_bool_exprs(&mut key);
    key
}

/// Returns whether normalized conjunctive constraints contain a direct contradiction.
pub(super) fn constraints_are_directly_unsat(constraints: &[SymBoolExpr]) -> bool {
    constraints.iter().any(|constraint| match constraint.kind() {
        SymBoolExprKind::Const(false) => true,
        SymBoolExprKind::Not(inner) => constraints.binary_search(inner).is_ok(),
        _ => constraints.binary_search(&constraint.clone().not()).is_ok(),
    })
}

/// Returns whether every expression in sorted `subset` appears in sorted `superset`.
pub(super) fn sorted_bool_exprs_are_subset(
    subset: &[SymBoolExpr],
    superset: &[SymBoolExpr],
) -> bool {
    if subset.len() > superset.len() {
        return false;
    }

    let mut superset = superset.iter();
    for expected in subset {
        loop {
            match superset.next() {
                Some(candidate) if candidate < expected => {}
                Some(candidate) if candidate == expected => break,
                _ => return false,
            }
        }
    }
    true
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(expr: SymBoolExpr) -> SymBoolExpr {
    SolverNormalizer::default().normalize_bool(expr)
}

#[derive(Default)]
struct SolverNormalizer {
    exprs: HashMap<usize, (SymExpr, SymExpr)>,
    bools: HashMap<usize, (SymBoolExpr, SymBoolExpr)>,
}

impl SolverNormalizer {
    fn normalize_bool(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        let id = expr.node_id();
        if let Some((_, normalized)) = self.bools.get(&id) {
            return normalized.clone();
        }

        let original = expr.clone();
        let expr = match expr.into_kind() {
            SymBoolExprKind::Not(value) => self.normalize_bool(value).not(),
            SymBoolExprKind::And(values) => SymBoolExpr::and(
                values.iter().cloned().map(|value| self.normalize_bool(value)).collect(),
            ),
            SymBoolExprKind::Eq(left, right) => SymBoolExpr::eq(left, right),
            SymBoolExprKind::Cmp(op, left, right) => SymBoolExpr::cmp(op, left, right),
            SymBoolExprKind::Const(value) => SymBoolExpr::constant(value),
        };
        let normalized = self.normalize_bool_node(expr);
        self.bools.insert(id, (original, normalized.clone()));
        normalized
    }

    fn normalize_bool_exprs(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.into_kind() {
            SymBoolExprKind::Not(value) => self.normalize_bool_exprs(value).not(),
            SymBoolExprKind::And(values) => SymBoolExpr::and(
                values.iter().cloned().map(|value| self.normalize_bool_exprs(value)).collect(),
            ),
            SymBoolExprKind::Eq(left, right) => {
                SymBoolExpr::eq(self.normalize_expr(left), self.normalize_expr(right))
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                SymBoolExpr::cmp(op, self.normalize_expr(left), self.normalize_expr(right))
            }
            SymBoolExprKind::Const(value) => SymBoolExpr::constant(value),
        }
    }

    fn normalize_bool_node(&mut self, expr: SymBoolExpr) -> SymBoolExpr {
        if let Some(normalized) = expr.normalize_udiv_for_solver() {
            return normalized;
        }

        match expr.into_kind() {
            SymBoolExprKind::Not(value) => value.not(),
            SymBoolExprKind::And(values) => SymBoolExpr::and(values.iter().cloned().collect()),
            SymBoolExprKind::Eq(left, right) => {
                let normalized =
                    SymBoolExpr::eq(self.normalize_expr(left), self.normalize_expr(right));
                normalized.normalize_udiv_for_solver().unwrap_or(normalized)
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let normalized =
                    SymBoolExpr::cmp(op, self.normalize_expr(left), self.normalize_expr(right));
                normalized.normalize_udiv_for_solver().unwrap_or(normalized)
            }
            SymBoolExprKind::Const(value) => SymBoolExpr::constant(value),
        }
    }

    fn normalize_expr(&mut self, expr: SymExpr) -> SymExpr {
        let id = expr.node_id();
        if let Some((_, normalized)) = self.exprs.get(&id) {
            return normalized.clone();
        }

        let original = expr.clone();
        let expr = match expr.into_kind() {
            SymExprKind::Keccak { name, len, bytes } => SymExpr::keccak_symbol(
                name,
                self.normalize_expr(len),
                bytes.iter().cloned().map(|byte| self.normalize_expr(byte)).collect(),
            ),
            SymExprKind::Hash { name, algorithm, bytes } => SymExpr::hash_symbol(
                name,
                algorithm,
                bytes.iter().cloned().map(|byte| self.normalize_expr(byte)).collect(),
            ),
            SymExprKind::Not(value) => SymExpr::not(self.normalize_expr(value)),
            SymExprKind::Op(op, left, right) => {
                SymExpr::op(op, self.normalize_expr(left), self.normalize_expr(right))
            }
            SymExprKind::AddMod { left, right, modulus } => SymExpr::addmod(
                self.normalize_expr(left),
                self.normalize_expr(right),
                self.normalize_expr(modulus),
            ),
            SymExprKind::MulMod { left, right, modulus } => SymExpr::mulmod(
                self.normalize_expr(left),
                self.normalize_expr(right),
                self.normalize_expr(modulus),
            ),
            SymExprKind::Ite(condition, then_expr, else_expr) => SymExpr::ite(
                self.normalize_bool_exprs(condition),
                self.normalize_expr(then_expr),
                self.normalize_expr(else_expr),
            ),
            SymExprKind::Const(value) => SymExpr::constant(value),
            SymExprKind::Var(name) => SymExpr::var_symbol(name),
            SymExprKind::GasLeft(gas) => SymExpr::gas_left(gas),
        };
        let normalized = self.normalize_expr_node(expr);
        self.exprs.insert(id, (original, normalized.clone()));
        normalized
    }

    fn normalize_expr_node(&mut self, expr: SymExpr) -> SymExpr {
        if let Some(rebuilt) = expr.rebuild_from_extracted_byte_terms()
            && rebuilt != expr
        {
            return self.normalize_expr(rebuilt);
        }
        if let Some(rebuilt) = expr.rebuild_from_shifted_word_fragments()
            && rebuilt != expr
        {
            return self.normalize_expr(rebuilt);
        }
        if let Some(rebuilt) = expr.normalize_masked_shift_for_solver()
            && rebuilt != expr
        {
            return self.normalize_expr(rebuilt);
        }
        if let Some(rebuilt) = expr.normalize_masked_or_for_solver()
            && rebuilt != expr
        {
            return self.normalize_expr(rebuilt);
        }
        if let Some(rebuilt) = expr.normalize_shift_right_for_solver()
            && rebuilt != expr
        {
            return self.normalize_expr(rebuilt);
        }

        match expr.kind() {
            SymExprKind::Op(op, left, right)
                if matches!(
                    op,
                    SymExprOp::Add
                        | SymExprOp::Mul
                        | SymExprOp::And
                        | SymExprOp::Or
                        | SymExprOp::Xor
                ) && right < left =>
            {
                let SymExprKind::Op(op, left, right) = expr.into_kind() else { unreachable!() };
                SymExpr::op(op, right, left)
            }
            SymExprKind::Ite(_, _, _) => {
                let SymExprKind::Ite(cond, left, right) = expr.into_kind() else { unreachable!() };
                self.normalize_ite_expr(cond, left, right)
            }
            _ => expr,
        }
    }

    fn normalize_ite_expr(&mut self, cond: SymBoolExpr, left: SymExpr, right: SymExpr) -> SymExpr {
        let cond = self.normalize_bool(cond);
        if left == right {
            return left;
        }
        if let Some(condition) = guarded_self_div_word_condition(&cond, &left, &right) {
            return SymExpr::bool_word(condition);
        }
        if left.as_const() == Some(U256::from(1))
            && self.normalized_bool_word_condition(&right).as_ref() == Some(&cond)
        {
            return right;
        }
        if right.as_const().is_some_and(|value| value.is_zero())
            && self.normalized_bool_word_condition(&left).as_ref() == Some(&cond)
        {
            return left;
        }
        SymExpr::ite(cond, left, right)
    }

    fn normalized_bool_word_condition(&mut self, expr: &SymExpr) -> Option<SymBoolExpr> {
        expr.strip_low_byte_mask()
            .bool_word_condition()
            .map(|condition| self.normalize_bool(condition))
    }
}

impl SymBoolExpr {
    fn cache_key(self) -> Self {
        self.fold(&mut Self::cache_key_node)
    }

    fn cache_key_node(expr: Self) -> Self {
        match expr.into_kind() {
            SymBoolExprKind::Not(value) => value.not(),
            SymBoolExprKind::And(values) => {
                let mut conjuncts = Vec::new();
                for value in values.iter().cloned() {
                    value.push_cache_key_conjuncts(&mut conjuncts);
                }
                sort_dedup_bool_exprs(&mut conjuncts);
                Self::and(conjuncts)
            }
            SymBoolExprKind::Eq(left, right) => {
                let left = left.cache_key();
                let right = right.cache_key();
                if left <= right { Self::eq(left, right) } else { Self::eq(right, left) }
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::cache_key_cmp(op, left.cache_key(), right.cache_key())
            }
            SymBoolExprKind::Const(value) => Self::constant(value),
        }
    }

    fn push_cache_key_conjuncts(self, out: &mut Vec<Self>) {
        match self.kind() {
            SymBoolExprKind::Const(true) => {}
            SymBoolExprKind::And(values) => {
                for value in values.iter().cloned() {
                    value.push_cache_key_conjuncts(out);
                }
            }
            _ => out.push(self),
        }
    }

    fn cache_key_cmp(op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> Self {
        match op {
            SymBoolExprOp::Ugt => Self::cmp(SymBoolExprOp::Ult, right, left),
            SymBoolExprOp::Uge => Self::cmp(SymBoolExprOp::Ule, right, left),
            SymBoolExprOp::Sgt => Self::cmp(SymBoolExprOp::Slt, right, left),
            SymBoolExprOp::Ult | SymBoolExprOp::Ule | SymBoolExprOp::Slt => {
                Self::cmp(op, left, right)
            }
        }
    }

    fn push_normalized_conjuncts(self, out: &mut Vec<Self>) {
        match self.kind() {
            SymBoolExprKind::Const(true) => {}
            SymBoolExprKind::And(values) => {
                for value in values.iter().cloned() {
                    value.push_normalized_conjuncts(out);
                }
            }
            _ => out.push(self),
        }
    }
}

impl SymExpr {
    fn cache_key(self) -> Self {
        self.fold(&mut Self::cache_key_node)
    }

    fn cache_key_node(expr: Self) -> Self {
        match expr.kind() {
            SymExprKind::Op(op, left, right) => {
                if op.is_commutative() && right < left {
                    let SymExprKind::Op(op, left, right) = expr.into_kind() else { unreachable!() };
                    Self::op(op, right, left)
                } else {
                    expr
                }
            }
            SymExprKind::AddMod { left, right, .. } => {
                if right < left {
                    let SymExprKind::AddMod { left, right, modulus } = expr.into_kind() else {
                        unreachable!()
                    };
                    Self::addmod(right, left, modulus)
                } else {
                    expr
                }
            }
            SymExprKind::MulMod { left, right, .. } => {
                if right < left {
                    let SymExprKind::MulMod { left, right, modulus } = expr.into_kind() else {
                        unreachable!()
                    };
                    Self::mulmod(right, left, modulus)
                } else {
                    expr
                }
            }
            SymExprKind::Ite(_, _, _) => {
                let SymExprKind::Ite(cond, left, right) = expr.into_kind() else { unreachable!() };
                Self::ite(cond.cache_key(), left, right)
            }
            _ => expr,
        }
    }
}

impl SymExprOp {
    const fn is_commutative(self) -> bool {
        matches!(self, Self::Add | Self::Mul | Self::And | Self::Or | Self::Xor)
    }
}

pub(super) fn write_smt_assertions(out: &mut String, constraints: &[SymBoolExpr]) {
    if constraints.is_empty() {
        return;
    }

    let plan = SmtCsePlan::new(constraints);
    if plan.bindings.is_empty() {
        for constraint in constraints {
            let _ = writeln!(out, "(assert {})", constraint.smt());
        }
        return;
    }

    let writer = SmtCseWriter { plan: &plan };
    // define binding_0 = term_0
    // ...
    // define binding_n = term_n
    // assert constraint_0
    // ...
    // assert constraint_n
    for (idx, binding) in plan.bindings.iter().enumerate() {
        out.push_str("(define-fun ");
        binding.write_definition_header(out, idx);
        match binding {
            SmtBinding::Expr(expr) => writer.write_expr(out, expr, Some(idx), None),
            SmtBinding::Bool(expr) => writer.write_bool(out, expr, None, Some(idx)),
        }
        out.push_str(")\n");
    }
    for constraint in constraints {
        out.push_str("(assert ");
        writer.write_bool(out, constraint, None, None);
        out.push_str(")\n");
    }
}

#[derive(Default)]
struct SmtCseVisit {
    count: usize,
    binding: Option<usize>,
}

struct SmtCsePlan {
    expr_visits: HashMap<SymExpr, SmtCseVisit>,
    bool_visits: HashMap<SymBoolExpr, SmtCseVisit>,
    bindings: Vec<SmtBinding>,
}

impl SmtCsePlan {
    fn new(constraints: &[SymBoolExpr]) -> Self {
        let mut plan = Self {
            expr_visits: HashMap::default(),
            bool_visits: HashMap::default(),
            bindings: Vec::new(),
        };
        for constraint in constraints {
            plan.count_bool(constraint);
        }
        for constraint in constraints {
            plan.collect_bool_binding(constraint);
        }
        plan
    }

    fn count_expr(&mut self, expr: &SymExpr) {
        self.expr_visits.entry(expr.clone()).or_default().count += 1;
        match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => {}
            SymExprKind::Not(value) => self.count_expr(value),
            SymExprKind::Op(_, left, right) => {
                self.count_expr(left);
                self.count_expr(right);
            }
            SymExprKind::AddMod { left, right, modulus }
            | SymExprKind::MulMod { left, right, modulus } => {
                self.count_expr(modulus);
                self.count_expr(left);
                self.count_expr(right);
                self.count_expr(modulus);
            }
            SymExprKind::Ite(cond, left, right) => {
                self.count_bool(cond);
                self.count_expr(left);
                self.count_expr(right);
            }
        }
    }

    fn count_bool(&mut self, expr: &SymBoolExpr) {
        self.bool_visits.entry(expr.clone()).or_default().count += 1;
        match expr.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => self.count_bool(value),
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    self.count_bool(value);
                }
            }
            SymBoolExprKind::Eq(left, right) | SymBoolExprKind::Cmp(_, left, right) => {
                self.count_expr(left);
                self.count_expr(right);
            }
        }
    }

    fn collect_expr_binding(&mut self, expr: &SymExpr) {
        match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => {}
            SymExprKind::Not(value) => self.collect_expr_binding(value),
            SymExprKind::Op(_, left, right) => {
                self.collect_expr_binding(left);
                self.collect_expr_binding(right);
            }
            SymExprKind::AddMod { left, right, modulus }
            | SymExprKind::MulMod { left, right, modulus } => {
                self.collect_expr_binding(modulus);
                self.collect_expr_binding(left);
                self.collect_expr_binding(right);
            }
            SymExprKind::Ite(cond, left, right) => {
                self.collect_bool_binding(cond);
                self.collect_expr_binding(left);
                self.collect_expr_binding(right);
            }
        }
        self.bind_expr(expr);
    }

    fn collect_bool_binding(&mut self, expr: &SymBoolExpr) {
        match expr.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => self.collect_bool_binding(value),
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    self.collect_bool_binding(value);
                }
            }
            SymBoolExprKind::Eq(left, right) | SymBoolExprKind::Cmp(_, left, right) => {
                self.collect_expr_binding(left);
                self.collect_expr_binding(right);
            }
        }
        self.bind_bool(expr);
    }

    fn bind_expr(&mut self, expr: &SymExpr) {
        let Some(visit) = self.expr_visits.get_mut(expr) else { return };
        if visit.count <= 1 || visit.binding.is_some() || !Self::expr_can_bind(expr) {
            return;
        }
        let idx = self.bindings.len();
        visit.binding = Some(idx);
        self.bindings.push(SmtBinding::Expr(expr.clone()));
    }

    fn bind_bool(&mut self, expr: &SymBoolExpr) {
        let Some(visit) = self.bool_visits.get_mut(expr) else { return };
        if visit.count <= 1 || visit.binding.is_some() || !Self::bool_can_bind(expr) {
            return;
        }
        let idx = self.bindings.len();
        visit.binding = Some(idx);
        self.bindings.push(SmtBinding::Bool(expr.clone()));
    }

    fn expr_binding(&self, expr: &SymExpr) -> Option<usize> {
        self.expr_visits.get(expr).and_then(|visit| visit.binding)
    }

    fn bool_binding(&self, expr: &SymBoolExpr) -> Option<usize> {
        self.bool_visits.get(expr).and_then(|visit| visit.binding)
    }

    fn expr_can_bind(expr: &SymExpr) -> bool {
        !matches!(
            expr.kind(),
            SymExprKind::Const(_)
                | SymExprKind::Var(_)
                | SymExprKind::GasLeft(_)
                | SymExprKind::Keccak { .. }
                | SymExprKind::Hash { .. }
        )
    }

    fn bool_can_bind(expr: &SymBoolExpr) -> bool {
        !matches!(expr.kind(), SymBoolExprKind::Const(_))
    }
}

enum SmtBinding {
    Expr(SymExpr),
    Bool(SymBoolExpr),
}

impl SmtBinding {
    fn write_definition_header(&self, out: &mut String, idx: usize) {
        match self {
            Self::Expr(_) => {
                Self::write_expr_name(out, idx);
                out.push_str(" () (_ BitVec 256) ");
            }
            Self::Bool(_) => {
                Self::write_bool_name(out, idx);
                out.push_str(" () Bool ");
            }
        }
    }

    fn write_expr_name(out: &mut String, idx: usize) {
        let _ = write!(out, "__sym_expr_{idx}");
    }

    fn write_bool_name(out: &mut String, idx: usize) {
        let _ = write!(out, "__sym_bool_{idx}");
    }
}

struct SmtCseWriter<'a> {
    plan: &'a SmtCsePlan,
}

impl SmtCseWriter<'_> {
    fn write_expr(
        &self,
        out: &mut String,
        expr: &SymExpr,
        skip_expr: Option<usize>,
        skip_bool: Option<usize>,
    ) {
        if let Some(idx) = self.plan.expr_binding(expr)
            && Some(idx) != skip_expr
        {
            SmtBinding::write_expr_name(out, idx);
            return;
        }

        match expr.kind() {
            SymExprKind::Const(value) => {
                let _ = write!(out, "(_ bv{value} 256)");
            }
            SymExprKind::Var(var) => out.push_str(var.as_str()),
            SymExprKind::GasLeft(id) => {
                let _ = write!(out, "gasleft_{id}");
            }
            SymExprKind::Keccak { name, .. } => out.push_str(name.as_str()),
            SymExprKind::Hash { name, .. } => out.push_str(name.as_str()),
            SymExprKind::Not(value) => {
                out.push_str("(bvnot ");
                self.write_expr(out, value, skip_expr, skip_bool);
                out.push(')');
            }
            SymExprKind::Op(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                self.write_expr(out, left, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, right, skip_expr, skip_bool);
                out.push(')');
            }
            SymExprKind::AddMod { left, right, modulus } => {
                self.write_wide_modular_arithmetic(out, "bvadd", left, right, modulus);
            }
            SymExprKind::MulMod { left, right, modulus } => {
                self.write_wide_modular_arithmetic(out, "bvmul", left, right, modulus);
            }
            SymExprKind::Ite(cond, left, right) => {
                out.push_str("(ite ");
                self.write_bool(out, cond, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, left, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, right, skip_expr, skip_bool);
                out.push(')');
            }
        }
    }

    fn write_wide_modular_arithmetic(
        &self,
        out: &mut String,
        op: &'static str,
        left: &SymExpr,
        right: &SymExpr,
        modulus: &SymExpr,
    ) {
        // if modulus == 0:
        //   0
        // else:
        //   low_256((zext(left) op zext(right)) urem zext(modulus))
        out.push_str("(ite (= ");
        self.write_expr(out, modulus, None, None);
        out.push_str(" (_ bv0 256)) (_ bv0 256) ((_ extract 255 0) (bvurem (");
        out.push_str(op);
        out.push_str(" ((_ zero_extend 256) ");
        self.write_expr(out, left, None, None);
        out.push_str(") ((_ zero_extend 256) ");
        self.write_expr(out, right, None, None);
        out.push_str(")) ((_ zero_extend 256) ");
        self.write_expr(out, modulus, None, None);
        out.push_str("))))");
    }

    fn write_bool(
        &self,
        out: &mut String,
        expr: &SymBoolExpr,
        skip_expr: Option<usize>,
        skip_bool: Option<usize>,
    ) {
        if let Some(idx) = self.plan.bool_binding(expr)
            && Some(idx) != skip_bool
        {
            SmtBinding::write_bool_name(out, idx);
            return;
        }

        match expr.kind() {
            SymBoolExprKind::Const(value) => out.push_str(if *value { "true" } else { "false" }),
            SymBoolExprKind::Not(value) => {
                out.push_str("(not ");
                self.write_bool(out, value, skip_expr, skip_bool);
                out.push(')');
            }
            SymBoolExprKind::And(values) => {
                out.push_str("(and");
                for value in values.iter() {
                    out.push(' ');
                    self.write_bool(out, value, skip_expr, skip_bool);
                }
                out.push(')');
            }
            SymBoolExprKind::Eq(left, right) => {
                out.push_str("(= ");
                self.write_expr(out, left, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, right, skip_expr, skip_bool);
                out.push(')');
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                self.write_expr(out, left, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, right, skip_expr, skip_bool);
                out.push(')');
            }
        }
    }
}

/// Simple facts learned from the normalized conjunction currently being queried.
#[derive(Default)]
struct ConstraintContext {
    upper_bounds: HashMap<SymExpr, U256>,
}

impl ConstraintContext {
    fn new(constraints: &[SymBoolExpr]) -> Self {
        let mut context = Self::default();
        for constraint in constraints {
            context.record_upper_bound_constraint(constraint);
        }
        context
    }

    fn upper_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn normalize_bool(&self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.kind() {
            _ if expr.zero_check_operand().is_some_and(|left| self.word_bool_always_true(left)) => {
                SymBoolExpr::constant(false)
            }
            SymBoolExprKind::Not(value)
                if value
                    .zero_check_operand()
                    .is_some_and(|left| self.word_bool_always_true(left)) =>
            {
                SymBoolExpr::constant(true)
            }
            _ => expr,
        }
    }

    fn record_upper_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.upper_bound_constraint(constraint) {
            self.record_upper_bound(expr.clone(), bound);
        }
    }

    fn record_upper_bound(&mut self, expr: SymExpr, bound: U256) {
        self.upper_bounds
            .entry(expr)
            .and_modify(|existing| *existing = (*existing).min(bound))
            .or_insert(bound);
    }

    fn upper_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Eq(left, right) => match (left.as_const(), right.as_const()) {
                (_, Some(value)) => Some((left, value)),
                (Some(value), _) => Some((right, value)),
                _ => None,
            },
            SymBoolExprKind::Cmp(op, left, right) => {
                match (*op, left.as_const(), right.as_const()) {
                    (SymBoolExprOp::Ult, _, Some(bound)) => {
                        (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                    }
                    (SymBoolExprOp::Ule, _, Some(bound)) => Some((left, bound)),
                    (SymBoolExprOp::Ugt, Some(bound), _) => {
                        (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                    }
                    (SymBoolExprOp::Uge, Some(bound), _) => Some((right, bound)),
                    _ => None,
                }
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => {
                    match (*op, left.as_const(), right.as_const()) {
                        (SymBoolExprOp::Ugt, _, Some(bound)) => Some((left, bound)),
                        (SymBoolExprOp::Uge, _, Some(bound)) => {
                            (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                        }
                        (SymBoolExprOp::Ult, Some(bound), _) => Some((right, bound)),
                        (SymBoolExprOp::Ule, Some(bound), _) => {
                            (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(expr: SymExpr) -> SymExpr {
    SolverNormalizer::default().normalize_expr(expr)
}

/// Returns the boolean represented by `a == 0 ? 0 : a / a`.
fn guarded_self_div_word_condition(
    cond: &SymBoolExpr,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    if left.as_const().is_some_and(|value| value.is_zero())
        && self_div_expr_matches_zero_check(cond, right)
    {
        return Some(cond.clone().not());
    }
    None
}

/// Returns whether `expr` is `a / a` for the operand guarded by `cond`.
fn self_div_expr_matches_zero_check(cond: &SymBoolExpr, expr: &SymExpr) -> bool {
    let Some(zero_operand) = cond.zero_check_operand() else { return false };
    let Some((numerator, denominator)) = expr.udiv_operands() else { return false };
    numerator == zero_operand && denominator == zero_operand
}

impl SymExpr {
    fn rebuild_from_extracted_byte_terms(&self) -> Option<Self> {
        let mut terms = Vec::new();
        self.push_or_terms(&mut terms);
        if terms.len() <= 1 {
            return None;
        }

        let mut source = None;
        let mut seen = [false; 32];
        for term in terms {
            if term.as_const().is_some_and(|value| value.is_zero()) {
                continue;
            }
            let (term_source, index) = term.extracted_shifted_byte_term()?;
            match &source {
                Some(source) if source != &term_source => return None,
                Some(_) => {}
                None => source = Some(term_source),
            }
            seen[index] = true;
        }

        let source = source?;
        for (index, seen) in seen.into_iter().enumerate() {
            if !seen && source.known_byte(index) != Some(0) {
                return None;
            }
        }
        Some(source)
    }

    fn push_or_terms<'a>(&'a self, terms: &mut Vec<&'a Self>) {
        match self.kind() {
            SymExprKind::Op(SymExprOp::Or, left, right) => {
                left.push_or_terms(terms);
                right.push_or_terms(terms);
            }
            _ => terms.push(self),
        }
    }

    fn extracted_shifted_byte_term(&self) -> Option<(Self, usize)> {
        match self.kind() {
            SymExprKind::Op(SymExprOp::Shl, byte, shift) => {
                let shift = shift.as_const()?;
                let Ok(shift) = usize::try_from(shift) else { return None };
                if shift % 8 != 0 || shift > 248 {
                    return None;
                }
                let index = 31 - shift / 8;
                let source = byte.extracted_unshifted_byte_source(index)?;
                Some((source, index))
            }
            _ => self.extracted_unshifted_byte_source(31).map(|source| (source, 31)),
        }
    }

    fn extracted_unshifted_byte_source(&self, index: usize) -> Option<Self> {
        let expr = self.strip_low_byte_mask();
        if index == 31 {
            return Some(expr.clone());
        }
        let SymExprKind::Op(SymExprOp::Shr, source, shift) = expr.kind() else { return None };
        let shift = shift.as_const()?;
        (shift == U256::from((31 - index) * 8)).then(|| source.clone())
    }

    fn rebuild_from_shifted_word_fragments(&self) -> Option<Self> {
        let mut terms = Vec::new();
        self.push_or_terms(&mut terms);
        if terms.len() != 2 {
            return None;
        }

        let left_low = terms[0].low_word_fragment();
        let right_low = terms[1].low_word_fragment();
        let left_high = terms[0].shifted_high_word_fragment();
        let right_high = terms[1].shifted_high_word_fragment();
        match (left_low, right_low, left_high, right_high) {
            (Some((low_source, low_bits)), None, None, Some((high_source, high_bits)))
            | (None, Some((low_source, low_bits)), Some((high_source, high_bits)), None)
                if low_source == high_source && low_bits == high_bits =>
            {
                Some(low_source)
            }
            _ => None,
        }
    }

    fn low_word_fragment(&self) -> Option<(Self, usize)> {
        let SymExprKind::Op(SymExprOp::And, left, right) = self.kind() else { return None };
        if let Some(mask) = right.as_const() {
            return mask_low_bits(mask).map(|bits| (left.clone(), bits));
        }
        let mask = left.as_const()?;
        mask_low_bits(mask).map(|bits| (right.clone(), bits))
    }

    fn shifted_high_word_fragment(&self) -> Option<(Self, usize)> {
        let SymExprKind::Op(SymExprOp::Shl, value, shift) = self.kind() else { return None };
        let bits = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
        if bits == 0 || bits >= 256 {
            return None;
        }

        let (source, source_shift, width) = value.shifted_low_fragment_source()?;
        (source_shift == bits && width == 256 - bits).then_some((source, bits))
    }

    fn shifted_low_fragment_source(&self) -> Option<(Self, usize, usize)> {
        let SymExprKind::Op(SymExprOp::And, left, right) = self.kind() else { return None };
        if let Some(mask) = right.as_const() {
            return Self::shifted_low_fragment_source_with_mask(left, mask);
        }
        let mask = left.as_const()?;
        Self::shifted_low_fragment_source_with_mask(right, mask)
    }

    fn shifted_low_fragment_source_with_mask(
        value: &Self,
        mask: U256,
    ) -> Option<(Self, usize, usize)> {
        let width = mask_low_bits(mask)?;
        match value.kind() {
            SymExprKind::Op(SymExprOp::Shr, source, shift) => {
                let shift = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
                Some((source.clone(), shift, width))
            }
            _ => Some((value.clone(), 0, width)),
        }
    }

    fn normalize_masked_shift_for_solver(&self) -> Option<Self> {
        let SymExprKind::Op(SymExprOp::And, left, right) = self.kind() else { return None };
        let (value, mask) = if let Some(mask) = right.as_const() {
            (left, mask)
        } else {
            (right, left.as_const()?)
        };
        let mask_bits = mask_low_bits(mask)?;
        let SymExprKind::Op(SymExprOp::Shl, _, shift) = value.kind() else { return None };
        let shift = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
        (mask_bits <= shift).then(Self::zero)
    }

    fn normalize_masked_or_for_solver(&self) -> Option<Self> {
        let SymExprKind::Op(SymExprOp::And, left, right) = self.kind() else { return None };
        let (value, mask) = if let Some(mask) = right.as_const() {
            (left, mask)
        } else {
            (right, left.as_const()?)
        };
        let SymExprKind::Op(SymExprOp::Or, or_left, or_right) = value.kind() else { return None };

        let left = normalize_expr_for_solver(Self::op(
            SymExprOp::And,
            or_left.clone(),
            Self::constant(mask),
        ));
        if left.as_const().is_some_and(|value| value.is_zero()) {
            return Some(normalize_expr_for_solver(Self::op(
                SymExprOp::And,
                or_right.clone(),
                Self::constant(mask),
            )));
        }

        let right = normalize_expr_for_solver(Self::op(
            SymExprOp::And,
            or_right.clone(),
            Self::constant(mask),
        ));
        if right.as_const().is_some_and(|value| value.is_zero()) {
            return Some(left);
        }

        None
    }

    fn normalize_shift_right_for_solver(&self) -> Option<Self> {
        let SymExprKind::Op(SymExprOp::Shr, value, shift) = self.kind() else { return None };
        let shift = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
        if shift == 0 || shift >= 256 {
            return None;
        }
        if value.unsigned_bits() <= shift {
            return Some(Self::zero());
        }

        if let SymExprKind::Op(SymExprOp::Shl, inner, left_shift) = value.kind()
            && left_shift.as_const() == Some(U256::from(shift))
            && inner.unsigned_bits() <= 256 - shift
        {
            return Some(inner.clone());
        }

        let SymExprKind::Op(SymExprOp::Or, left, right) = value.kind() else { return None };
        let left = normalize_expr_for_solver(Self::op(
            SymExprOp::Shr,
            left.clone(),
            Self::constant(U256::from(shift)),
        ));
        if left.as_const().is_some_and(|value| value.is_zero()) {
            return Some(normalize_expr_for_solver(Self::op(
                SymExprOp::Shr,
                right.clone(),
                Self::constant(U256::from(shift)),
            )));
        }

        let right = normalize_expr_for_solver(Self::op(
            SymExprOp::Shr,
            right.clone(),
            Self::constant(U256::from(shift)),
        ));
        if right.as_const().is_some_and(|value| value.is_zero()) {
            return Some(left);
        }

        None
    }

    fn add_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().max(right.unsigned_bits()).saturating_add(1) <= 256
    }

    fn word_bool_always_true(&self) -> bool {
        ConstraintContext::default().word_bool_always_true(self)
    }

    pub(crate) fn mul_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().saturating_add(right.unsigned_bits()) <= 256
    }

    fn unsigned_bits(&self) -> usize {
        match self.kind() {
            SymExprKind::Const(value) => value.bit_len().max(1),
            SymExprKind::Op(SymExprOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    left.unsigned_bits().min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    right.unsigned_bits().min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                left.unsigned_bits().max(right.unsigned_bits()).saturating_add(1).min(256)
            }
            SymExprKind::Op(SymExprOp::Mul, left, right) => {
                left.unsigned_bits().saturating_add(right.unsigned_bits()).min(256)
            }
            SymExprKind::Op(SymExprOp::UDiv, left, _) => left.unsigned_bits(),
            SymExprKind::AddMod { modulus, .. } | SymExprKind::MulMod { modulus, .. } => {
                modulus.unsigned_bits()
            }
            SymExprKind::Ite(_, left, right) => left.unsigned_bits().max(right.unsigned_bits()),
            _ => 256,
        }
    }
}

fn mask_low_bits(mask: U256) -> Option<usize> {
    let bits = mask.bit_len();
    (mask == mask_bits(U256::MAX, bits)).then_some(bits)
}

impl SymBoolExpr {
    fn normalize_udiv_for_solver(&self) -> Option<Self> {
        match self.kind() {
            SymBoolExprKind::Eq(left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                left.normalized_bool_word_condition().map(Self::not).or_else(|| {
                    if left.word_bool_always_true() {
                        Some(Self::constant(false))
                    } else {
                        Self::normalize_udiv_eq_zero(left, &SymExpr::zero())
                    }
                })
            }
            SymBoolExprKind::Eq(left, right)
                if left.as_const().is_some_and(|value| value.is_zero()) =>
            {
                right.normalized_bool_word_condition().map(Self::not).or_else(|| {
                    if right.word_bool_always_true() {
                        Some(Self::constant(false))
                    } else {
                        Self::normalize_udiv_eq_zero(&SymExpr::zero(), right)
                    }
                })
            }
            SymBoolExprKind::Eq(left, right) if right.as_const() == Some(U256::from(1)) => {
                left.normalized_bool_word_condition()
            }
            SymBoolExprKind::Eq(left, right) if left.as_const() == Some(U256::from(1)) => {
                right.normalized_bool_word_condition()
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => {
                    Self::normalize_add_overflow_cmp(*op, left, right)
                        .map(Self::not)
                        .or_else(|| Self::normalize_udiv_cmp(*op, left, right).map(Self::not))
                }
                SymBoolExprKind::Eq(left, right)
                    if right.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if left.word_bool_always_true() {
                        Some(Self::constant(true))
                    } else {
                        Self::normalize_udiv_eq_zero(left, &SymExpr::zero()).map(Self::not)
                    }
                }
                SymBoolExprKind::Eq(left, right)
                    if left.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if right.word_bool_always_true() {
                        Some(Self::constant(true))
                    } else {
                        Self::normalize_udiv_eq_zero(&SymExpr::zero(), right).map(Self::not)
                    }
                }
                SymBoolExprKind::Eq(left, right) => {
                    Self::normalize_udiv_eq_zero(left, right).map(Self::not)
                }
                _ => None,
            },
            SymBoolExprKind::Eq(left, right) => Self::normalize_udiv_eq_zero(left, right),
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::normalize_add_overflow_cmp(*op, left, right)
                    .or_else(|| Self::normalize_udiv_cmp(*op, left, right))
            }
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn zero_check_operand(&self) -> Option<&SymExpr> {
        match self.kind() {
            SymBoolExprKind::Eq(left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                Some(left)
            }
            SymBoolExprKind::Eq(left, right)
                if left.as_const().is_some_and(|value| value.is_zero()) =>
            {
                Some(right)
            }
            _ => None,
        }
    }

    fn normalize_add_overflow_cmp(
        op: SymBoolExprOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        match op {
            SymBoolExprOp::Ugt if left.add_overflow_check(right) => Some(Self::constant(false)),
            SymBoolExprOp::Ult if right.add_overflow_check(left) => Some(Self::constant(false)),
            _ => None,
        }
    }

    fn normalize_udiv_eq_zero(left: &SymExpr, right: &SymExpr) -> Option<Self> {
        if right.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = left.normalize_eq_zero_for_solver()
        {
            return Some(condition);
        }
        if left.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = right.normalize_eq_zero_for_solver()
        {
            return Some(condition);
        }
        None
    }

    fn normalize_udiv_cmp(op: SymBoolExprOp, left: &SymExpr, right: &SymExpr) -> Option<Self> {
        match (op, left.as_const(), right.as_const()) {
            (SymBoolExprOp::Ugt, _, Some(value)) if value.is_zero() => {
                left.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Uge, _, Some(value)) if value == U256::from(1) => {
                left.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Ule, _, Some(value)) if value.is_zero() => {
                left.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ult, _, Some(value)) if value == U256::from(1) => {
                left.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ult, Some(value), _) if value.is_zero() => {
                right.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Ule, Some(value), _) if value == U256::from(1) => {
                right.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Uge, Some(value), _) if value.is_zero() => {
                right.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ugt, Some(value), _) if value == U256::from(1) => {
                right.normalize_eq_zero_for_solver()
            }
            _ => None,
        }
    }
}

impl SymExpr {
    fn normalized_bool_word_condition(&self) -> Option<SymBoolExpr> {
        self.strip_low_byte_mask().bool_word_condition().map(normalize_bool_for_solver)
    }

    fn add_overflow_check(&self, right: &Self) -> bool {
        let Some((base, increment)) = right.add_with_operand(self) else { return false };
        base == self && base.add_cannot_overflow_256(increment)
    }

    fn add_with_operand<'a>(&'a self, operand: &Self) -> Option<(&'a Self, &'a Self)> {
        let SymExprKind::Op(SymExprOp::Add, left, right) = self.kind() else { return None };
        if left == operand {
            Some((left, right))
        } else if right == operand {
            Some((right, left))
        } else {
            None
        }
    }

    fn normalize_eq_zero_for_solver(&self) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            return Some(Self::udiv_zero_condition(numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_zero = then_expr.normalize_eq_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(then_expr.clone()), Self::zero())
            });
            let else_zero = else_expr.normalize_eq_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(else_expr.clone()), Self::zero())
            });
            if then_zero.contains_udiv() || else_zero.contains_udiv() {
                return None;
            }
            let condition = normalize_bool_for_solver(condition.clone());
            return Some(SymBoolExpr::or(vec![
                SymBoolExpr::and(vec![condition.clone(), then_zero]),
                SymBoolExpr::and(vec![condition.not(), else_zero]),
            ]));
        }
        None
    }

    fn normalize_ne_zero_for_solver(&self) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            return Some(Self::udiv_nonzero_condition(numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_nonzero = then_expr.normalize_ne_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(then_expr.clone()), Self::zero()).not()
            });
            let else_nonzero = else_expr.normalize_ne_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(else_expr.clone()), Self::zero()).not()
            });
            if then_nonzero.contains_udiv() || else_nonzero.contains_udiv() {
                return None;
            }
            let condition = normalize_bool_for_solver(condition.clone());
            return Some(SymBoolExpr::or(vec![
                SymBoolExpr::and(vec![condition.clone(), then_nonzero]),
                SymBoolExpr::and(vec![condition.not(), else_nonzero]),
            ]));
        }
        None
    }

    fn udiv_operands(&self) -> Option<(&Self, &Self)> {
        match self.kind() {
            SymExprKind::Op(SymExprOp::UDiv, numerator, denominator) => {
                Some((numerator, denominator))
            }
            _ => None,
        }
    }

    fn udiv_zero_condition(numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(numerator.clone());
        let denominator = normalize_expr_for_solver(denominator.clone());
        SymBoolExpr::or(vec![
            SymBoolExpr::eq(denominator.clone(), Self::zero()),
            SymBoolExpr::cmp(SymBoolExprOp::Ult, numerator, denominator),
        ])
    }

    fn udiv_nonzero_condition(numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(numerator.clone());
        let denominator = normalize_expr_for_solver(denominator.clone());
        SymBoolExpr::and(vec![
            SymBoolExpr::eq(denominator.clone(), Self::zero()).not(),
            SymBoolExpr::cmp(SymBoolExprOp::Uge, numerator, denominator),
        ])
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, expr: &SymExpr) -> bool {
        let mut terms = Vec::new();
        expr.push_or_terms(&mut terms);
        if terms.len() <= 1 {
            return false;
        }

        let bool_terms = terms
            .iter()
            .filter_map(|term| term.normalized_bool_word_condition())
            .collect::<Vec<_>>();
        if bool_terms.iter().any(|term| {
            let negated = term.clone().not();
            bool_terms.contains(&negated)
        }) {
            return true;
        }
        for zero_term in &bool_terms {
            let Some(zero_operand) = zero_term.zero_check_operand() else { continue };
            if bool_terms.iter().any(|term| self.checked_mul_guard_for_operand(term, zero_operand))
            {
                return true;
            }
        }
        false
    }

    fn checked_mul_guard_for_operand(&self, expr: &SymBoolExpr, zero_operand: &SymExpr) -> bool {
        let SymBoolExprKind::Eq(left, right) = expr.kind() else { return false };
        self.checked_mul_guard_side(left, right, zero_operand)
            || self.checked_mul_guard_side(right, left, zero_operand)
    }

    fn checked_mul_guard_side(
        &self,
        div_expr: &SymExpr,
        expected: &SymExpr,
        zero_operand: &SymExpr,
    ) -> bool {
        let SymExprKind::Ite(condition, then_expr, else_expr) = div_expr.kind() else {
            return false;
        };
        if condition.zero_check_operand().is_none_or(|operand| operand != zero_operand) {
            return false;
        }
        if !then_expr.as_const().is_some_and(|value| value.is_zero()) {
            return false;
        }
        let Some((numerator, denominator)) = else_expr.udiv_operands() else { return false };
        if denominator != zero_operand {
            return false;
        }
        let SymExprKind::Op(SymExprOp::Mul, left, right) = numerator.kind() else {
            return false;
        };
        let other = if left == zero_operand {
            right
        } else if right == zero_operand {
            left
        } else {
            return false;
        };
        other == expected && self.mul_cannot_overflow_256(zero_operand, other)
    }

    fn mul_cannot_overflow_256(&self, left: &SymExpr, right: &SymExpr) -> bool {
        self.unsigned_bits(left).saturating_add(self.unsigned_bits(right)) <= 256
    }

    fn unsigned_bits(&self, expr: &SymExpr) -> usize {
        let bits = match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. }
            | SymExprKind::Not(_) => expr.unsigned_bits(),
            SymExprKind::Op(SymExprOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.unsigned_bits(left).min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    self.unsigned_bits(right).min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right)).saturating_add(1).min(256)
            }
            SymExprKind::Op(SymExprOp::Mul, left, right) => {
                self.unsigned_bits(left).saturating_add(self.unsigned_bits(right)).min(256)
            }
            SymExprKind::Op(SymExprOp::UDiv, left, _) => self.unsigned_bits(left),
            SymExprKind::Ite(_, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right))
            }
            _ => 256,
        };

        self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits)
    }
}
