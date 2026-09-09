use super::MissingZeroCheck;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            address_call_receiver, branch_always_exits, is_address_type, is_require_or_assert,
            is_zero_value, lhs_local_var, loop_stmts, underlying_var,
        },
    },
};
use solar::{
    ast::{self, BinOpKind, UnOpKind},
    interface::data_structures::Never,
    sema::{
        Gcx,
        hir::{self, ExprKind, StmtKind, VariableId, Visit},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    slice,
};

declare_forge_lint!(
    MISSING_ZERO_CHECK,
    Severity::Low,
    "missing-zero-check",
    "address parameter is used in a state write or value transfer without a zero-address check"
);

impl<'gcx> LateLintPass<'gcx> for MissingZeroCheck {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        let is_entry_point = !matches!(
            func.state_mutability,
            ast::StateMutability::Pure | ast::StateMutability::View
        ) && (func.is_constructor()
            || (func.kind.is_function()
                && matches!(func.visibility, ast::Visibility::Public | ast::Visibility::External)));
        let Some(body) = func.body.filter(|_| is_entry_point) else { return };

        let params: HashSet<_> =
            func.parameters.iter().copied().filter(|&id| is_address_type(&gcx.hir, id)).collect();
        if params.is_empty() {
            return;
        }

        let mut a = Analyzer::new(gcx, &params);
        for m in func.modifiers {
            let Some(modifier_id) = m.id.as_function() else { continue };
            let modifier = gcx.hir.function(modifier_id);
            // Map each direct-ident argument back to the caller's parameter and analyze the
            // modifier body as if it were a prefix of the function.
            let mapping: HashMap<_, _> = modifier
                .parameters
                .iter()
                .zip(m.args.exprs())
                .filter_map(|(&mp, arg)| {
                    let caller = underlying_var(arg).filter(|v| params.contains(v))?;
                    Some((mp, caller))
                })
                .collect();
            if let Some(body) = modifier.body.filter(|_| !mapping.is_empty()) {
                let mut ma = Analyzer::new(gcx, &mapping.keys().copied().collect());
                ma.visit_stmts(body.stmts);
                a.guarded.extend(ma.guarded.iter().filter_map(|mp| mapping.get(mp)));
            }
        }
        a.visit_stmts(body.stmts);

        for &p in &params {
            if a.sinks.contains(&p) {
                ctx.emit(&MISSING_ZERO_CHECK, gcx.hir.variable(p).span);
            }
        }
    }
}

/// Tracks address-parameter taint, sinks reached, and guards observed in a function body.
struct Analyzer<'gcx> {
    gcx: Gcx<'gcx>,
    /// Variables transitively derived from candidate parameters, mapped to their sources.
    /// Each parameter is initially mapped to itself.
    taint: HashMap<VariableId, HashSet<VariableId>>,
    /// Source parameters that reached a sink.
    sinks: HashSet<VariableId>,
    /// Source parameters proven non-zero so far.
    guarded: HashSet<VariableId>,
    sink_depth: u32,
}

impl<'gcx> Analyzer<'gcx> {
    fn new(gcx: Gcx<'gcx>, params: &HashSet<VariableId>) -> Self {
        Self {
            gcx,
            taint: params.iter().map(|&p| (p, HashSet::from([p]))).collect(),
            sinks: HashSet::new(),
            guarded: HashSet::new(),
            sink_depth: 0,
        }
    }

    fn visit_stmts(&mut self, stmts: impl IntoIterator<Item = &'gcx hir::Stmt<'gcx>>) {
        for s in stmts {
            let _ = self.visit_stmt(s);
        }
    }

    /// Visits `stmts` without letting their guards escape.
    fn scoped_guards(
        &mut self,
        stmts: impl IntoIterator<Item = &'gcx hir::Stmt<'gcx>>,
    ) -> HashSet<VariableId> {
        let baseline = self.guarded.clone();
        self.visit_stmts(stmts);
        std::mem::replace(&mut self.guarded, baseline)
    }

    /// Sources proven non-zero when `pred` evaluates to `!negate`.
    fn nonzero_facts(&self, pred: &'gcx hir::Expr<'gcx>, negate: bool) -> HashSet<VariableId> {
        let nonzero_cmp = if negate { BinOpKind::Eq } else { BinOpKind::Ne };
        match &pred.peel_parens().kind {
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.nonzero_facts(inner, !negate)
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let lhs = self.nonzero_facts(lhs, negate);
                let rhs = self.nonzero_facts(rhs, negate);
                if matches!((op.kind, negate), (BinOpKind::And, false) | (BinOpKind::Or, true)) {
                    &lhs | &rhs
                } else {
                    &lhs & &rhs
                }
            }
            ExprKind::Binary(lhs, op, rhs) if op.kind == nonzero_cmp => {
                let mut facts = HashSet::new();
                for (candidate, zero) in [(lhs, rhs), (rhs, lhs)] {
                    if is_zero_value(zero)
                        && let Some(sources) =
                            underlying_var(candidate).and_then(|v| self.taint.get(&v))
                    {
                        facts.extend(sources);
                    }
                }
                facts
            }
            _ => HashSet::new(),
        }
    }

    fn taint_sources(&self, expr: &hir::Expr<'_>) -> HashSet<VariableId> {
        let mut out = HashSet::new();
        let _ = expr.visit(&mut |e| {
            if let Some(srcs) = underlying_var(e).and_then(|v| self.taint.get(&v)) {
                out.extend(srcs);
            }
            ControlFlow::<Never>::Continue(())
        });
        out
    }

    fn propagate(&mut self, local: VariableId, value: &hir::Expr<'_>) {
        // Propagate taint through address-typed locals only; this avoids marking unrelated
        // values (e.g. `bool ok = a.send(1)`) as derived from `a`.
        if is_address_type(&self.gcx.hir, local) {
            let srcs = self.taint_sources(value);
            if !srcs.is_empty() {
                self.taint.entry(local).or_default().extend(srcs);
            }
        }
    }
}

impl<'gcx> Visit<'gcx> for Analyzer<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx hir::Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.guarded.clone();

                self.guarded.extend(self.nonzero_facts(cond, false));
                let then_guards = self.scoped_guards(slice::from_ref(then));

                self.guarded = baseline;
                self.guarded.extend(self.nonzero_facts(cond, true));
                let else_guards = else_
                    .map(|e| self.scoped_guards(slice::from_ref(e)))
                    .unwrap_or_else(|| self.guarded.clone());

                // A guard in an exiting branch holds for everything after the `if`; otherwise it
                // must hold on both branches.
                let then_exits = branch_always_exits(then);
                let else_exits = else_.is_some_and(branch_always_exits);
                self.guarded = match (then_exits, else_exits) {
                    (true, true) => &then_guards | &else_guards,
                    (true, false) => else_guards,
                    (false, true) => then_guards,
                    (false, false) => &then_guards & &else_guards,
                };
                return ControlFlow::Continue(());
            }
            // Loop bodies may execute zero times, so guards inside must not persist.
            StmtKind::Loop(block, source) => {
                self.scoped_guards(loop_stmts(block, source));
                return ControlFlow::Continue(());
            }
            // Each try/catch clause is taken on a single path; discard clause-local guards.
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                for clause in t.clauses {
                    self.scoped_guards(clause.block.stmts);
                }
                return ControlFlow::Continue(());
            }
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
                    self.propagate(var_id, init);
                }
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // `require(cond, ..)` / `assert(cond)`: only the first arg is a guard predicate.
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let mut iter = args.exprs();
                if let Some(cond) = iter.next() {
                    self.guarded.extend(self.nonzero_facts(cond, false));
                    let _ = self.visit_expr(cond);
                }
                for rest in iter {
                    let _ = self.visit_expr(rest);
                }
                return ControlFlow::Continue(());
            }
            // `<addr>.call/.delegatecall/.transfer/.send(..)`: receiver is the sink.
            ExprKind::Call(callee, args, _) => {
                if let Some(receiver) = address_call_receiver(callee) {
                    self.sink_depth += 1;
                    let _ = self.visit_expr(receiver);
                    self.sink_depth -= 1;
                    return self.visit_call_args(args);
                }
            }
            ExprKind::Assign(lhs, _, rhs) => {
                // Sink: assignment to an address state variable.
                if let Some(v) = underlying_var(lhs)
                    && self.gcx.hir.variable(v).kind.is_state()
                    && is_address_type(&self.gcx.hir, v)
                {
                    self.sink_depth += 1;
                    let _ = self.visit_expr(rhs);
                    self.sink_depth -= 1;
                    return ControlFlow::Continue(());
                }
                if let Some(local) = lhs_local_var(&self.gcx.hir, lhs) {
                    self.propagate(local, rhs);
                }
            }
            ExprKind::Ident(_) => {
                if self.sink_depth > 0
                    && let Some(srcs) = underlying_var(expr).and_then(|v| self.taint.get(&v))
                {
                    self.sinks.extend(srcs.iter().filter(|src| !self.guarded.contains(src)));
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}
