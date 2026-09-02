use super::*;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
#[cfg(test)]
pub(crate) fn normalize_constraints_for_solver(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
) -> Vec<SymBoolExpr> {
    normalize_constraints_for_solver_with(cx, constraints, |cx, constraint| {
        normalize_bool_for_solver(cx, constraint.clone())
    })
}

/// Reuses context-free normalization results while retaining per-query contextual rewrites.
pub(super) fn normalize_constraints_for_solver_cached(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    normalization_cache: &mut HashMap<SymBoolExpr, SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    normalize_constraints_for_solver_with(cx, constraints, |cx, constraint| {
        if let Some(normalized) = normalization_cache.get(constraint) {
            return normalized.clone();
        }
        let normalized = normalize_bool_for_solver(cx, constraint.clone());
        // These are strong hash-consed handles, so bound their lifetime like the SAT cache.
        if normalization_cache.len() < SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES {
            normalization_cache.insert(constraint.clone(), normalized.clone());
        }
        normalized
    })
}

fn normalize_constraints_for_solver_with(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    mut normalize: impl FnMut(&mut SymCx, &SymBoolExpr) -> SymBoolExpr,
) -> Vec<SymBoolExpr> {
    let normalized = normalize_constraint_batch(
        constraints.iter().map(|constraint| normalize(cx, constraint)),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)) {
        return normalized;
    }

    // Context-dependent rewrites must not contribute facts to the context that proves them. Mark
    // removable candidates by syntax rather than by whether the full context happens to prove a
    // rewrite: contradictory bounds can make an interval unavailable until another candidate is
    // removed. Rewrites to `false` remain in the context because they terminate the conjunction
    // rather than dropping a fact.
    let retained_count = normalized
        .iter()
        .filter(|constraint| !ConstraintContext::could_contextually_disappear(constraint))
        .count();
    let retained = normalized
        .iter()
        .filter(|constraint| !ConstraintContext::could_contextually_disappear(constraint));
    let context = ConstraintContext::from_constraints(retained, retained_count);
    let normalized_len = normalized.len();
    normalize_constraint_batch(
        normalized.into_iter().map(|constraint| context.normalize_bool(cx, constraint)),
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
            return vec![constraint];
        }
        constraint.push_normalized_conjuncts(&mut normalized);
    }
    sort_dedup_bool_exprs(&mut normalized);
    normalized
}

fn sort_dedup_bool_exprs(exprs: &mut Vec<SymBoolExpr>) {
    // Hash-consing already caches deterministic structural hashes. Only render full structural
    // keys for the exceedingly rare case where two distinct expressions collide.
    exprs.sort_unstable_by(bool_expr_cmp);
    exprs.dedup();
}

fn bool_expr_cmp(left: &SymBoolExpr, right: &SymBoolExpr) -> std::cmp::Ordering {
    if left == right {
        return std::cmp::Ordering::Equal;
    }
    left.stable_hash_cmp(right)
        .then_with(|| bool_structural_key(left).cmp(&bool_structural_key(right)))
}

fn bool_structural_key(expr: &SymBoolExpr) -> String {
    let mut key = String::new();
    write_bool_structural_key(&mut key, expr);
    key
}

fn write_bool_structural_key(out: &mut String, expr: &SymBoolExpr) {
    match expr.kind() {
        SymBoolExprKind::Const(value) => {
            let _ = write!(out, "0:{value}");
        }
        SymBoolExprKind::Not(value) => {
            out.push_str("1:");
            write_bool_structural_key(out, value);
        }
        SymBoolExprKind::And(values) => {
            let _ = write!(out, "2:{}:", values.len());
            for value in values.iter() {
                write_bool_structural_key(out, value);
                out.push(';');
            }
        }
        SymBoolExprKind::Cmp(op, left, right) => {
            let _ = write!(out, "3:{}:", cmp_op_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
        }
    }
}

fn write_expr_structural_key(out: &mut String, expr: &SymExpr) {
    match expr.kind() {
        SymExprKind::Const(value) => {
            let _ = write!(out, "0:{value:064x}");
        }
        SymExprKind::Var(name) => {
            let _ = write!(out, "1:{}", name.id());
        }
        SymExprKind::GasLeft(symbol) => {
            let _ = write!(out, "2:{}", symbol.id());
        }
        SymExprKind::Keccak { name, len, bytes } => {
            let _ = write!(out, "3:{}:", name.id());
            write_expr_structural_key(out, len);
            write_exprs_structural_key(out, bytes);
        }
        SymExprKind::Hash { name, algorithm, bytes } => {
            let _ = write!(out, "4:{}:{algorithm}:", name.id());
            write_exprs_structural_key(out, bytes);
        }
        SymExprKind::Not(value) => {
            out.push_str("5:");
            write_expr_structural_key(out, value);
        }
        SymExprKind::BinOp(op, left, right) => {
            let _ = write!(out, "6:{}:", expr_binop_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
        }
        SymExprKind::TernOp(op, left, right, modulus) => {
            let _ = write!(out, "7:{}:", expr_ternop_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
            out.push(':');
            write_expr_structural_key(out, modulus);
        }
        SymExprKind::Ite(condition, then_expr, else_expr) => {
            out.push_str("9:");
            write_bool_structural_key(out, condition);
            out.push(':');
            write_expr_structural_key(out, then_expr);
            out.push(':');
            write_expr_structural_key(out, else_expr);
        }
    }
}

fn write_exprs_structural_key(out: &mut String, exprs: &[SymExpr]) {
    let _ = write!(out, "{}:", exprs.len());
    for expr in exprs {
        write_expr_structural_key(out, expr);
        out.push(';');
    }
}

const fn cmp_op_key(op: SymCmpOp) -> u8 {
    match op {
        SymCmpOp::Eq => 0,
        SymCmpOp::Ult => 1,
        SymCmpOp::Ugt => 2,
        SymCmpOp::Ule => 3,
        SymCmpOp::Uge => 4,
        SymCmpOp::Slt => 5,
        SymCmpOp::Sgt => 6,
    }
}

const fn expr_binop_key(op: SymBinOp) -> u8 {
    match op {
        SymBinOp::Add => 0,
        SymBinOp::Sub => 1,
        SymBinOp::Mul => 2,
        SymBinOp::UDiv => 3,
        SymBinOp::URem => 4,
        SymBinOp::SDiv => 5,
        SymBinOp::SRem => 6,
        SymBinOp::And => 7,
        SymBinOp::Or => 8,
        SymBinOp::Xor => 9,
        SymBinOp::Shl => 10,
        SymBinOp::Shr => 11,
        SymBinOp::Sar => 12,
    }
}

const fn expr_ternop_key(op: SymTernOp) -> u8 {
    match op {
        SymTernOp::AddMod => 0,
        SymTernOp::MulMod => 1,
    }
}

/// Returns whether canonically ordered normalized constraints contain a direct contradiction.
pub(super) fn constraints_are_directly_unsat(cx: &mut SymCx, constraints: &[SymBoolExpr]) -> bool {
    let mut derived = Vec::new();
    for constraint in constraints {
        let Some(fact) = bitwise_bool_word_fact(cx, constraint) else { continue };
        if let SymBoolExprKind::And(values) = fact.kind() {
            // A positive conjunction implies each member independently. Retain the aggregate for
            // exact matches, but expose its members to the direct contradiction check as well.
            derived.extend(values.iter().cloned());
        }
        derived.push(fact);
    }
    let contains = |expected: &SymBoolExpr| {
        constraints.binary_search_by(|candidate| bool_expr_cmp(candidate, expected)).is_ok()
            || derived.contains(expected)
    };
    constraints.iter().chain(&derived).any(|constraint| match constraint.kind() {
        SymBoolExprKind::Const(false) => true,
        SymBoolExprKind::Not(inner)
            if let SymBoolExprKind::And(values) = inner.kind()
                && values.iter().all(&contains) =>
        {
            true
        }
        SymBoolExprKind::Not(inner) => contains(inner),
        _ => {
            let negated = constraint.clone().not(cx);
            contains(&negated)
        }
    })
}

fn bitwise_bool_word_fact(cx: &mut SymCx, constraint: &SymBoolExpr) -> Option<SymBoolExpr> {
    match constraint.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
            if right.as_const().is_some_and(|value| value.is_zero()) =>
        {
            left.bitwise_bool_word_condition(cx).map(|condition| condition.not(cx))
        }
        SymBoolExprKind::Not(inner) => {
            let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = inner.kind() else { return None };
            if !right.as_const().is_some_and(|value| value.is_zero()) {
                return None;
            }
            left.bitwise_bool_word_condition(cx)
        }
        _ => None,
    }
}

/// Returns whether every expression in `subset` appears in `superset`.
pub(super) fn sorted_bool_exprs_are_subset(
    subset: &[SymBoolExpr],
    superset: &[SymBoolExpr],
) -> bool {
    if subset.len() > superset.len() {
        return false;
    }

    let superset: HashSet<_> = superset.iter().collect();
    subset.iter().all(|expected| superset.contains(expected))
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
    expr.fold(cx, &mut normalize_bool_node_for_solver)
}

impl SymBoolExpr {
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

pub(super) fn write_smt_assertions(
    cx: &SymCx,
    out: &mut String,
    constraints: &[SymBoolExpr],
) -> Result<(), SymbolicError> {
    if constraints.is_empty() {
        return Ok(());
    }
    if constraints.iter().any(SymBoolExpr::contains_gasleft) {
        return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
    }

    let plan = SmtCsePlan::new(constraints);
    if plan.bindings.is_empty() {
        for constraint in constraints {
            let _ = writeln!(out, "(assert {})", constraint.smt(cx));
        }
        return Ok(());
    }

    let writer = SmtCseWriter { cx, plan: &plan };
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
    Ok(())
}

#[derive(Default)]
struct SmtCseVisit {
    count: usize,
    binding: Option<usize>,
    collected: bool,
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
        let visit = self.expr_visits.entry(expr.clone()).or_default();
        visit.count += 1;
        if visit.count != 1 {
            return;
        }
        match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => {}
            SymExprKind::Not(value) => self.count_expr(value),
            SymExprKind::BinOp(_, left, right) => {
                self.count_expr(left);
                self.count_expr(right);
            }
            SymExprKind::TernOp(_, left, right, modulus) => {
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
        let visit = self.bool_visits.entry(expr.clone()).or_default();
        visit.count += 1;
        if visit.count != 1 {
            return;
        }
        match expr.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => self.count_bool(value),
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    self.count_bool(value);
                }
            }
            SymBoolExprKind::Cmp(_, left, right) => {
                self.count_expr(left);
                self.count_expr(right);
            }
        }
    }

    fn collect_expr_binding(&mut self, expr: &SymExpr) {
        {
            let Some(visit) = self.expr_visits.get_mut(expr) else { return };
            if visit.collected {
                return;
            }
            visit.collected = true;
        }
        match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => {}
            SymExprKind::Not(value) => self.collect_expr_binding(value),
            SymExprKind::BinOp(_, left, right) => {
                self.collect_expr_binding(left);
                self.collect_expr_binding(right);
            }
            SymExprKind::TernOp(_, left, right, modulus) => {
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
        {
            let Some(visit) = self.bool_visits.get_mut(expr) else { return };
            if visit.collected {
                return;
            }
            visit.collected = true;
        }
        match expr.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => self.collect_bool_binding(value),
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    self.collect_bool_binding(value);
                }
            }
            SymBoolExprKind::Cmp(_, left, right) => {
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
    cx: &'a SymCx,
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
            SymExprKind::Var(symbol)
            | SymExprKind::GasLeft(symbol)
            | SymExprKind::Keccak { name: symbol, .. }
            | SymExprKind::Hash { name: symbol, .. } => out.push_str(self.cx.symbol_name(*symbol)),
            SymExprKind::Not(value) => {
                out.push_str("(bvnot ");
                self.write_expr(out, value, skip_expr, skip_bool);
                out.push(')');
            }
            SymExprKind::BinOp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                self.write_expr(out, left, skip_expr, skip_bool);
                out.push(' ');
                self.write_expr(out, right, skip_expr, skip_bool);
                out.push(')');
            }
            SymExprKind::TernOp(op, left, right, modulus) => {
                self.write_wide_modular_arithmetic(out, op.smt(), left, right, modulus);
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

fn normalize_bool_node_for_solver(cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
    if let Some(normalized) = expr.normalize_udiv_for_solver(cx) {
        return normalized;
    }

    match expr.kind() {
        SymBoolExprKind::Not(value) => match value.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right)
                if matches!(left.kind(), SymExprKind::Not(_)) =>
            {
                SymBoolExpr::cmp(cx, SymCmpOp::Ule, right.clone(), left.clone())
            }
            _ => expr,
        },
        SymBoolExprKind::Cmp(op, left, right) => {
            let left = normalize_expr_for_solver(cx, left.clone());
            let right = normalize_expr_for_solver(cx, right.clone());
            if *op == SymCmpOp::Eq && polynomial_identity(&left, &right) {
                return SymBoolExpr::constant(cx, true);
            }
            let normalized = normalize_cmp_for_solver(cx, *op, left, right);
            normalized.normalize_udiv_for_solver(cx).unwrap_or(normalized)
        }
        _ => expr,
    }
}

fn normalize_cmp_for_solver(
    cx: &mut SymCx,
    op: SymCmpOp,
    left: SymExpr,
    right: SymExpr,
) -> SymBoolExpr {
    if op == SymCmpOp::Eq {
        if right.as_const().is_some_and(|value| value.is_zero())
            && let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = left.kind()
        {
            // Word subtraction is zero exactly when both operands are equal, including at the
            // modular boundary. Solc commonly lowers optimized equality checks to this shape.
            return SymBoolExpr::eq(cx, minuend.clone(), subtrahend.clone());
        }
        if left.as_const().is_some_and(|value| value.is_zero())
            && let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = right.kind()
        {
            return SymBoolExpr::eq(cx, minuend.clone(), subtrahend.clone());
        }
    }

    match op {
        // `a > b => b < a`.
        SymCmpOp::Ugt => SymBoolExpr::cmp(cx, SymCmpOp::Ult, right, left),
        // `a >= b => b <= a`.
        SymCmpOp::Uge => SymBoolExpr::cmp(cx, SymCmpOp::Ule, right, left),
        // `a >s b => b <s a`.
        SymCmpOp::Sgt => SymBoolExpr::cmp(cx, SymCmpOp::Slt, right, left),
        SymCmpOp::Eq | SymCmpOp::Ult | SymCmpOp::Ule | SymCmpOp::Slt => {
            SymBoolExpr::cmp(cx, op, left, right)
        }
    }
}

/// Simple facts learned from the normalized conjunction currently being queried.
#[derive(Default)]
pub(super) struct ConstraintContext {
    upper_bounds: HashMap<SymExpr, U256>,
    lower_bounds: HashMap<SymExpr, U256>,
}

#[derive(Clone, Copy)]
struct WordInterval {
    min: U256,
    max: U256,
}

// These analyses are solver optimizations, so exceeding their local work budget must only make
// them decline a rewrite. Keeping the bound shared and private prevents deeply nested bytecode
// expressions from turning a proof shortcut into unbounded Rust recursion.
const MAX_LOCAL_ANALYSIS_NODES: usize = 256;

impl WordInterval {
    fn new(min: U256, max: U256) -> Option<Self> {
        (min <= max).then_some(Self { min, max })
    }

    const fn exact(value: U256) -> Self {
        Self { min: value, max: value }
    }

    fn with_bounds(self, lower: Option<U256>, upper: Option<U256>) -> Option<Self> {
        Self::new(
            self.min.max(lower.unwrap_or(U256::ZERO)),
            self.max.min(upper.unwrap_or(U256::MAX)),
        )
    }
}

impl ConstraintContext {
    pub(super) fn new(constraints: &[SymBoolExpr]) -> Self {
        Self::from_constraints(constraints.iter(), constraints.len())
    }

    fn from_constraints<'a>(
        constraints: impl Clone + Iterator<Item = &'a SymBoolExpr>,
        constraint_count: usize,
    ) -> Self {
        let mut context = Self::default();
        for constraint in constraints.clone() {
            context.record_upper_bound_constraint(constraint);
            context.record_lower_bound_constraint(constraint);
        }
        // A bounded number of rounds closes ordinary order chains. Relational propagation keeps
        // strict comparisons weak (`a < b` propagates only `a <= upper(b)`), so inconsistent
        // cycles cannot tighten a bound one integer at a time across the uint256 domain.
        for _ in 0..constraint_count {
            let mut changed = false;
            for constraint in constraints.clone() {
                changed |= context.propagate_order_bounds(constraint);
            }
            if !changed {
                break;
            }
        }
        context
    }

    fn upper_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn lower_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.lower_bounds.get(expr).copied()
    }

    /// Conservatively identifies every conjunct that path facts may rewrite to `true`.
    fn could_contextually_disappear(expr: &SymBoolExpr) -> bool {
        match expr.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                Self::mul_div_identity_operands(left, right).is_some()
                    || Self::mul_div_identity_operands(right, left).is_some()
                    || Self::masked_word_side_eq_self_shape(left, right).is_some()
                    || Self::masked_word_side_eq_self_shape(right, left).is_some()
            }
            SymBoolExprKind::Not(value) => value
                .zero_check_operand()
                .is_some_and(|word| matches!(word.kind(), SymExprKind::BinOp(SymBinOp::Or, _, _))),
            SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(_, _, _) | SymBoolExprKind::And(_) => {
                false
            }
        }
    }

    fn normalize_bool(&self, cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.kind() {
            SymBoolExprKind::Not(value) if self.unsigned_bool_always_true(value) => {
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if self.mul_div_identity(left, right) || self.mul_div_identity(right, left) =>
            {
                SymBoolExpr::constant(cx, true)
            }
            SymBoolExprKind::Not(value)
                if matches!(
                    value.kind(),
                    SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                        if self.mul_div_identity(left, right)
                            || self.mul_div_identity(right, left)
                ) =>
            {
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if self.masked_word_eq_self(left, right) =>
            {
                // `x & mask == x => true` when the current context proves `x <= mask`.
                SymBoolExpr::constant(cx, true)
            }
            SymBoolExprKind::Not(value) if self.masked_eq_self_condition(value) => {
                // `x & mask != x => false` when the current context proves `x <= mask`.
                SymBoolExpr::constant(cx, false)
            }
            _ if expr
                .zero_check_operand()
                .is_some_and(|left| self.word_bool_always_true(cx, left)) =>
            {
                // `always_true_word == 0 => false`.
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Not(value)
                if value
                    .zero_check_operand()
                    .is_some_and(|left| self.word_bool_always_true(cx, left)) =>
            {
                // `always_true_word != 0 => true`.
                SymBoolExpr::constant(cx, true)
            }
            _ => expr,
        }
    }

    fn masked_eq_self_condition(&self, expr: &SymBoolExpr) -> bool {
        match expr.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                self.masked_word_eq_self(left, right)
            }
            _ => false,
        }
    }

    fn masked_word_eq_self(&self, left: &SymExpr, right: &SymExpr) -> bool {
        self.masked_word_side_eq_self(left, right) || self.masked_word_side_eq_self(right, left)
    }

    fn masked_word_side_eq_self(&self, masked: &SymExpr, value: &SymExpr) -> bool {
        Self::masked_word_side_eq_self_shape(masked, value)
            .is_some_and(|bits| self.unsigned_bits(value) <= bits)
    }

    fn masked_word_side_eq_self_shape(masked: &SymExpr, value: &SymExpr) -> Option<usize> {
        let SymExprKind::BinOp(SymBinOp::And, left, right) = masked.kind() else { return None };
        let (source, mask) = right
            .as_const()
            .map(|mask| (left, mask))
            .or_else(|| left.as_const().map(|mask| (right, mask)))?;
        let bits = mask_low_bits(mask)?;
        (source == value).then_some(bits)
    }

    fn record_upper_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.upper_bound_constraint(constraint) {
            self.record_upper_bound(expr.clone(), bound);
        }
    }

    fn record_upper_bound(&mut self, expr: SymExpr, bound: U256) -> bool {
        match self.upper_bounds.entry(expr) {
            alloy_primitives::map::Entry::Occupied(mut entry) if bound < *entry.get() => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Vacant(entry) => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Occupied(_) => false,
        }
    }

    fn record_lower_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.lower_bound_constraint(constraint) {
            self.record_lower_bound(expr.clone(), bound);
        }
    }

    fn record_lower_bound(&mut self, expr: SymExpr, bound: U256) -> bool {
        match self.lower_bounds.entry(expr) {
            alloy_primitives::map::Entry::Occupied(mut entry) if bound > *entry.get() => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Vacant(entry) => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Occupied(_) => false,
        }
    }

    fn propagate_order_bounds(&mut self, constraint: &SymBoolExpr) -> bool {
        match constraint.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match op {
                SymCmpOp::Ult | SymCmpOp::Ule => self.propagate_less_or_equal_bounds(left, right),
                SymCmpOp::Ugt | SymCmpOp::Uge => self.propagate_less_or_equal_bounds(right, left),
                SymCmpOp::Eq => {
                    let changed = self.propagate_less_or_equal_bounds(left, right);
                    self.propagate_less_or_equal_bounds(right, left) || changed
                }
                SymCmpOp::Slt | SymCmpOp::Sgt => false,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => match op {
                    SymCmpOp::Ult | SymCmpOp::Ule => {
                        self.propagate_less_or_equal_bounds(right, left)
                    }
                    SymCmpOp::Ugt | SymCmpOp::Uge => {
                        self.propagate_less_or_equal_bounds(left, right)
                    }
                    SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => false,
                },
                _ => false,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => false,
        }
    }

    /// Propagates interval bounds through the known unsigned relation `left <= right`.
    fn propagate_less_or_equal_bounds(&mut self, left: &SymExpr, right: &SymExpr) -> bool {
        let upper = self.upper_bound(right);
        let lower = self.lower_bound(left);
        let upper_changed = upper.is_some_and(|bound| self.record_upper_bound(left.clone(), bound));
        let lower_changed =
            lower.is_some_and(|bound| self.record_lower_bound(right.clone(), bound));
        upper_changed || lower_changed
    }

    fn upper_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match *op {
                SymCmpOp::Eq => const_side_bound(left, right),
                SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                    (_, Some(bound)) => (!bound.is_zero()).then(|| (left, bound - U256::from(1))),
                    _ => None,
                },
                SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                    (_, Some(bound)) => Some((left, bound)),
                    _ => None,
                },
                SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                    (Some(bound), _) => (!bound.is_zero()).then(|| (right, bound - U256::from(1))),
                    _ => None,
                },
                SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                    (Some(bound), _) => Some((right, bound)),
                    _ => None,
                },
                SymCmpOp::Slt | SymCmpOp::Sgt => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => match *op {
                    SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                        (_, Some(bound)) => Some((left, bound)),
                        _ => None,
                    },
                    SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                        (_, Some(bound)) => {
                            (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                        }
                        _ => None,
                    },
                    SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                        (Some(bound), _) => Some((right, bound)),
                        _ => None,
                    },
                    SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                        (Some(bound), _) => {
                            (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                        }
                        _ => None,
                    },
                    SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
                },
                _ => None,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn lower_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => const_side_bound(left, right),
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                    if right.as_const().is_some_and(|value| value.is_zero()) {
                        Some((left, U256::from(1)))
                    } else if left.as_const().is_some_and(|value| value.is_zero()) {
                        Some((right, U256::from(1)))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn unsigned_bool_always_true(&self, expr: &SymBoolExpr) -> bool {
        match expr.kind() {
            SymBoolExprKind::Cmp(op, left, right) => {
                self.unsigned_cmp_always_true(*op, left, right)
            }
            _ => false,
        }
    }

    fn unsigned_cmp_always_true(&self, op: SymCmpOp, left: &SymExpr, right: &SymExpr) -> bool {
        if op == SymCmpOp::Eq
            && (self.mul_div_identity(left, right) || self.mul_div_identity(right, left))
        {
            return true;
        }
        let Some(left) = self.interval(left) else { return false };
        let Some(right) = self.interval(right) else { return false };
        match op {
            SymCmpOp::Ult => left.max < right.min,
            SymCmpOp::Ule => left.max <= right.min,
            SymCmpOp::Ugt => left.min > right.max,
            SymCmpOp::Uge => left.min >= right.max,
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => false,
        }
    }

    fn mul_div_identity(&self, quotient: &SymExpr, expected: &SymExpr) -> bool {
        let Some((denominator, other)) = Self::mul_div_identity_operands(quotient, expected) else {
            return false;
        };

        self.interval(denominator).is_some_and(|interval| !interval.min.is_zero())
            && self.mul_cannot_overflow_256(denominator, other)
    }

    fn mul_div_identity_operands<'a>(
        quotient: &'a SymExpr,
        expected: &SymExpr,
    ) -> Option<(&'a SymExpr, &'a SymExpr)> {
        let (numerator, denominator) = quotient.udiv_operands()?;
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = numerator.kind() else {
            return None;
        };
        let other = if left == denominator {
            right
        } else if right == denominator {
            left
        } else {
            return None;
        };
        (other == expected).then_some((denominator, other))
    }

    fn interval(&self, expr: &SymExpr) -> Option<WordInterval> {
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.interval_cached(expr, &mut intervals, &mut remaining)
    }

    fn interval_cached(
        &self,
        expr: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<WordInterval> {
        if let Some(interval) = intervals.get(expr) {
            return *interval;
        }

        let lower = self.lower_bound(expr);
        let upper = self.upper_bound(expr);
        let explicit_bounds = || {
            if lower.is_none() && upper.is_none() {
                return None;
            }
            WordInterval::new(lower.unwrap_or(U256::ZERO), upper.unwrap_or(U256::MAX))
        };
        if *remaining == 0 {
            let interval = explicit_bounds();
            intervals.insert(expr.clone(), interval);
            return interval;
        }
        *remaining -= 1;

        let interval =
            self.structural_interval(expr, intervals, remaining).or_else(explicit_bounds);
        let interval = interval.and_then(|interval| interval.with_bounds(lower, upper));
        intervals.insert(expr.clone(), interval);
        interval
    }

    fn structural_interval(
        &self,
        expr: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<WordInterval> {
        match expr.kind() {
            SymExprKind::Const(value) => Some(WordInterval::exact(*value)),
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                let mask = left.as_const().or_else(|| right.as_const())?;
                Some(WordInterval { min: U256::ZERO, max: mask })
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval {
                    min: left.min.checked_add(right.min)?,
                    max: left.max.checked_add(right.max)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                if left.min < right.max {
                    return None;
                }
                Some(WordInterval {
                    min: left.min.checked_sub(right.max)?,
                    max: left.max.checked_sub(right.min)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval {
                    min: left.min.checked_mul(right.min)?,
                    max: left.max.checked_mul(right.max)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Shr, value, shift) => {
                let shift = shift.as_const()?;
                if shift >= U256::from(256) {
                    return Some(WordInterval::exact(U256::ZERO));
                }
                let value = self.interval_cached(value, intervals, remaining)?;
                let shift = shift.to::<usize>();
                Some(WordInterval { min: value.min >> shift, max: value.max >> shift })
            }
            SymExprKind::Ite(_, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval { min: left.min.min(right.min), max: left.max.max(right.max) })
            }
            _ => None,
        }
    }
}

fn const_side_bound<'a>(left: &'a SymExpr, right: &'a SymExpr) -> Option<(&'a SymExpr, U256)> {
    right
        .as_const()
        .map(|value| (left, value))
        .or_else(|| left.as_const().map(|value| (right, value)))
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(cx: &mut SymCx, expr: SymExpr) -> SymExpr {
    if expr.contains_ite() { expr.fold(cx, &mut normalize_expr_node_for_solver) } else { expr }
}

fn polynomial_identity(left: &SymExpr, right: &SymExpr) -> bool {
    if !polynomial_normalization_can_help(left) && !polynomial_normalization_can_help(right) {
        return false;
    }
    matches!(
        (Polynomial::from_expr(left), Polynomial::from_expr(right)),
        (Some(left), Some(right)) if left == right
    )
}

fn polynomial_normalization_can_help(expr: &SymExpr) -> bool {
    let crosses_sum_product_boundary = match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
            matches!(left.kind(), SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, ..))
                || matches!(right.kind(), SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, ..))
        }
        SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, left, right) => {
            matches!(left.kind(), SymExprKind::BinOp(SymBinOp::Mul, ..))
                || matches!(right.kind(), SymExprKind::BinOp(SymBinOp::Mul, ..))
                || matches!(
                    left.kind(),
                    SymExprKind::BinOp(SymBinOp::Shl, _, shift)
                        if shift.as_const().is_some_and(|shift| shift < U256::from(256))
                )
                || matches!(
                    right.kind(),
                    SymExprKind::BinOp(SymBinOp::Shl, _, shift)
                        if shift.as_const().is_some_and(|shift| shift < U256::from(256))
                )
        }
        _ => false,
    };
    if !crosses_sum_product_boundary {
        return false;
    }

    fn ring_shape(
        expr: &SymExpr,
        shapes: &mut HashMap<SymExpr, Option<(usize, usize)>>,
        remaining: &mut usize,
    ) -> Option<(usize, usize)> {
        if let Some(shape) = shapes.get(expr) {
            return *shape;
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        let shape = (|| match expr.kind() {
            SymExprKind::Const(_) | SymExprKind::Var(_) => Some((0, 0)),
            SymExprKind::BinOp(
                op @ (SymBinOp::Add | SymBinOp::Sub | SymBinOp::Mul),
                left,
                right,
            ) => {
                let left = ring_shape(left, shapes, remaining)?;
                let right = ring_shape(right, shapes, remaining)?;
                let operations = left.0.saturating_add(right.0).saturating_add(1);
                let multiplications = left
                    .1
                    .saturating_add(right.1)
                    .saturating_add(usize::from(*op == SymBinOp::Mul));
                Some((operations, multiplications))
            }
            SymExprKind::BinOp(SymBinOp::Shl, value, shift)
                if shift.as_const().is_some_and(|shift| shift < U256::from(256)) =>
            {
                let shape = ring_shape(value, shapes, remaining)?;
                Some((shape.0.saturating_add(1), shape.1.saturating_add(1)))
            }
            _ => None,
        })();
        shapes.insert(expr.clone(), shape);
        shape
    }

    let mut shapes = HashMap::default();
    let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
    ring_shape(expr, &mut shapes, &mut remaining)
        .is_some_and(|(operations, multiplications)| operations > 1 && multiplications > 0)
}

// Keep distributive expansion predictably bounded. The motivating accounting identity needs two
// terms with two factors; these limits leave ample room for ordinary identities without allowing
// adversarial expressions to explode.
const MAX_POLYNOMIAL_TERMS: usize = 32;
const MAX_MONOMIAL_FACTORS: usize = 8;
const MAX_POLYNOMIAL_PRODUCTS: usize = 256;

type Monomial = Vec<SymExpr>;

/// A sparse polynomial over the EVM word ring Z/(2^256).
///
/// Addition, subtraction, and multiplication of EVM words obey the ring laws even when they
/// wrap. Canonicalizing small expressions here lets the solver recognize nonlinear algebraic
/// identities without replacing bit-vector semantics with unbounded integer arithmetic.
#[derive(Clone, PartialEq, Eq)]
struct Polynomial {
    terms: HashMap<Monomial, U256>,
}

impl Polynomial {
    fn from_expr(expr: &SymExpr) -> Option<Self> {
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        Self::from_expr_cached(expr, &mut HashMap::default(), &mut remaining)
    }

    fn from_expr_cached(
        expr: &SymExpr,
        polynomials: &mut HashMap<SymExpr, Option<Self>>,
        remaining: &mut usize,
    ) -> Option<Self> {
        if let Some(polynomial) = polynomials.get(expr) {
            return polynomial.clone();
        }
        if *remaining == 0 {
            polynomials.insert(expr.clone(), None);
            return None;
        }
        *remaining -= 1;
        let polynomial =
            (|| match expr.kind() {
                SymExprKind::Const(value) => Some(Self::constant(*value)),
                SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .add(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .sub(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .mul(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Shl, value, shift)
                    if let Some(shift) = shift.as_const()
                        && shift < U256::from(256) =>
                {
                    let coefficient = U256::ONE << usize::try_from(shift).ok()?;
                    Self::from_expr_cached(value, polynomials, remaining)?
                        .mul(Self::constant(coefficient))
                }
                _ => {
                    let terms = HashMap::from_iter([(vec![expr.clone()], U256::ONE)]);
                    Some(Self { terms })
                }
            })();
        polynomials.insert(expr.clone(), polynomial.clone());
        polynomial
    }

    fn constant(value: U256) -> Self {
        let mut terms = HashMap::default();
        if !value.is_zero() {
            terms.insert(Vec::new(), value);
        }
        Self { terms }
    }

    fn add(mut self, right: Self) -> Option<Self> {
        for (monomial, coefficient) in right.terms {
            self.add_term(monomial, coefficient);
            if self.terms.len() > MAX_POLYNOMIAL_TERMS {
                return None;
            }
        }
        Some(self)
    }

    fn sub(mut self, right: Self) -> Option<Self> {
        for (monomial, coefficient) in right.terms {
            self.add_term(monomial, U256::ZERO.wrapping_sub(coefficient));
            if self.terms.len() > MAX_POLYNOMIAL_TERMS {
                return None;
            }
        }
        Some(self)
    }

    fn mul(self, right: Self) -> Option<Self> {
        let products = self.terms.len().checked_mul(right.terms.len())?;
        if products > MAX_POLYNOMIAL_PRODUCTS {
            return None;
        }

        let mut out = Self { terms: HashMap::default() };
        for (left_monomial, left_coefficient) in &self.terms {
            for (right_monomial, right_coefficient) in &right.terms {
                let factor_count = left_monomial.len().checked_add(right_monomial.len())?;
                if factor_count > MAX_MONOMIAL_FACTORS {
                    return None;
                }
                let mut monomial = Vec::with_capacity(factor_count);
                monomial.extend(left_monomial.iter().cloned());
                monomial.extend(right_monomial.iter().cloned());
                SymExpr::sort_interned_factors(&mut monomial);
                out.add_term(monomial, left_coefficient.wrapping_mul(*right_coefficient));
                if out.terms.len() > MAX_POLYNOMIAL_TERMS {
                    return None;
                }
            }
        }
        Some(out)
    }

    fn add_term(&mut self, monomial: Monomial, coefficient: U256) {
        if coefficient.is_zero() {
            return;
        }
        let coefficient =
            self.terms.get(&monomial).copied().unwrap_or_default().wrapping_add(coefficient);
        if coefficient.is_zero() {
            self.terms.remove(&monomial);
        } else {
            self.terms.insert(monomial, coefficient);
        }
    }
}

fn normalize_expr_node_for_solver(cx: &mut SymCx, expr: SymExpr) -> SymExpr {
    match expr.kind() {
        SymExprKind::Ite(cond, left, right) => {
            normalize_ite_expr_for_solver(cx, cond.clone(), left.clone(), right.clone())
        }
        _ => expr,
    }
}

fn normalize_ite_expr_for_solver(
    cx: &mut SymCx,
    cond: SymBoolExpr,
    left: SymExpr,
    right: SymExpr,
) -> SymExpr {
    let cond = normalize_bool_for_solver(cx, cond);
    if left == right {
        // `ite(c, a, a) => a`.
        return left;
    }
    if left.as_const() == Some(U256::from(1))
        && right.normalized_bool_word_condition(cx).as_ref() == Some(&cond)
    {
        // `ite(c, 1, bool_word(c)) => bool_word(c)`.
        return right;
    }
    if right.as_const().is_some_and(|value| value.is_zero())
        && left.normalized_bool_word_condition(cx).as_ref() == Some(&cond)
    {
        // `ite(c, bool_word(c), 0) => bool_word(c)`.
        return left;
    }
    SymExpr::ite(cx, cond, left, right)
}

impl SymExpr {
    fn add_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().max(right.unsigned_bits()).saturating_add(1) <= 256
    }

    fn word_bool_always_true(&self, cx: &mut SymCx) -> bool {
        ConstraintContext::default().word_bool_always_true(cx, self)
    }
}

impl SymBoolExpr {
    fn normalize_udiv_for_solver(&self, cx: &mut SymCx) -> Option<Self> {
        match self.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                left.normalized_bool_word_condition(cx).map(|value| value.not(cx)).or_else(|| {
                    if left.word_bool_always_true(cx) {
                        // `always_true_word == 0 => false`.
                        Some(Self::constant(cx, false))
                    } else {
                        let zero = SymExpr::zero(cx);
                        Self::normalize_udiv_eq_zero(cx, left, &zero)
                    }
                })
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if right.as_const() == Some(U256::from(1)) =>
            {
                // `bool_word(c) == 1 => c`.
                left.normalized_bool_word_condition(cx)
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                    if right.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if left.word_bool_always_true(cx) {
                        // `always_true_word != 0 => true`.
                        Some(Self::constant(cx, true))
                    } else {
                        let zero = SymExpr::zero(cx);
                        Self::normalize_udiv_eq_zero(cx, left, &zero).map(|value| value.not(cx))
                    }
                }
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                    Self::normalize_udiv_eq_zero(cx, left, right).map(|value| value.not(cx))
                }
                SymBoolExprKind::Cmp(op, left, right) => {
                    Self::normalize_add_overflow_cmp(cx, *op, left, right)
                        .map(|value| value.not(cx))
                        .or_else(|| {
                            Self::normalize_udiv_cmp(cx, *op, left, right)
                                .map(|value| value.not(cx))
                        })
                }
                _ => None,
            },
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                Self::normalize_udiv_eq_zero(cx, left, right)
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::normalize_add_overflow_cmp(cx, *op, left, right)
                    .or_else(|| Self::normalize_udiv_cmp(cx, *op, left, right))
            }
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn normalize_add_overflow_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        // Strict forms test overflow and non-strict forms its complement; addition wraps iff the
        // increment exceeds `~base`.
        let (base, increment, overflow) = match op {
            SymCmpOp::Ugt => {
                right.add_with_operand(left).map(|(_, increment)| (left, increment, true))
            }
            SymCmpOp::Ult => {
                left.add_with_operand(right).map(|(_, increment)| (right, increment, true))
            }
            SymCmpOp::Uge => {
                left.add_with_operand(right).map(|(_, increment)| (right, increment, false))
            }
            SymCmpOp::Ule => {
                right.add_with_operand(left).map(|(_, increment)| (left, increment, false))
            }
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
        }?;
        if base.add_cannot_overflow_256(increment) {
            return Some(Self::constant(cx, !overflow));
        }

        let limit = match base.kind() {
            SymExprKind::BinOp(SymBinOp::Sub, max, value) if max.as_const() == Some(U256::MAX) => {
                value.clone()
            }
            _ => SymExpr::not(cx, base.clone()),
        };
        Some(if overflow {
            Self::cmp(cx, SymCmpOp::Ult, limit, increment.clone())
        } else {
            Self::cmp(cx, SymCmpOp::Ule, increment.clone(), limit)
        })
    }

    fn normalize_udiv_eq_zero(cx: &mut SymCx, left: &SymExpr, right: &SymExpr) -> Option<Self> {
        if right.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = left.normalize_eq_zero_for_solver(cx)
        {
            // `word_bool(c) == 0 => !c`.
            return Some(condition);
        }
        None
    }

    fn normalize_udiv_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        match op {
            SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                // `a > 0 => a != 0`.
                (_, Some(value)) if value.is_zero() => left
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, left).not(cx))),
                // `1 > a => a == 0`.
                (Some(value), _) if value == U256::from(1) => right
                    .normalize_eq_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right))),
                _ => None,
            },
            SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                // `a >= 1 => a != 0`.
                (_, Some(value)) if value == U256::from(1) => left
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, left).not(cx))),
                // `0 >= a => a == 0`.
                (Some(value), _) if value.is_zero() => right
                    .normalize_eq_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right))),
                _ => None,
            },
            SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                // `a <= 0 => a == 0`.
                (_, Some(value)) if value.is_zero() => {
                    left.normalize_eq_zero_for_solver(cx).or_else(|| Some(Self::eq_zero(cx, left)))
                }
                // `1 <= a => a != 0`.
                (Some(value), _) if value == U256::from(1) => right
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right).not(cx))),
                _ => None,
            },
            SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                // `a < 1 => a == 0`.
                (_, Some(value)) if value == U256::from(1) => {
                    left.normalize_eq_zero_for_solver(cx).or_else(|| Some(Self::eq_zero(cx, left)))
                }
                // `0 < a => a != 0`.
                (Some(value), _) if value.is_zero() => right
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right).not(cx))),
                _ => None,
            },
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
        }
    }

    fn eq_zero(cx: &mut SymCx, expr: &SymExpr) -> Self {
        let zero = SymExpr::zero(cx);
        Self::eq(cx, expr.clone(), zero)
    }
}

impl SymExpr {
    fn normalized_bool_word_condition(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        self.strip_low_byte_mask()
            .bool_word_condition()
            .map(|condition| normalize_bool_for_solver(cx, condition))
    }

    fn add_with_operand<'a>(&'a self, operand: &Self) -> Option<(&'a Self, &'a Self)> {
        let SymExprKind::BinOp(SymBinOp::Add, left, right) = self.kind() else { return None };
        if left == operand {
            Some((left, right))
        } else if right == operand {
            Some((right, left))
        } else {
            None
        }
    }

    fn normalize_eq_zero_for_solver(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            // `a / b == 0 => b == 0 || a < b`.
            return Some(Self::udiv_zero_condition(cx, numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_zero = match then_expr.normalize_eq_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let then_expr = normalize_expr_for_solver(cx, then_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, then_expr, zero)
                }
            };
            let else_zero = match else_expr.normalize_eq_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let else_expr = normalize_expr_for_solver(cx, else_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, else_expr, zero)
                }
            };
            if then_zero.contains_udiv() || else_zero.contains_udiv() {
                return None;
            }
            // `ite(c, a, b) == 0 => (c && a == 0) || (!c && b == 0)`.
            let condition = normalize_bool_for_solver(cx, condition.clone());
            let then_condition = SymBoolExpr::and(cx, vec![condition.clone(), then_zero]);
            let not_condition = condition.not(cx);
            let else_condition = SymBoolExpr::and(cx, vec![not_condition, else_zero]);
            return Some(SymBoolExpr::or(cx, vec![then_condition, else_condition]));
        }
        None
    }

    fn normalize_ne_zero_for_solver(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            // `a / b != 0 => b != 0 && a >= b`.
            return Some(Self::udiv_nonzero_condition(cx, numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_nonzero = match then_expr.normalize_ne_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let then_expr = normalize_expr_for_solver(cx, then_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, then_expr, zero).not(cx)
                }
            };
            let else_nonzero = match else_expr.normalize_ne_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let else_expr = normalize_expr_for_solver(cx, else_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, else_expr, zero).not(cx)
                }
            };
            if then_nonzero.contains_udiv() || else_nonzero.contains_udiv() {
                return None;
            }
            // `ite(c, a, b) != 0 => (c && a != 0) || (!c && b != 0)`.
            let condition = normalize_bool_for_solver(cx, condition.clone());
            let then_condition = SymBoolExpr::and(cx, vec![condition.clone(), then_nonzero]);
            let not_condition = condition.not(cx);
            let else_condition = SymBoolExpr::and(cx, vec![not_condition, else_nonzero]);
            return Some(SymBoolExpr::or(cx, vec![then_condition, else_condition]));
        }
        None
    }

    fn udiv_zero_condition(cx: &mut SymCx, numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(cx, numerator.clone());
        let denominator = normalize_expr_for_solver(cx, denominator.clone());
        let zero = Self::zero(cx);
        let denominator_zero = SymBoolExpr::eq(cx, denominator.clone(), zero);
        let below_denominator = SymBoolExpr::cmp(cx, SymCmpOp::Ult, numerator, denominator);
        SymBoolExpr::or(cx, vec![denominator_zero, below_denominator])
    }

    fn udiv_nonzero_condition(cx: &mut SymCx, numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(cx, numerator.clone());
        let denominator = normalize_expr_for_solver(cx, denominator.clone());
        let zero = Self::zero(cx);
        let denominator_nonzero = SymBoolExpr::eq(cx, denominator.clone(), zero).not(cx);
        let at_least_denominator = SymBoolExpr::cmp(cx, SymCmpOp::Uge, numerator, denominator);
        SymBoolExpr::and(cx, vec![denominator_nonzero, at_least_denominator])
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, cx: &mut SymCx, expr: &SymExpr) -> bool {
        let mut terms = Vec::new();
        expr.push_or_terms(&mut terms);
        if terms.len() <= 1 {
            return false;
        }

        let bool_terms = terms
            .iter()
            .filter_map(|term| term.normalized_bool_word_condition(cx))
            .collect::<Vec<_>>();
        if bool_terms.iter().any(|term| {
            let negated = term.clone().not(cx);
            bool_terms.contains(&negated)
        }) {
            // `c || !c => true`.
            return true;
        }
        for zero_term in &bool_terms {
            let Some(zero_operand) = zero_term.zero_check_operand() else { continue };
            if bool_terms.iter().any(|term| self.checked_mul_guard_for_operand(term, zero_operand))
            {
                // `a == 0 || guarded_mul_div(a) => true`.
                return true;
            }
        }
        false
    }

    fn checked_mul_guard_for_operand(&self, expr: &SymBoolExpr, zero_operand: &SymExpr) -> bool {
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = expr.kind() else {
            return false;
        };
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
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = numerator.kind() else {
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

    pub(super) fn mul_cannot_overflow_256(&self, left: &SymExpr, right: &SymExpr) -> bool {
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        if self
            .interval_cached(left, &mut intervals, &mut remaining)
            .zip(self.interval_cached(right, &mut intervals, &mut remaining))
            .is_some_and(|(left, right)| left.max.checked_mul(right.max).is_some())
        {
            return true;
        }

        let mut bit_widths = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.unsigned_bits_cached(left, &mut bit_widths, &mut remaining)
            .zip(self.unsigned_bits_cached(right, &mut bit_widths, &mut remaining))
            .is_some_and(|(left, right)| left.saturating_add(right) <= 256)
    }

    pub(super) fn unsigned_bits(&self, expr: &SymExpr) -> usize {
        let mut bit_widths = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.unsigned_bits_cached(expr, &mut bit_widths, &mut remaining).unwrap_or(256)
    }

    fn unsigned_bits_cached(
        &self,
        expr: &SymExpr,
        bit_widths: &mut HashMap<SymExpr, usize>,
        remaining: &mut usize,
    ) -> Option<usize> {
        if let Some(bits) = bit_widths.get(expr) {
            return Some(*bits);
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;

        let bits = match expr.kind() {
            SymExprKind::Const(value) => value.bit_len().max(1),
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. }
            | SymExprKind::Not(_) => 256,
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.unsigned_bits_cached(left, bit_widths, remaining)?.min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .max(self.unsigned_bits_cached(right, bit_widths, remaining)?)
                .saturating_add(1)
                .min(256),
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .saturating_add(self.unsigned_bits_cached(right, bit_widths, remaining)?)
                .min(256),
            SymExprKind::BinOp(SymBinOp::UDiv, left, _) => {
                self.unsigned_bits_cached(left, bit_widths, remaining)?
            }
            SymExprKind::Ite(_, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .max(self.unsigned_bits_cached(right, bit_widths, remaining)?),
            _ => 256,
        };

        let bits =
            self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits);
        bit_widths.insert(expr.clone(), bits);
        Some(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_normalization_keeps_contextual_rewrites_per_query() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let mask = SymExpr::constant(&mut cx, (U256::from(1) << 160) - U256::from(1));
        let masked = SymExpr::binop(&mut cx, SymBinOp::And, value.clone(), mask);
        let identity = SymBoolExpr::eq(&mut cx, masked, value.clone());
        let upper = SymExpr::constant(&mut cx, U256::from(1) << 160);
        let bounded = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, value, upper);
        let mut cache = HashMap::default();

        let normalized = normalize_constraints_for_solver_cached(
            &mut cx,
            &[identity.clone(), bounded.clone()],
            &mut cache,
        );
        assert_eq!(normalized, vec![bounded]);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&identity), Some(&identity));

        let normalized = normalize_constraints_for_solver_cached(
            &mut cx,
            std::slice::from_ref(&identity),
            &mut cache,
        );
        assert_eq!(normalized, vec![identity]);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn direct_contradiction_uses_members_of_derived_positive_conjunction() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let zero = SymExpr::zero(&mut cx);
        let x_is_zero = SymBoolExpr::eq(&mut cx, x, zero.clone());
        let y_is_zero = SymBoolExpr::eq(&mut cx, y, zero.clone());
        let x_word = SymExpr::bool_word(&mut cx, x_is_zero.clone());
        let y_word = SymExpr::bool_word(&mut cx, y_is_zero);
        let either_word = SymExpr::binop(&mut cx, SymBinOp::Or, x_word, y_word);
        let neither_is_zero = SymBoolExpr::eq(&mut cx, either_word, zero);

        let constraints = normalize_constraints_for_solver(&mut cx, &[neither_is_zero, x_is_zero]);
        assert!(constraints_are_directly_unsat(&mut cx, &constraints));
    }

    #[test]
    fn direct_contradiction_does_not_expand_derived_negated_conjunction() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let zero = SymExpr::zero(&mut cx);
        let x_is_zero = SymBoolExpr::eq(&mut cx, x, zero.clone());
        let y_is_zero = SymBoolExpr::eq(&mut cx, y, zero.clone());
        let x_word = SymExpr::bool_word(&mut cx, x_is_zero.clone());
        let y_word = SymExpr::bool_word(&mut cx, y_is_zero);
        let both_word = SymExpr::binop(&mut cx, SymBinOp::And, x_word, y_word);
        let not_both = SymBoolExpr::eq(&mut cx, both_word, zero);

        let constraints = normalize_constraints_for_solver(&mut cx, &[not_both, x_is_zero]);
        assert!(!constraints_are_directly_unsat(&mut cx, &constraints));
    }

    #[test]
    fn polynomial_identity_handles_shared_dag() {
        let mut cx = SymCx::new();
        let shared_atom = SymExpr::var(&mut cx, "shared");
        let mut shared = shared_atom.clone();
        for _ in 0..64 {
            shared = SymExpr::binop(&mut cx, SymBinOp::Add, shared.clone(), shared);
        }
        let factor = SymExpr::var(&mut cx, "factor");
        let expression = SymExpr::binop(&mut cx, SymBinOp::Mul, shared, factor.clone());
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, shared_atom, factor);
        let shift = SymExpr::constant(&mut cx, U256::from(64));
        let expected = SymExpr::binop(&mut cx, SymBinOp::Shl, product, shift);

        assert!(polynomial_identity(&expression, &expected));
    }

    #[test]
    fn polynomial_factors_use_interned_identity_order() {
        let mut cx = SymCx::new();
        let left = SymExpr::var(&mut cx, "left");
        let right = SymExpr::var(&mut cx, "right");
        let left_right = SymExpr::from_kind(
            &mut cx,
            SymExprKind::BinOp(SymBinOp::Mul, left.clone(), right.clone()),
        );
        let right_left =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::Mul, right, left));

        assert!(Polynomial::from_expr(&left_right) == Polynomial::from_expr(&right_left));
    }

    #[test]
    fn polynomial_identity_stops_at_factor_limit() {
        let mut cx = SymCx::new();
        let mut prefix = SymExpr::one(&mut cx);
        for index in 0..MAX_MONOMIAL_FACTORS - 1 {
            let factor = SymExpr::var(&mut cx, &format!("x_{index}"));
            prefix = SymExpr::binop(&mut cx, SymBinOp::Mul, prefix, factor);
        }
        let left = SymExpr::var(&mut cx, "left");
        let right = SymExpr::var(&mut cx, "right");
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, left.clone(), right.clone());
        let factored = SymExpr::binop(&mut cx, SymBinOp::Mul, prefix.clone(), sum);
        let left_product = SymExpr::binop(&mut cx, SymBinOp::Mul, prefix.clone(), left);
        let right_product = SymExpr::binop(&mut cx, SymBinOp::Mul, prefix, right);
        let expanded = SymExpr::binop(&mut cx, SymBinOp::Add, left_product, right_product);

        assert!(polynomial_identity(&factored, &expanded));

        let extra = SymExpr::var(&mut cx, "extra");
        let over_limit = SymExpr::binop(&mut cx, SymBinOp::Mul, factored, extra);

        assert!(!polynomial_identity(&over_limit, &over_limit));
    }

    #[test]
    fn polynomial_identity_stops_at_term_limit() {
        let mut cx = SymCx::new();
        let mut expression = SymExpr::zero(&mut cx);
        for index in 0..33 {
            let term = SymExpr::var(&mut cx, &format!("x_{index}"));
            expression = SymExpr::binop(&mut cx, SymBinOp::Add, expression, term);
        }
        let factor = SymExpr::var(&mut cx, "factor");
        expression = SymExpr::binop(&mut cx, SymBinOp::Mul, expression, factor);

        assert!(!polynomial_identity(&expression, &expression));
    }

    #[test]
    fn polynomial_identity_stops_at_product_limit() {
        let mut cx = SymCx::new();
        let mut left = SymExpr::zero(&mut cx);
        for index in 0..17 {
            let term = SymExpr::var(&mut cx, &format!("left_{index}"));
            left = SymExpr::binop(&mut cx, SymBinOp::Add, left, term);
        }
        let mut right = SymExpr::zero(&mut cx);
        for index in 0..16 {
            let term = SymExpr::var(&mut cx, &format!("right_{index}"));
            right = SymExpr::binop(&mut cx, SymBinOp::Add, right, term);
        }
        let expression = SymExpr::binop(&mut cx, SymBinOp::Mul, left, right);

        assert!(!polynomial_identity(&expression, &expression));
    }

    #[test]
    fn polynomial_identity_skips_irrelevant_and_unsupported_shapes() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let single_product = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), y.clone());
        assert!(!polynomial_identity(&single_product, &single_product));

        let denominator = SymExpr::var(&mut cx, "denominator");
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, x.clone(), denominator);
        let sum = SymExpr::binop(&mut cx, SymBinOp::Add, quotient, y);
        let unsupported = SymExpr::binop(&mut cx, SymBinOp::Mul, sum, x);
        assert!(!polynomial_identity(&unsupported, &unsupported));
    }

    #[test]
    fn polynomial_analysis_stops_at_input_node_limit() {
        let mut cx = SymCx::new();
        let one = SymExpr::one(&mut cx);
        let mut expression = SymExpr::var(&mut cx, "source");
        for _ in 0..MAX_LOCAL_ANALYSIS_NODES {
            expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::Add, expression, one.clone()),
            );
        }
        let factor = SymExpr::var(&mut cx, "factor");
        let product = SymExpr::from_kind(
            &mut cx,
            SymExprKind::BinOp(SymBinOp::Mul, expression.clone(), factor),
        );

        assert!(!polynomial_normalization_can_help(&product));
        assert!(Polynomial::from_expr(&expression).is_none());
    }

    #[test]
    fn shared_arithmetic_dag_interval_analysis_is_memoized() {
        let mut cx = SymCx::new();
        let source = SymExpr::var(&mut cx, "source");
        let one = SymExpr::one(&mut cx);
        let mut shared = SymExpr::binop(&mut cx, SymBinOp::And, source, one);
        for _ in 0..64 {
            shared = SymExpr::binop(&mut cx, SymBinOp::Add, shared.clone(), shared);
        }

        assert!(ConstraintContext::default().mul_cannot_overflow_256(&shared, &shared));
    }

    #[test]
    fn unique_arithmetic_chains_stop_at_analysis_budget() {
        let mut cx = SymCx::new();
        let zero = SymExpr::zero(&mut cx);
        let one = SymExpr::one(&mut cx);
        let mut shifted = one.clone();
        let mut divided = one.clone();
        for _ in 0..MAX_LOCAL_ANALYSIS_NODES {
            shifted = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::Shr, shifted, zero.clone()),
            );
            divided = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::UDiv, divided, one.clone()),
            );
        }

        let context = ConstraintContext::default();
        assert!(context.interval(&shifted).is_none());
        assert_eq!(context.unsigned_bits(&divided), 256);
        assert!(!context.mul_cannot_overflow_256(&divided, &divided));
    }
}
