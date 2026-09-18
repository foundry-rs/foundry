//! Query-level SMT-LIB assertion emission with common-subexpression bindings.

use super::*;

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
            let Some(visit) = self.expr_visits.get_mut(expr) else {
                return;
            };
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
            let Some(visit) = self.bool_visits.get_mut(expr) else {
                return;
            };
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
        let Some(visit) = self.expr_visits.get_mut(expr) else {
            return;
        };
        if visit.count <= 1 || visit.binding.is_some() || !Self::expr_can_bind(expr) {
            return;
        }
        let idx = self.bindings.len();
        visit.binding = Some(idx);
        self.bindings.push(SmtBinding::Expr(expr.clone()));
    }

    fn bind_bool(&mut self, expr: &SymBoolExpr) {
        let Some(visit) = self.bool_visits.get_mut(expr) else {
            return;
        };
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
