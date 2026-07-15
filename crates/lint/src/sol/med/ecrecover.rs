use super::Ecrecover;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::primitives::{branch_always_exits, is_require_or_assert},
    },
};
use alloy_primitives::{U256, uint};
use solar::{
    ast::{BinOpKind, ElementaryType, UnOpKind},
    interface::{Span, data_structures::Never},
    sema::{
        Gcx,
        builtins::Builtin,
        eval::ConstantEvaluator,
        hir::{self, ExprKind, ItemId, LoopSource, Res, StmtKind, TypeKind, Visit},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

declare_forge_lint!(
    ECRECOVER,
    Severity::Med,
    "ecrecover",
    "ecrecover should reject malleable signatures"
);

/// Largest canonical secp256k1 `s` value, `n / 2`.
const SECP256K1_HALF_ORDER: U256 =
    uint!(0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0_U256);

impl<'hir> LateLintPass<'hir> for Ecrecover {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        let Some(body) = func.body else { return };
        let mut analyzer = Analyzer::new(gcx, hir);
        for stmt in body.stmts {
            let _ = analyzer.visit_stmt(stmt);
            if branch_always_exits(stmt) {
                break;
            }
        }
        for span in analyzer.hits {
            ctx.emit(&ECRECOVER, span);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ValueId {
    Initial(hir::VariableId),
    Assigned(u32),
}

#[derive(Clone, Default)]
struct FlowState {
    values: HashMap<hir::VariableId, ValueId>,
    low_s: HashSet<ValueId>,
}

impl FlowState {
    fn value(&self, var: hir::VariableId) -> ValueId {
        self.values.get(&var).copied().unwrap_or(ValueId::Initial(var))
    }
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    state: FlowState,
    next_value: u32,
    hits: Vec<Span>,
}

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self { gcx, hir, state: FlowState::default(), next_value: 0, hits: Vec::new() }
    }

    fn fresh_value(&mut self) -> ValueId {
        let value = ValueId::Assigned(self.next_value);
        self.next_value += 1;
        value
    }

    fn snapshot(&self) -> FlowState {
        self.state.clone()
    }

    fn restore(&mut self, state: FlowState) {
        self.state = state;
    }

    fn join(&mut self, left: FlowState, right: FlowState) -> FlowState {
        let mut joined = FlowState {
            low_s: left.low_s.intersection(&right.low_s).copied().collect(),
            ..FlowState::default()
        };
        let vars: HashSet<_> = left.values.keys().chain(right.values.keys()).copied().collect();
        let mut joined_values = HashMap::new();

        for var in vars {
            let left_value = left.value(var);
            let right_value = right.value(var);
            if left_value == right_value {
                joined.values.insert(var, left_value);
                continue;
            }

            let value = *joined_values
                .entry((left_value, right_value))
                .or_insert_with(|| self.fresh_value());
            if left.low_s.contains(&left_value) && right.low_s.contains(&right_value) {
                joined.low_s.insert(value);
            }
            joined.values.insert(var, value);
        }
        joined
    }

    fn current_value(&self, expr: &'hir hir::Expr<'hir>) -> Option<ValueId> {
        match &expr.peel_parens().kind {
            ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
                Res::Item(ItemId::Variable(var)) => Some(self.state.value(*var)),
                _ => None,
            }),
            ExprKind::Call(callee, args, _)
                if is_transparent_signature_cast(callee) && args.len() == 1 =>
            {
                args.exprs().next().and_then(|arg| self.current_value(arg))
            }
            ExprKind::Assign(_, None, rhs) => self.current_value(rhs),
            _ => None,
        }
    }

    fn const_value(&self, expr: &'hir hir::Expr<'hir>) -> Option<U256> {
        ConstantEvaluator::new(self.gcx).try_eval(expr).ok()?.as_u256()
    }

    fn is_proven_low_s(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.const_value(expr).is_some_and(|value| value <= SECP256K1_HALF_ORDER)
            || self.current_value(expr).is_some_and(|value| self.state.low_s.contains(&value))
    }

    fn assign_var(&mut self, var: hir::VariableId, rhs: Option<&'hir hir::Expr<'hir>>) {
        let value =
            rhs.and_then(|rhs| self.current_value(rhs)).unwrap_or_else(|| self.fresh_value());
        if rhs.is_some_and(|rhs| self.is_proven_low_s(rhs)) {
            self.state.low_s.insert(value);
        }
        self.state.values.insert(var, value);
    }

    fn assign_lhs(&mut self, lhs: &'hir hir::Expr<'hir>, rhs: Option<&'hir hir::Expr<'hir>>) {
        if let ExprKind::Tuple(lhs_elems) = &lhs.peel_parens().kind {
            let rhs_elems = match rhs.map(|rhs| &rhs.peel_parens().kind) {
                Some(ExprKind::Tuple(elems)) => Some(*elems),
                _ => None,
            };
            for (index, lhs) in lhs_elems.iter().enumerate() {
                let Some(lhs) = lhs else { continue };
                let rhs = rhs_elems.and_then(|elems| elems.get(index)).copied().flatten();
                self.assign_lhs(lhs, rhs);
            }
        } else if let Some(var) = underlying_var(lhs) {
            self.assign_var(var, rhs);
        }
    }

    fn mark_deleted(&mut self, target: &'hir hir::Expr<'hir>) {
        let Some(var) = underlying_var(target) else { return };
        let value = self.fresh_value();
        self.state.values.insert(var, value);
        self.state.low_s.insert(value);
    }

    fn invalidate(&mut self, target: &'hir hir::Expr<'hir>) {
        let Some(var) = underlying_var(target) else { return };
        let value = self.fresh_value();
        self.state.values.insert(var, value);
    }

    fn add_facts(&mut self, predicate: &'hir hir::Expr<'hir>, negate: bool) {
        if expr_has_fact_side_effect(predicate) {
            return;
        }
        match &predicate.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let conjunctive =
                    matches!((op.kind, negate), (BinOpKind::And, false) | (BinOpKind::Or, true));
                let disjunctive =
                    matches!((op.kind, negate), (BinOpKind::Or, false) | (BinOpKind::And, true));
                if conjunctive {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                } else if disjunctive {
                    self.add_disjunctive_facts(lhs, rhs, negate);
                } else {
                    self.add_comparison_fact(lhs, op.kind, rhs, negate);
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.add_facts(inner, !negate);
            }
            _ => {}
        }
    }

    fn add_disjunctive_facts(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        negate: bool,
    ) {
        let baseline = self.state.low_s.clone();
        self.add_facts(lhs, negate);
        let lhs_added: HashSet<_> = self.state.low_s.difference(&baseline).copied().collect();
        self.state.low_s.clone_from(&baseline);
        self.add_facts(rhs, negate);
        let rhs_added: HashSet<_> = self.state.low_s.difference(&baseline).copied().collect();
        self.state.low_s = baseline;
        self.state.low_s.extend(lhs_added.intersection(&rhs_added).copied());
    }

    fn add_comparison_fact(
        &mut self,
        lhs: &'hir hir::Expr<'hir>,
        op: BinOpKind,
        rhs: &'hir hir::Expr<'hir>,
        negate: bool,
    ) {
        let op = if negate { negate_comparison(op) } else { op };
        for (candidate, bound, op) in [(lhs, rhs, op), (rhs, lhs, reverse_comparison(op))] {
            let Some(value) = self.current_value(candidate) else { continue };
            let Some(bound) = self.const_value(bound) else { continue };
            let proves_low = match op {
                BinOpKind::Lt => bound <= SECP256K1_HALF_ORDER + U256::from(1),
                BinOpKind::Le => bound <= SECP256K1_HALF_ORDER,
                BinOpKind::Eq => bound <= SECP256K1_HALF_ORDER,
                _ => false,
            };
            if proves_low {
                self.state.low_s.insert(value);
            }
        }
    }

    fn is_unsafe_ecrecover(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
        if !is_ecrecover_builtin(callee) || args.len() != 4 {
            return false;
        }
        args.exprs().nth(3).is_some_and(|s| !self.is_proven_low_s(s))
    }
}

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                for stmt in block.stmts {
                    let _ = self.visit_stmt(stmt);
                    if branch_always_exits(stmt) {
                        break;
                    }
                }
                return ControlFlow::Continue(());
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                let _ = self.visit_expr(condition);
                let baseline = self.snapshot();

                self.add_facts(condition, false);
                let _ = self.visit_stmt(then_stmt);
                let then_exits = branch_always_exits(then_stmt);
                let after_then = self.snapshot();

                self.restore(baseline);
                self.add_facts(condition, true);
                let else_exits = if let Some(else_stmt) = else_stmt {
                    let _ = self.visit_stmt(else_stmt);
                    branch_always_exits(else_stmt)
                } else {
                    false
                };
                let after_else = self.snapshot();

                let joined = match (then_exits, else_exits) {
                    (true, false) => after_else,
                    (false, true) => after_then,
                    _ => self.join(after_then, after_else),
                };
                self.restore(joined);
                return ControlFlow::Continue(());
            }
            StmtKind::Loop(block, source) => {
                let baseline = self.snapshot();
                for stmt in block.stmts {
                    let _ = self.visit_stmt(stmt);
                    if branch_always_exits(stmt) {
                        break;
                    }
                }
                if !matches!(source, LoopSource::DoWhile) {
                    let after_loop = self.snapshot();
                    let joined = self.join(baseline, after_loop);
                    self.restore(joined);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let baseline = self.snapshot();
                let mut fallthrough = Vec::new();
                for clause in stmt_try.clauses {
                    self.restore(baseline.clone());
                    for stmt in clause.block.stmts {
                        let _ = self.visit_stmt(stmt);
                        if branch_always_exits(stmt) {
                            break;
                        }
                    }
                    if !clause.block.stmts.iter().any(branch_always_exits) {
                        fallthrough.push(self.snapshot());
                    }
                }
                let joined = fallthrough
                    .into_iter()
                    .reduce(|left, right| self.join(left, right))
                    .unwrap_or(baseline);
                self.restore(joined);
                return ControlFlow::Continue(());
            }
            StmtKind::DeclSingle(var) => {
                let init = self.hir.variable(*var).initializer;
                if let Some(init) = init {
                    let _ = self.visit_expr(init);
                }
                self.assign_var(*var, init);
                return ControlFlow::Continue(());
            }
            StmtKind::DeclMulti(vars, init) => {
                let _ = self.visit_expr(init);
                if let ExprKind::Tuple(exprs) = &init.peel_parens().kind {
                    for (var, expr) in vars.iter().zip(exprs.iter()) {
                        if let Some(var) = var {
                            self.assign_var(*var, *expr);
                        }
                    }
                } else {
                    for var in vars.iter().flatten() {
                        self.assign_var(*var, None);
                    }
                }
                return ControlFlow::Continue(());
            }
            StmtKind::Err(_) | StmtKind::AssemblyBlock(_) => {
                self.state = FlowState::default();
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::And | BinOpKind::Or)
        {
            let _ = self.visit_expr(lhs);
            let skipped_rhs = self.snapshot();
            self.add_facts(lhs, op.kind == BinOpKind::Or);
            let _ = self.visit_expr(rhs);
            let ran_rhs = self.snapshot();
            let joined = self.join(skipped_rhs, ran_rhs);
            self.restore(joined);
            return ControlFlow::Continue(());
        }
        if let ExprKind::Ternary(condition, then_expr, else_expr) = &expr.kind {
            let _ = self.visit_expr(condition);
            let baseline = self.snapshot();
            self.add_facts(condition, false);
            let _ = self.visit_expr(then_expr);
            let after_then = self.snapshot();
            self.restore(baseline);
            self.add_facts(condition, true);
            let _ = self.visit_expr(else_expr);
            let after_else = self.snapshot();
            let joined = self.join(after_then, after_else);
            self.restore(joined);
            return ControlFlow::Continue(());
        }

        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let result = self.walk_expr(expr);
                if let Some(condition) = args.exprs().next() {
                    self.add_facts(condition, false);
                }
                return result;
            }
            ExprKind::Assign(lhs, op, rhs) => {
                let result = self.walk_expr(expr);
                self.assign_lhs(lhs, op.is_none().then_some(*rhs));
                return result;
            }
            ExprKind::Delete(target) => {
                let result = self.walk_expr(expr);
                self.mark_deleted(target);
                return result;
            }
            ExprKind::Unary(op, target)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) =>
            {
                let result = self.walk_expr(expr);
                self.invalidate(target);
                return result;
            }
            _ => {}
        }

        let result = self.walk_expr(expr);
        if self.is_unsafe_ecrecover(expr) {
            self.hits.push(expr.span);
        }
        result
    }
}

fn underlying_var(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
            Res::Item(ItemId::Variable(var)) => Some(*var),
            _ => None,
        }),
        ExprKind::Call(callee, args, _)
            if is_transparent_signature_cast(callee) && args.len() == 1 =>
        {
            args.exprs().next().and_then(underlying_var)
        }
        _ => None,
    }
}

fn is_transparent_signature_cast(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(
                ElementaryType::UInt(size) | ElementaryType::FixedBytes(size)
            ),
            ..
        }) if size.bits() == 256
    )
}

fn is_ecrecover_builtin(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| matches!(res, Res::Builtin(Builtin::EcRecover)))
    )
}

fn negate_comparison(op: BinOpKind) -> BinOpKind {
    match op {
        BinOpKind::Lt => BinOpKind::Ge,
        BinOpKind::Le => BinOpKind::Gt,
        BinOpKind::Gt => BinOpKind::Le,
        BinOpKind::Ge => BinOpKind::Lt,
        BinOpKind::Eq => BinOpKind::Ne,
        BinOpKind::Ne => BinOpKind::Eq,
        _ => op,
    }
}

fn reverse_comparison(op: BinOpKind) -> BinOpKind {
    match op {
        BinOpKind::Lt => BinOpKind::Gt,
        BinOpKind::Le => BinOpKind::Ge,
        BinOpKind::Gt => BinOpKind::Lt,
        BinOpKind::Ge => BinOpKind::Le,
        _ => op,
    }
}

fn expr_has_fact_side_effect(expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Assign(..) | ExprKind::Delete(_) => true,
        ExprKind::Unary(op, inner) => {
            matches!(
                op.kind,
                UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
            ) || expr_has_fact_side_effect(inner)
        }
        ExprKind::Array(exprs) => exprs.iter().any(expr_has_fact_side_effect),
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_fact_side_effect(lhs) || expr_has_fact_side_effect(rhs)
        }
        ExprKind::Call(callee, args, options) => {
            expr_has_fact_side_effect(callee)
                || args.exprs().any(expr_has_fact_side_effect)
                || options.is_some_and(|options| {
                    options.args.iter().any(|arg| expr_has_fact_side_effect(&arg.value))
                })
        }
        ExprKind::Index(base, index) => {
            expr_has_fact_side_effect(base) || index.is_some_and(expr_has_fact_side_effect)
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_fact_side_effect(base)
                || start.is_some_and(expr_has_fact_side_effect)
                || end.is_some_and(expr_has_fact_side_effect)
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::YulMember(base, _) => {
            expr_has_fact_side_effect(base)
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            expr_has_fact_side_effect(condition)
                || expr_has_fact_side_effect(then_expr)
                || expr_has_fact_side_effect(else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_has_fact_side_effect(expr))
        }
        ExprKind::Lit(_)
        | ExprKind::Ident(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}
