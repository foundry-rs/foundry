use super::*;

/// Writes solver assertions with local sharing for repeated SMT subterms.
pub(crate) fn write_smt_assertions(out: &mut String, constraints: &[SymBoolExpr]) {
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
    out.push_str("(assert ");
    for (idx, binding) in plan.bindings.iter().enumerate() {
        out.push_str("(let ((");
        binding.write_name(out, idx);
        out.push(' ');
        match binding {
            SmtBinding::Expr(expr) => writer.write_expr(out, expr, Some(idx), None),
            SmtBinding::Bool(expr) => writer.write_bool(out, expr, None, Some(idx)),
        }
        out.push_str(")) ");
    }
    writer.write_bool_conjunction(out, constraints);
    for _ in &plan.bindings {
        out.push(')');
    }
    out.push_str(")\n");
}

struct SmtCsePlan {
    expr_counts: HashMap<SymExpr, usize>,
    bool_counts: HashMap<SymBoolExpr, usize>,
    expr_bindings: HashMap<SymExpr, usize>,
    bool_bindings: HashMap<SymBoolExpr, usize>,
    bindings: Vec<SmtBinding>,
}

impl SmtCsePlan {
    fn new(constraints: &[SymBoolExpr]) -> Self {
        let mut plan = Self {
            expr_counts: HashMap::default(),
            bool_counts: HashMap::default(),
            expr_bindings: HashMap::default(),
            bool_bindings: HashMap::default(),
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
        *self.expr_counts.entry(expr.clone()).or_default() += 1;
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
        *self.bool_counts.entry(expr.clone()).or_default() += 1;
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
        if self.should_bind_expr(expr) && !self.expr_bindings.contains_key(expr) {
            let idx = self.bindings.len();
            self.expr_bindings.insert(expr.clone(), idx);
            self.bindings.push(SmtBinding::Expr(expr.clone()));
        }
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
        if self.should_bind_bool(expr) && !self.bool_bindings.contains_key(expr) {
            let idx = self.bindings.len();
            self.bool_bindings.insert(expr.clone(), idx);
            self.bindings.push(SmtBinding::Bool(expr.clone()));
        }
    }

    fn should_bind_expr(&self, expr: &SymExpr) -> bool {
        self.expr_counts.get(expr).copied().unwrap_or_default() > 1
            && !matches!(
                expr.kind(),
                SymExprKind::Const(_)
                    | SymExprKind::Var(_)
                    | SymExprKind::GasLeft(_)
                    | SymExprKind::Keccak { .. }
                    | SymExprKind::Hash { .. }
            )
    }

    fn should_bind_bool(&self, expr: &SymBoolExpr) -> bool {
        self.bool_counts.get(expr).copied().unwrap_or_default() > 1
            && !matches!(expr.kind(), SymBoolExprKind::Const(_))
    }
}

enum SmtBinding {
    Expr(SymExpr),
    Bool(SymBoolExpr),
}

impl SmtBinding {
    fn write_name(&self, out: &mut String, idx: usize) {
        match self {
            Self::Expr(_) => Self::write_expr_name(out, idx),
            Self::Bool(_) => Self::write_bool_name(out, idx),
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
        if let Some(idx) = self.plan.expr_bindings.get(expr).copied()
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
        if let Some(idx) = self.plan.bool_bindings.get(expr).copied()
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

    fn write_bool_conjunction(&self, out: &mut String, constraints: &[SymBoolExpr]) {
        if constraints.len() == 1 {
            self.write_bool(out, &constraints[0], None, None);
            return;
        }

        out.push_str("(and");
        for constraint in constraints {
            out.push(' ');
            self.write_bool(out, constraint, None, None);
        }
        out.push(')');
    }
}
