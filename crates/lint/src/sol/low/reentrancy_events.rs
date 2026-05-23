use super::{
    ReentrancyEvents,
    calls_loop::{is_state_mutating_external_call, resolved_internal_function_ids},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::LitKind,
    interface::{Span, kw, sym},
    sema::{
        Gcx,
        hir::{self, Block, Expr, ExprKind, Function, FunctionId, Hir, Res, Stmt, StmtKind},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    REENTRANCY_EVENTS,
    Severity::Low,
    "reentrancy-events",
    "event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on"
);

impl<'hir> LateLintPass<'hir> for ReentrancyEvents {
    fn check_function_with_gcx(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, gcx, hir);
        let _ = analyzer.analyze_callable(func, body, FlowState::default());
    }
}

type Placeholder<'hir> = Option<(&'hir [hir::Modifier<'hir>], usize, Block<'hir>)>;

/// Per-path state tracked by the may-analysis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FlowState {
    /// True iff an external call has occurred on the path leading to the current program point.
    external_call_seen: bool,
}

impl FlowState {
    const fn merge(&mut self, other: &Self) {
        self.external_call_seen |= other.external_call_seen;
    }
}

/// Summarises how a piece of code can exit, with the [`FlowState`] reaching each exit kind.
/// `None` means no path produces that exit; `Some(_)` means some path does.
///
/// Aborting paths (`revert`/`require(false)`/etc.) drop their state — they are simply absent
/// from every bucket, so they cannot taint subsequent statements.
#[derive(Clone, Debug, Default)]
struct Exits {
    /// Control falls through to the next statement of the enclosing block.
    fallthrough: Option<FlowState>,
    /// Control exits the enclosing function via `return`.
    return_: Option<FlowState>,
    /// Control exits the enclosing loop via `break`.
    break_: Option<FlowState>,
    /// Control goes back to the loop header via `continue`.
    continue_: Option<FlowState>,
}

impl Exits {
    fn fallthrough(state: FlowState) -> Self {
        Self { fallthrough: Some(state), ..Default::default() }
    }

    fn return_(state: FlowState) -> Self {
        Self { return_: Some(state), ..Default::default() }
    }

    fn break_(state: FlowState) -> Self {
        Self { break_: Some(state), ..Default::default() }
    }

    fn continue_(state: FlowState) -> Self {
        Self { continue_: Some(state), ..Default::default() }
    }

    /// Aborting exit (`revert`, infinite loop, panic): no paths flow out.
    fn abort() -> Self {
        Self::default()
    }

    const fn merge(&mut self, other: Self) {
        merge_opt(&mut self.fallthrough, other.fallthrough);
        merge_opt(&mut self.return_, other.return_);
        merge_opt(&mut self.break_, other.break_);
        merge_opt(&mut self.continue_, other.continue_);
    }
}

const fn merge_opt(dst: &mut Option<FlowState>, src: Option<FlowState>) {
    match (dst.as_mut(), src) {
        (None, src) => *dst = src,
        (Some(_), None) => {}
        (Some(d), Some(s)) => d.merge(&s),
    }
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    /// Call stack to break recursion when inlining internal helpers and modifiers.
    call_stack: Vec<FunctionId>,
    /// Spans already reported, to dedupe diagnostics across paths/iterations.
    emitted: HashSet<Span>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(ctx: &'ctx LintContext<'s, 'c>, gcx: Gcx<'hir>, hir: &'hir Hir<'hir>) -> Self {
        Self { ctx, gcx, hir, call_stack: Vec::new(), emitted: HashSet::new() }
    }

    fn analyze_callable(
        &mut self,
        func: &'hir Function<'hir>,
        body: Block<'hir>,
        entry: FlowState,
    ) -> Exits {
        self.analyze_modifier_chain(func.modifiers, 0, body, entry)
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: Block<'hir>,
        mut entry: FlowState,
    ) -> Exits {
        let Some(modifier) = modifiers.get(index) else {
            return self.analyze_block(body, None, entry);
        };

        for arg in modifier.args.exprs() {
            self.analyze_expr(arg, &mut entry);
        }

        let Some(modifier_id) = modifier.id.as_function() else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, entry);
        };

        // Note: we deliberately do NOT skip duplicate modifier IDs here. A modifier may
        // legitimately appear at multiple indices in the chain (e.g. `f() m(false) m(true)`),
        // and the chain itself cannot recurse infinitely because `index` strictly increases.
        // True recursion through internal calls is still handled by `analyze_internal_call`.

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, entry);
        };

        self.call_stack.push(modifier_id);
        let summary = self.analyze_block(modifier_body, Some((modifiers, index + 1, body)), entry);
        self.call_stack.pop();
        summary
    }

    fn analyze_block(
        &mut self,
        block: Block<'hir>,
        placeholder: Placeholder<'hir>,
        mut entry: FlowState,
    ) -> Exits {
        let mut summary = Exits::default();
        for stmt in block.stmts {
            let stmt_exits = self.analyze_stmt(stmt, placeholder, entry);
            // Non-fallthrough exits propagate up out of the block.
            merge_opt(&mut summary.return_, stmt_exits.return_);
            merge_opt(&mut summary.break_, stmt_exits.break_);
            merge_opt(&mut summary.continue_, stmt_exits.continue_);
            // Only the fallthrough state reaches the next statement.
            match stmt_exits.fallthrough {
                Some(next) => entry = next,
                None => return summary, // Subsequent statements are dead.
            }
        }
        summary.fallthrough = Some(entry);
        summary
    }

    fn analyze_stmt(
        &mut self,
        stmt: &'hir Stmt<'hir>,
        placeholder: Placeholder<'hir>,
        mut entry: FlowState,
    ) -> Exits {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.analyze_expr(init, &mut entry);
                }
                Exits::fallthrough(entry)
            }
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) => {
                self.analyze_expr(expr, &mut entry);
                // Several Solidity builtins terminate execution but lower to plain expression
                // statements rather than `StmtKind::Revert`: `revert()`/`revert("msg")`,
                // `selfdestruct(...)`, `require(false, ...)`, and `assert(false)`. Treat them
                // as aborts so following statements are not misanalysed as reachable.
                if is_aborting_call(expr) {
                    return Exits::abort();
                }
                Exits::fallthrough(entry)
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, placeholder, entry)
            }
            StmtKind::Emit(expr) => {
                // Solidity evaluates event arguments before emitting, so an external call inside
                // the arguments also taints this emit. Analyze the args first, then check state.
                self.analyze_expr(expr, &mut entry);
                if entry.external_call_seen && self.emitted.insert(stmt.span) {
                    self.ctx.emit(&REENTRANCY_EVENTS, stmt.span);
                }
                Exits::fallthrough(entry)
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, &mut entry);
                Exits::abort()
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, &mut entry);
                }
                Exits::return_(entry)
            }
            StmtKind::Break => Exits::break_(entry),
            StmtKind::Continue => Exits::continue_(entry),
            StmtKind::Loop(block, _) => {
                // Two-pass fixpoint: with a 1-bit state the back-edge can only strengthen
                // `external_call_seen` from false to true, so a second pass with the merged
                // entry suffices to catch emits tainted only on iterations 2..N. Duplicate
                // diagnostics from the first pass are deduped via `self.emitted`.
                let first = self.analyze_block(block, placeholder, entry.clone());

                // Back-edge entry: pre-loop entry merged with anything that loops back
                // (fallthrough at the end of the body, or an explicit `continue`).
                let mut back_edge = entry.clone();
                if let Some(ft) = &first.fallthrough {
                    back_edge.merge(ft);
                }
                if let Some(c) = &first.continue_ {
                    back_edge.merge(c);
                }

                let body = if back_edge == entry {
                    first
                } else {
                    self.analyze_block(block, placeholder, back_edge)
                };

                // Post-loop state combines the entry (zero iterations), fallthrough at end of
                // body, plus any `break` or `continue` exits. Aborting paths are absent and
                // therefore drop out.
                let mut post = entry;
                if let Some(ft) = body.fallthrough {
                    post.merge(&ft);
                }
                if let Some(b) = body.break_ {
                    post.merge(&b);
                }
                if let Some(c) = body.continue_ {
                    post.merge(&c);
                }
                Exits {
                    fallthrough: Some(post),
                    return_: body.return_,
                    break_: None,
                    continue_: None,
                }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, &mut entry);

                let then_exits = self.analyze_stmt(then_stmt, placeholder, entry.clone());
                let else_exits = if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, placeholder, entry)
                } else {
                    Exits::fallthrough(entry)
                };

                let mut merged = then_exits;
                merged.merge(else_exits);
                merged
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, &mut entry);

                let mut summary = Exits::default();
                for clause in try_stmt.clauses {
                    let clause_exits = self.analyze_block(clause.block, placeholder, entry.clone());
                    summary.merge(clause_exits);
                }
                summary
            }
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body)) = placeholder {
                    self.analyze_modifier_chain(modifiers, index, body, entry)
                } else {
                    Exits::fallthrough(entry)
                }
            }
            StmtKind::Err(_) => Exits::fallthrough(entry),
        }
    }

    fn analyze_expr(&mut self, expr: &'hir Expr<'hir>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee, state);
                if let Some(opts) = opts {
                    for opt in *opts {
                        self.analyze_expr(&opt.value, state);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg, state);
                }

                if is_state_mutating_external_call(self.gcx, self.hir, callee, args.len())
                    || is_new_expression(callee)
                {
                    // Deploying a contract (`new Foo(args)`) executes that contract's
                    // constructor, which is an external interaction that may emit events
                    // or call back into this contract.
                    state.external_call_seen = true;
                }

                // Follow internal/private/public helpers transitively so external calls in
                // helpers also taint the caller's flow state.
                for func_id in resolved_internal_function_ids(self.hir, callee) {
                    self.analyze_internal_call(func_id, state);
                }
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs, state);
                self.analyze_expr(rhs, state);
            }
            ExprKind::Unary(_, inner) | ExprKind::Delete(inner) | ExprKind::Payable(inner) => {
                self.analyze_expr(inner, state);
            }
            ExprKind::Index(base, index) => {
                self.analyze_expr(base, state);
                if let Some(index) = index {
                    self.analyze_expr(index, state);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_expr(base, state);
                if let Some(start) = start {
                    self.analyze_expr(start, state);
                }
                if let Some(end) = end {
                    self.analyze_expr(end, state);
                }
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.analyze_expr(cond, state);

                let mut then_state = state.clone();
                self.analyze_expr(then_expr, &mut then_state);

                let mut else_state = state.clone();
                self.analyze_expr(else_expr, &mut else_state);

                *state = then_state;
                state.merge(&else_state);
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.analyze_expr(expr, state);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_expr(expr, state);
                }
            }
            ExprKind::Member(base, _) => self.analyze_expr(base, state),
            ExprKind::Ident(_)
            | ExprKind::Lit(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(&mut self, func_id: FunctionId, state: &mut FlowState) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        let summary = self.analyze_callable(func, body, state.clone());
        self.call_stack.pop();

        // Caller only inherits the state of paths that return normally to it (implicit
        // fallthrough at the end of the body, or explicit `return`). If every path inside the
        // callee aborts (e.g. always `revert`s), the caller's state is left unchanged here —
        // post-call code is technically dead, but we cannot signal that through the expression
        // visitor. This is a documented limitation.
        let any_normal = summary.fallthrough.is_some() || summary.return_.is_some();
        if any_normal {
            let mut after = FlowState::default();
            if let Some(ft) = summary.fallthrough {
                after.merge(&ft);
            }
            if let Some(rt) = summary.return_ {
                after.merge(&rt);
            }
            *state = after;
        }
    }
}

/// Returns `true` when the expression-statement is a builtin call that always terminates
/// execution: `revert()` / `revert("msg")`, `selfdestruct(...)`, `require(false, ...)`, or
/// `assert(false)`.
fn is_aborting_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else {
        return false;
    };
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else {
        return false;
    };
    for res in *reses {
        let Res::Builtin(b) = res else { continue };
        let name = b.name();
        if name == kw::Revert || name == kw::Selfdestruct {
            return true;
        }
        if (name == sym::require || name == sym::assert)
            && args.exprs().next().is_some_and(literal_false)
        {
            return true;
        }
    }
    false
}

/// Returns `true` if `expr` is the boolean literal `false`.
fn literal_false(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Bool(false))
    )
}

/// Returns `true` if `callee` is a `new` expression — i.e. the call is a contract deployment
/// (`new Foo(args)`), which executes the deployed contract's constructor and is an external
/// interaction for our purposes.
fn is_new_expression(callee: &Expr<'_>) -> bool {
    matches!(callee.peel_parens().kind, ExprKind::New(_))
}
