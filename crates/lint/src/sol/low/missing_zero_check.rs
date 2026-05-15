use super::MissingZeroCheck;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{data_structures::Never, kw, sym},
    sema::hir::{self, ElementaryType, ExprKind, ItemId, Res, StmtKind, TypeKind, Visit},
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
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
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !is_entry_point(func) {
            return;
        }

        let params: HashSet<hir::VariableId> =
            func.parameters.iter().copied().filter(|id| is_address(hir, *id)).collect();

        if params.is_empty() {
            return;
        }

        let Some(body) = func.body else { return };

        let mut a = Analyzer::new(hir, &params);

        for m in func.modifiers {
            collect_modifier_guards(hir, m, &params, &mut a.guarded);
        }

        for stmt in body.stmts {
            let _ = a.visit_stmt(stmt);
        }

        for &p in &params {
            if a.sinks.contains(&p) {
                ctx.emit(&MISSING_ZERO_CHECK, hir.variable(p).span);
            }
        }
    }
}

/// Externally callable, state-mutating functions and constructors.
fn is_entry_point(func: &hir::Function<'_>) -> bool {
    if matches!(func.state_mutability, ast::StateMutability::Pure | ast::StateMutability::View) {
        return false;
    }
    if func.is_constructor() {
        return true;
    }
    func.kind.is_function()
        && matches!(func.visibility, ast::Visibility::Public | ast::Visibility::External)
}

fn is_address(hir: &hir::Hir<'_>, id: hir::VariableId) -> bool {
    matches!(hir.variable(id).ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

/// Tracks address-parameter taint, sinks reached, and guards observed in a function body.
struct Analyzer<'hir> {
    hir: &'hir hir::Hir<'hir>,
    /// Variables transitively derived from candidate parameters, mapped to their sources.
    /// Each parameter is initially mapped to itself.
    taint: HashMap<hir::VariableId, HashSet<hir::VariableId>>,
    /// Source parameters that reached a sink.
    sinks: HashSet<hir::VariableId>,
    /// Source parameters read inside an `if`/`require`/`assert` predicate.
    guarded: HashSet<hir::VariableId>,
    guard_depth: u32,
    sink_depth: u32,
}

impl<'hir> Analyzer<'hir> {
    fn new(hir: &'hir hir::Hir<'hir>, params: &HashSet<hir::VariableId>) -> Self {
        let mut taint = HashMap::with_capacity(params.len());
        for &p in params {
            taint.insert(p, HashSet::from([p]));
        }
        Self {
            hir,
            taint,
            sinks: HashSet::new(),
            guarded: HashSet::new(),
            guard_depth: 0,
            sink_depth: 0,
        }
    }

    fn taint_sources(&self, expr: &hir::Expr<'_>) -> HashSet<hir::VariableId> {
        let mut out = HashSet::new();
        collect_taint_sources(&self.taint, expr, &mut out);
        out
    }
}

fn collect_taint_sources(
    taint: &HashMap<hir::VariableId, HashSet<hir::VariableId>>,
    expr: &hir::Expr<'_>,
    out: &mut HashSet<hir::VariableId>,
) {
    match &expr.kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(vid)) = res
                    && let Some(srcs) = taint.get(vid)
                {
                    out.extend(srcs.iter().copied());
                }
            }
        }
        ExprKind::Assign(_, _, rhs) => collect_taint_sources(taint, rhs, out),
        ExprKind::Binary(lhs, _, rhs) => {
            collect_taint_sources(taint, lhs, out);
            collect_taint_sources(taint, rhs, out);
        }
        ExprKind::Unary(_, e)
        | ExprKind::Delete(e)
        | ExprKind::Member(e, _)
        | ExprKind::Payable(e) => collect_taint_sources(taint, e, out),
        ExprKind::Ternary(_, t, f) => {
            collect_taint_sources(taint, t, out);
            collect_taint_sources(taint, f, out);
        }
        ExprKind::Tuple(elems) => {
            for e in elems.iter().copied().flatten() {
                collect_taint_sources(taint, e, out);
            }
        }
        ExprKind::Array(elems) => {
            for e in *elems {
                collect_taint_sources(taint, e, out);
            }
        }
        ExprKind::Index(base, idx) => {
            collect_taint_sources(taint, base, out);
            if let Some(i) = idx {
                collect_taint_sources(taint, i, out);
            }
        }
        // Covers type casts (`address(x)`), method calls, and ordinary calls; conservative.
        ExprKind::Call(callee, args, _) => {
            collect_taint_sources(taint, callee, out);
            for a in args.exprs() {
                collect_taint_sources(taint, a, out);
            }
        }
        _ => {}
    }
}

/// Returns the underlying local `VariableId` if `lhs` is a direct identifier reference to a
/// non-state variable.
fn lhs_local_var(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> Option<hir::VariableId> {
    if let ExprKind::Ident(reses) = &lhs.kind {
        for res in *reses {
            if let Res::Item(ItemId::Variable(vid)) = res
                && !hir.variable(*vid).kind.is_state()
            {
                return Some(*vid);
            }
        }
    }
    None
}

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir hir::Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match stmt.kind {
            StmtKind::If(cond, then, else_) => {
                self.guard_depth += 1;
                let _ = self.visit_expr(cond);
                self.guard_depth -= 1;

                let baseline = self.guarded.clone();
                let _ = self.visit_stmt(then);
                let then_added: HashSet<hir::VariableId> =
                    self.guarded.difference(&baseline).copied().collect();
                let then_exits = branch_always_exits(then);

                let (else_added, else_exits) = if let Some(e) = else_ {
                    self.guarded = baseline.clone();
                    let _ = self.visit_stmt(e);
                    let added: HashSet<hir::VariableId> =
                        self.guarded.difference(&baseline).copied().collect();
                    (added, branch_always_exits(e))
                } else {
                    (HashSet::new(), false)
                };

                self.guarded = baseline;
                let to_add: HashSet<hir::VariableId> = match (then_exits, else_exits) {
                    (true, true) => then_added.union(&else_added).copied().collect(),
                    (true, false) => else_added,
                    (false, true) => then_added,
                    (false, false) => then_added.intersection(&else_added).copied().collect(),
                };
                self.guarded.extend(to_add);

                return ControlFlow::Continue(());
            }
            // Loop bodies may execute zero times, so guards inside must not persist.
            StmtKind::Loop(block, _) => {
                let baseline = self.guarded.clone();
                for s in block.stmts {
                    let _ = self.visit_stmt(s);
                }
                self.guarded = baseline;
                return ControlFlow::Continue(());
            }
            // Each try/catch clause is taken on a single path; discard clause-local guards.
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                for clause in t.clauses {
                    let baseline = self.guarded.clone();
                    for s in clause.block.stmts {
                        let _ = self.visit_stmt(s);
                    }
                    self.guarded = baseline;
                }
                return ControlFlow::Continue(());
            }
            // Propagate taint through address-typed local declarations only; this avoids
            // marking unrelated values (e.g. `bool ok = a.send(1)`) as derived from `a`.
            StmtKind::DeclSingle(var_id) => {
                let v = self.hir.variable(var_id);
                if let Some(init) = v.initializer
                    && is_address(self.hir, var_id)
                {
                    let srcs = self.taint_sources(init);
                    if !srcs.is_empty() {
                        self.taint.entry(var_id).or_default().extend(srcs);
                    }
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
                    let _ = self.visit_call_args(args);
                    return ControlFlow::Continue(());
                }
            }

            ExprKind::Assign(lhs, _, rhs) => {
                // Sink: assignment to an address state variable.
                if is_address_state_var_lhs(self.hir, lhs) {
                    let _ = self.visit_expr(lhs);
                    self.sink_depth += 1;
                    let _ = self.visit_expr(rhs);
                    self.sink_depth -= 1;
                    return ControlFlow::Continue(());
                }
                // Taint propagation: assignment to an address local.
                if let Some(local) = lhs_local_var(self.hir, lhs)
                    && is_address(self.hir, local)
                {
                    let srcs = self.taint_sources(rhs);
                    if !srcs.is_empty() {
                        self.taint.entry(local).or_default().extend(srcs);
                    }
                }
            }

            // Identifier reads contribute to whichever contexts are currently active.
            ExprKind::Ident(reses) => {
                for res in *reses {
                    if let Res::Item(ItemId::Variable(vid)) = res
                        && let Some(srcs) = self.taint.get(vid)
                    {
                        if self.guard_depth > 0 {
                            self.guarded.extend(srcs.iter().copied());
                        }
                        if self.sink_depth > 0 {
                            for &src in srcs {
                                if !self.guarded.contains(&src) {
                                    self.sinks.insert(src);
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }
        self.walk_expr(expr)
    }
}

fn is_require_or_assert(callee: &hir::Expr<'_>) -> bool {
    if let ExprKind::Ident(reses) = &callee.kind {
        return reses.iter().any(|r| {
            if let Res::Builtin(b) = r {
                let n = b.name();
                n == sym::require || n == sym::assert
            } else {
                false
            }
        });
    }
    false
}

/// If `callee` is `<receiver>.{call,delegatecall,transfer,send}` (with or without
/// call options), returns the `<receiver>` expression.
fn address_call_receiver<'hir>(callee: &'hir hir::Expr<'hir>) -> Option<&'hir hir::Expr<'hir>> {
    // `addr.call{value: x}(..)` lowers as `Call(Member(receiver, "call"), ..)` — peel an
    // outer call layer so the inner Member is reachable.
    let inner = match &callee.kind {
        ExprKind::Call(inner, ..) => inner,
        _ => callee,
    };
    let target = if matches!(inner.kind, ExprKind::Member(..)) { inner } else { callee };
    if let ExprKind::Member(receiver, name) = &target.kind {
        let n = name.name;
        if n == kw::Call || n == kw::Delegatecall || n == sym::transfer || n == sym::send {
            return Some(receiver);
        }
    }
    None
}

fn branch_always_exits(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.last().is_some_and(branch_always_exits)
        }
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        _ => false,
    }
}

fn is_address_state_var_lhs(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> bool {
    if let ExprKind::Ident(reses) = &lhs.kind {
        for res in *reses {
            if let Res::Item(ItemId::Variable(vid)) = res {
                let v = hir.variable(*vid);
                if v.kind.is_state()
                    && matches!(v.ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Maps each direct-ident modifier argument back to its caller-side parameter, runs the same guard
/// analysis on the modifier body, and records any caller params whose mapped modifier parameter is
/// guarded.
fn collect_modifier_guards(
    hir: &hir::Hir<'_>,
    invocation: &hir::Modifier<'_>,
    caller_params: &HashSet<hir::VariableId>,
    guarded: &mut HashSet<hir::VariableId>,
) {
    let ItemId::Function(fid) = invocation.id else { return };
    let modifier = hir.function(fid);
    if !matches!(modifier.kind, hir::FunctionKind::Modifier) {
        return;
    }

    let mod_params = modifier.parameters;
    let mut mapping: HashSet<hir::VariableId> = HashSet::new();
    let mut caller_for_modparam: HashMap<hir::VariableId, hir::VariableId> = HashMap::new();
    for (i, arg_expr) in invocation.args.exprs().enumerate() {
        if let ExprKind::Ident(reses) = &arg_expr.kind {
            for res in *reses {
                if let Res::Item(ItemId::Variable(vid)) = res
                    && caller_params.contains(vid)
                    && let Some(&mp) = mod_params.get(i)
                {
                    caller_for_modparam.insert(mp, *vid);
                    mapping.insert(mp);
                }
            }
        }
    }
    if mapping.is_empty() {
        return;
    }

    let Some(body) = modifier.body else { return };
    let mut a = Analyzer::new(hir, &mapping);
    for stmt in body.stmts {
        let _ = a.visit_stmt(stmt);
    }

    for mp in a.guarded {
        if let Some(caller_vid) = caller_for_modparam.get(&mp) {
            guarded.insert(*caller_vid);
        }
    }
}
