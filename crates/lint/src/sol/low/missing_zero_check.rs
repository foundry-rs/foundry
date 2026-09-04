use super::MissingZeroCheck;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            address_call_receiver, branch_always_exits, is_address_type, is_require_or_assert,
            lhs_local_var, loop_stmts, underlying_var,
        },
    },
};
use solar::{
    ast,
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

impl<'hir> LateLintPass<'hir> for MissingZeroCheck {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        let hir = &gcx.hir;
        let is_entry_point = !matches!(
            func.state_mutability,
            ast::StateMutability::Pure | ast::StateMutability::View
        ) && (func.is_constructor()
            || (func.kind.is_function()
                && matches!(func.visibility, ast::Visibility::Public | ast::Visibility::External)));
        let Some(body) = func.body.filter(|_| is_entry_point) else { return };

        let params: HashSet<_> =
            func.parameters.iter().copied().filter(|&id| is_address_type(hir, id)).collect();
        if params.is_empty() {
            return;
        }

        let mut a = Analyzer::new(gcx, &params);
        for m in func.modifiers {
            let Some(modifier_id) = m.id.as_function() else { continue };
            let modifier = hir.function(modifier_id);
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
                ctx.emit(&MISSING_ZERO_CHECK, hir.variable(p).span);
            }
        }
    }
}

/// Tracks address-parameter taint, sinks reached, and guards observed in a function body.
struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    /// Variables transitively derived from candidate parameters, mapped to their sources.
    /// Each parameter is initially mapped to itself.
    taint: HashMap<VariableId, HashSet<VariableId>>,
    /// Source parameters that reached a sink.
    sinks: HashSet<VariableId>,
    /// Source parameters read inside an `if`/`require`/`assert` predicate.
    guarded: HashSet<VariableId>,
    guard_depth: u32,
    sink_depth: u32,
}

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, params: &HashSet<VariableId>) -> Self {
        Self {
            gcx,
            taint: params.iter().map(|&p| (p, HashSet::from([p]))).collect(),
            sinks: HashSet::new(),
            guarded: HashSet::new(),
            guard_depth: 0,
            sink_depth: 0,
        }
    }

    fn visit_stmts(&mut self, stmts: impl IntoIterator<Item = &'hir hir::Stmt<'hir>>) {
        for s in stmts {
            let _ = self.visit_stmt(s);
        }
    }

    /// Guards added while visiting `stmts`, leaving `self.guarded` untouched.
    fn scoped_guards(
        &mut self,
        stmts: impl IntoIterator<Item = &'hir hir::Stmt<'hir>>,
    ) -> HashSet<VariableId> {
        let baseline = self.guarded.clone();
        self.visit_stmts(stmts);
        let added = &self.guarded - &baseline;
        self.guarded = baseline;
        added
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

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            StmtKind::If(cond, then, else_) => {
                self.guard_depth += 1;
                let _ = self.visit_expr(cond);
                self.guard_depth -= 1;

                let then_added = self.scoped_guards(slice::from_ref(then));
                let else_added =
                    else_.map(|e| self.scoped_guards(slice::from_ref(e))).unwrap_or_default();
                // A guard in an exiting branch holds for everything after the `if`; otherwise it
                // must hold on both branches.
                let then_exits = branch_always_exits(then);
                let else_exits = else_.is_some_and(branch_always_exits);
                let to_add: HashSet<_> = match (then_exits, else_exits) {
                    (true, true) => &then_added | &else_added,
                    (true, false) => else_added,
                    (false, true) => then_added,
                    (false, false) => &then_added & &else_added,
                };
                self.guarded.extend(to_add);
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

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // `require(cond, ..)` / `assert(cond)`: only the first arg is a guard predicate.
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let mut iter = args.exprs();
                if let Some(cond) = iter.next() {
                    self.guard_depth += 1;
                    let _ = self.visit_expr(cond);
                    self.guard_depth -= 1;
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
            // Identifier reads contribute to whichever contexts are currently active.
            ExprKind::Ident(_) => {
                if let Some(srcs) = underlying_var(expr).and_then(|v| self.taint.get(&v)) {
                    if self.guard_depth > 0 {
                        self.guarded.extend(srcs);
                    }
                    if self.sink_depth > 0 {
                        self.sinks.extend(srcs.iter().filter(|src| !self.guarded.contains(src)));
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}
