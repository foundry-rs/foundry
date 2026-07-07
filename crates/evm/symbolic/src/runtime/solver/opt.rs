use super::*;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
pub(crate) fn normalize_constraints_for_solver(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
) -> Vec<SymBoolExpr> {
    let normalized = normalize_constraint_batch(
        constraints.iter().cloned().map(|constraint| normalize_bool_for_solver(cx, constraint)),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)) {
        return normalized;
    }

    let context = ConstraintContext::new(&normalized);
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
    exprs.sort_by_cached_key(bool_structural_key);
    exprs.dedup();
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

/// Returns whether normalized conjunctive constraints contain a direct contradiction.
pub(super) fn constraints_are_directly_unsat(cx: &mut SymCx, constraints: &[SymBoolExpr]) -> bool {
    constraints.iter().any(|constraint| match constraint.kind() {
        SymBoolExprKind::Const(false) => true,
        SymBoolExprKind::Not(inner) => constraints.contains(inner),
        _ => {
            let negated = constraint.clone().not(cx);
            constraints.contains(&negated)
        }
    })
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
        SymBoolExprKind::Cmp(op, left, right) => {
            let left = normalize_expr_for_solver(cx, left.clone());
            let right = normalize_expr_for_solver(cx, right.clone());
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
struct ConstraintContext {
    upper_bounds: HashMap<SymExpr, U256>,
    lower_bounds: HashMap<SymExpr, U256>,
}

#[derive(Clone, Copy)]
struct WordInterval {
    min: U256,
    max: U256,
}

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
    fn new(constraints: &[SymBoolExpr]) -> Self {
        let mut context = Self::default();
        for constraint in constraints {
            context.record_upper_bound_constraint(constraint);
            context.record_lower_bound_constraint(constraint);
        }
        context
    }

    fn upper_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn lower_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.lower_bounds.get(expr).copied()
    }

    fn normalize_bool(&self, cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.kind() {
            SymBoolExprKind::Not(value) if self.unsigned_bool_always_true(value) => {
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
        let SymExprKind::BinOp(SymBinOp::And, left, right) = masked.kind() else { return false };
        let Some((source, mask)) = right
            .as_const()
            .map(|mask| (left, mask))
            .or_else(|| left.as_const().map(|mask| (right, mask)))
        else {
            return false;
        };
        let Some(bits) = mask_low_bits(mask) else { return false };
        source == value && self.unsigned_bits(value) <= bits
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

    fn record_lower_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.lower_bound_constraint(constraint) {
            self.record_lower_bound(expr.clone(), bound);
        }
    }

    fn record_lower_bound(&mut self, expr: SymExpr, bound: U256) {
        self.lower_bounds
            .entry(expr)
            .and_modify(|existing| *existing = (*existing).max(bound))
            .or_insert(bound);
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
                    nonzero_bound(left, right).or_else(|| nonzero_bound(right, left))
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

    fn interval(&self, expr: &SymExpr) -> Option<WordInterval> {
        let lower = self.lower_bound(expr);
        let upper = self.upper_bound(expr);
        let interval = self.structural_interval(expr).or_else(|| {
            (lower.is_some() || upper.is_some()).then(|| WordInterval {
                min: lower.unwrap_or(U256::ZERO),
                max: upper.unwrap_or(U256::MAX),
            })
        })?;
        interval.with_bounds(lower, upper)
    }

    fn structural_interval(&self, expr: &SymExpr) -> Option<WordInterval> {
        match expr.kind() {
            SymExprKind::Const(value) => Some(WordInterval::exact(*value)),
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                let mask = left.as_const().or_else(|| right.as_const())?;
                Some(WordInterval { min: U256::ZERO, max: mask })
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                let left = self.interval(left)?;
                let right = self.interval(right)?;
                Some(WordInterval {
                    min: left.min.checked_add(right.min)?,
                    max: left.max.checked_add(right.max)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                let left = self.interval(left)?;
                let right = self.interval(right)?;
                if left.min < right.max {
                    return None;
                }
                Some(WordInterval {
                    min: left.min.checked_sub(right.max)?,
                    max: left.max.checked_sub(right.min)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                let left = self.interval(left)?;
                let right = self.interval(right)?;
                Some(WordInterval {
                    min: left.min.checked_mul(right.min)?,
                    max: left.max.checked_mul(right.max)?,
                })
            }
            SymExprKind::Ite(_, left, right) => {
                let left = self.interval(left)?;
                let right = self.interval(right)?;
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

fn nonzero_bound<'a>(expr: &'a SymExpr, value: &'a SymExpr) -> Option<(&'a SymExpr, U256)> {
    value.as_const().is_some_and(|value| value.is_zero()).then(|| (expr, U256::from(1)))
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(cx: &mut SymCx, expr: SymExpr) -> SymExpr {
    if !expr.contains_ite() {
        return expr;
    }
    expr.fold(cx, &mut normalize_expr_node_for_solver)
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

    pub(crate) fn mul_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().saturating_add(right.unsigned_bits()) <= 256
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
        match op {
            // `a + b > a => false` when `a + b` cannot overflow.
            SymCmpOp::Ugt if left.add_overflow_check(right) => Some(Self::constant(cx, false)),
            // `a < a + b => false` when `a + b` cannot overflow.
            SymCmpOp::Ult if right.add_overflow_check(left) => Some(Self::constant(cx, false)),
            _ => None,
        }
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

    fn add_overflow_check(&self, right: &Self) -> bool {
        let Some((base, increment)) = right.add_with_operand(self) else { return false };
        base == self && base.add_cannot_overflow_256(increment)
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
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.unsigned_bits(left).min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right)).saturating_add(1).min(256)
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                self.unsigned_bits(left).saturating_add(self.unsigned_bits(right)).min(256)
            }
            SymExprKind::BinOp(SymBinOp::UDiv, left, _) => self.unsigned_bits(left),
            SymExprKind::Ite(_, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right))
            }
            _ => 256,
        };

        self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits)
    }
}
