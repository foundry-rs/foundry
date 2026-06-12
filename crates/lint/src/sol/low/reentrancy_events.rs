use super::{
    ReentrancyEvents,
    calls_loop::{
        is_state_mutating_external_call, resolved_internal_function_ids,
        resolved_super_function_ids,
    },
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::helper_cache::{DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache},
    },
};
use solar::{
    ast::LitKind,
    interface::{Span, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, Block, ContractId, Expr, ExprKind, Function, FunctionId, Hir, Res, Stmt, StmtKind,
        },
    },
};
use std::collections::{HashMap, HashSet};

declare_forge_lint!(
    REENTRANCY_EVENTS,
    Severity::Low,
    "reentrancy-events",
    "event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on"
);

impl<'hir> LateLintPass<'hir> for ReentrancyEvents {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, gcx, hir, func.contract);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    external_call_seen: bool,
    suppress_inline_reports: bool,
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    /// Top-level analyzed contract; used to resolve `this.<method>` without consulting
    /// Solar for the `this` builtin. Held fixed across inlined helpers (runtime `this`).
    enclosing_contract: Option<ContractId>,
    /// Call stack to break recursion when inlining internal helpers and modifiers.
    call_stack: Vec<FunctionId>,
    /// Cached summaries for transitive helper analysis. This keeps shared helper graphs from
    /// being rescanned for every call edge in a function.
    inline_cache: HelperAnalysisCache<InlineCallKey, Exits>,
    /// Cached conservative summaries used only when a recursive edge is cut.
    external_call_reachability: HashMap<FunctionId, bool>,
    /// Spans already reported, to dedupe diagnostics across paths/iterations.
    emitted: HashSet<Span>,
    /// When `true`, suppress emit diagnostics: we are inside an inlined helper that was
    /// entered with a clean state, so the helper's own self-pass will catch any taint.
    suppress_inline_reports: bool,
    /// Set by `analyze_internal_call` when the inlined callee has no normal exits, so the
    /// enclosing statement can treat itself as aborting.
    expr_aborted: bool,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(
        ctx: &'ctx LintContext<'s, 'c>,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        enclosing_contract: Option<ContractId>,
    ) -> Self {
        Self {
            ctx,
            gcx,
            hir,
            enclosing_contract,
            call_stack: Vec::new(),
            inline_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            external_call_reachability: HashMap::new(),
            emitted: HashSet::new(),
            suppress_inline_reports: false,
            expr_aborted: false,
        }
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
            self.expr_aborted = false;
            self.analyze_expr(arg, &mut entry);
            // An aborting arg means the modifier (and therefore its body) is never entered.
            if self.expr_aborted {
                return Exits::abort();
            }
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
        // Reset once per statement so each branch can read `expr_aborted` after analyzing
        // its top-level expressions without leaking state from a previous statement.
        self.expr_aborted = false;
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.analyze_expr(init, &mut entry);
                }
                if self.expr_aborted {
                    return Exits::abort();
                }
                Exits::fallthrough(entry)
            }
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) => {
                self.analyze_expr(expr, &mut entry);
                // Aborts via builtins (`revert()`, `selfdestruct(...)`, `require(false, …)`,
                // `assert(false)`) or via an inlined helper with no normal exit.
                if is_aborting_call(expr) || self.expr_aborted {
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
                // If an argument aborts (e.g. `emit E(helperThatAlwaysReverts())`), the emit
                // itself is unreachable, so it must not be reported and the path aborts.
                if self.expr_aborted {
                    return Exits::abort();
                }
                if entry.external_call_seen
                    && !self.suppress_inline_reports
                    && self.emitted.insert(stmt.span)
                {
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
                // If the return value computation aborts, the `return` itself never runs.
                if self.expr_aborted {
                    return Exits::abort();
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
                // If the condition aborts (e.g. `if (helperThatAlwaysReverts())`), neither
                // branch is reachable.
                if self.expr_aborted {
                    return Exits::abort();
                }

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
                // If evaluating the try-call expression aborts before the call itself runs
                // (e.g. an aborting arg), no clause can execute.
                if self.expr_aborted {
                    return Exits::abort();
                }

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
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                // Inline assembly can perform external interactions
                // (call/delegatecall/create, logs). Conservatively taint.
                entry.external_call_seen = true;
                Exits::fallthrough(entry)
            }
        }
    }

    fn analyze_expr(&mut self, expr: &'hir Expr<'hir>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                self.analyze_expr(callee, state);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        self.analyze_expr(&opt.value, state);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg, state);
                }

                if is_state_mutating_external_call(
                    self.gcx,
                    self.hir,
                    callee,
                    args.len(),
                    self.enclosing_contract,
                ) {
                    state.external_call_seen = true;
                }

                // Follow internal/private/public helpers transitively so external calls in
                // helpers also taint the caller's flow state.
                for func_id in resolved_internal_function_ids(self.hir, callee) {
                    self.analyze_internal_call(func_id, state);
                }
                // Same for `super.<member>(...)` base-chain dispatch.
                for func_id in resolved_super_function_ids(
                    self.hir,
                    self.enclosing_contract,
                    callee,
                    args.len(),
                ) {
                    self.analyze_internal_call(func_id, state);
                }
            }
            ExprKind::Binary(lhs, op, rhs)
                if matches!(op.kind, hir::BinOpKind::And | hir::BinOpKind::Or) =>
            {
                // Short-circuiting `&&`/`||`: LHS always runs, RHS is conditional. Model RHS
                // on a forked state so its taint only reaches the merged result when the
                // short-circuit path is also possible, and so an aborting RHS does not kill
                // the whole expression (the short-circuit path still falls through).
                self.analyze_expr(lhs, state);
                let lhs_aborted = std::mem::replace(&mut self.expr_aborted, false);

                let mut rhs_state = state.clone();
                self.analyze_expr(rhs, &mut rhs_state);
                let rhs_aborted = self.expr_aborted;

                // The expression aborts iff LHS aborts (then no path survives); an
                // RHS-only abort just drops the non-short-circuit path.
                self.expr_aborted = lhs_aborted;

                if !lhs_aborted && !rhs_aborted {
                    state.merge(&rhs_state);
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
                // Sample `expr_aborted` per branch so an aborting branch can't poison the
                // sibling. The ternary aborts iff `cond` aborts OR both branches abort.
                let outer_aborted = std::mem::replace(&mut self.expr_aborted, false);

                let mut then_state = state.clone();
                self.analyze_expr(then_expr, &mut then_state);
                let then_aborted = std::mem::replace(&mut self.expr_aborted, false);

                let mut else_state = state.clone();
                self.analyze_expr(else_expr, &mut else_state);
                let else_aborted = self.expr_aborted;

                self.expr_aborted = outer_aborted || (then_aborted && else_aborted);

                // Aborting branches drop their state; only surviving branches contribute.
                match (then_aborted, else_aborted) {
                    (true, true) => {}
                    (true, false) => *state = else_state,
                    (false, true) => *state = then_state,
                    (false, false) => {
                        *state = then_state;
                        state.merge(&else_state);
                    }
                }
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
            | ExprKind::YulMember(..)
            | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(&mut self, func_id: FunctionId, state: &mut FlowState) {
        if self.call_stack.contains(&func_id) {
            // Keep inline summaries stack-insensitive by replacing cut recursive edges with a
            // conservative cached "can this helper ever taint by external call?" summary.
            if self.helper_may_reach_external_call(func_id) {
                state.external_call_seen = true;
            }
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        let suppress_inline_reports = self.suppress_inline_reports || !state.external_call_seen;
        let key = InlineCallKey {
            func_id,
            external_call_seen: state.external_call_seen,
            suppress_inline_reports,
        };

        if self.inline_cache.is_in_progress(&key) {
            return;
        }

        if let Some(summary) = self.inline_cache.get(&key).cloned() {
            self.apply_inline_summary(summary, state);
            return;
        }

        // Suppress diagnostics inside helpers entered with a clean state — the helper's
        // own self-pass will independently catch any intra-helper taint, avoiding
        // duplicate reports across callers.
        let prev_suppress = self.suppress_inline_reports;
        self.suppress_inline_reports = suppress_inline_reports;

        self.inline_cache.start(key);
        self.call_stack.push(func_id);
        let summary = self.analyze_callable(func, body, state.clone());
        self.call_stack.pop();

        self.inline_cache.finish(key, summary.clone());

        self.suppress_inline_reports = prev_suppress;

        self.apply_inline_summary(summary, state);
    }

    fn apply_inline_summary(&mut self, summary: Exits, state: &mut FlowState) {
        // Caller inherits the state of paths that return normally. If the callee has no
        // normal exits (always aborts), signal abort to the enclosing statement.
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
        } else {
            self.expr_aborted = true;
        }
    }

    fn helper_may_reach_external_call(&mut self, func_id: FunctionId) -> bool {
        self.helper_may_reach_external_call_inner(func_id, &mut HashSet::new()).0
    }

    fn helper_may_reach_external_call_inner(
        &mut self,
        func_id: FunctionId,
        seen: &mut HashSet<FunctionId>,
    ) -> (bool, bool) {
        if let Some(cached) = self.external_call_reachability.get(&func_id) {
            return (*cached, false);
        }
        if !seen.insert(func_id) {
            return (false, true);
        }

        let func = self.hir.function(func_id);
        let mut may_reach = false;
        let mut cut_recursive_edge = false;
        for modifier in func.modifiers {
            for arg in modifier.args.exprs() {
                let (arg_may_reach, arg_cut_recursive_edge) =
                    self.expr_may_reach_external_call(arg, seen);
                cut_recursive_edge |= arg_cut_recursive_edge;
                if arg_may_reach {
                    may_reach = true;
                    break;
                }
            }
            if may_reach {
                break;
            }
            if let Some(modifier_id) = modifier.id.as_function() {
                let (modifier_may_reach, modifier_cut_recursive_edge) =
                    self.helper_may_reach_external_call_inner(modifier_id, seen);
                cut_recursive_edge |= modifier_cut_recursive_edge;
                if modifier_may_reach {
                    may_reach = true;
                    break;
                }
            }
        }
        if !may_reach && let Some(body) = func.body {
            let (body_may_reach, body_cut_recursive_edge) =
                self.block_may_reach_external_call(body, seen);
            may_reach = body_may_reach;
            cut_recursive_edge |= body_cut_recursive_edge;
        }

        seen.remove(&func_id);
        if may_reach || !cut_recursive_edge {
            self.external_call_reachability.insert(func_id, may_reach);
        }
        (may_reach, cut_recursive_edge)
    }

    fn block_may_reach_external_call(
        &mut self,
        block: Block<'hir>,
        seen: &mut HashSet<FunctionId>,
    ) -> (bool, bool) {
        let mut cut_recursive_edge = false;
        for stmt in block.stmts {
            let (may_reach, stmt_cut_recursive_edge) =
                self.stmt_may_reach_external_call(stmt, seen);
            cut_recursive_edge |= stmt_cut_recursive_edge;
            if may_reach {
                return (true, cut_recursive_edge);
            }
        }
        (false, cut_recursive_edge)
    }

    fn stmt_may_reach_external_call(
        &mut self,
        stmt: &'hir Stmt<'hir>,
        seen: &mut HashSet<FunctionId>,
    ) -> (bool, bool) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => match self.hir.variable(var_id).initializer {
                Some(expr) => self.expr_may_reach_external_call(expr, seen),
                None => (false, false),
            },
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Expr(expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr) => self.expr_may_reach_external_call(expr, seen),
            StmtKind::Return(expr) => match expr {
                Some(expr) => self.expr_may_reach_external_call(expr, seen),
                None => (false, false),
            },
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.block_may_reach_external_call(block, seen)
            }
            StmtKind::If(cond, then_stmt, else_stmt) => Self::any_may_reach_external_call([
                self.expr_may_reach_external_call(cond, seen),
                self.stmt_may_reach_external_call(then_stmt, seen),
                else_stmt
                    .map(|else_stmt| self.stmt_may_reach_external_call(else_stmt, seen))
                    .unwrap_or((false, false)),
            ]),
            StmtKind::Try(try_stmt) => {
                let mut cut_recursive_edge = false;
                let (expr_may_reach, expr_cut_recursive_edge) =
                    self.expr_may_reach_external_call(&try_stmt.expr, seen);
                cut_recursive_edge |= expr_cut_recursive_edge;
                if expr_may_reach {
                    return (true, cut_recursive_edge);
                }
                for clause in try_stmt.clauses {
                    let (clause_may_reach, clause_cut_recursive_edge) =
                        self.block_may_reach_external_call(clause.block, seen);
                    cut_recursive_edge |= clause_cut_recursive_edge;
                    if clause_may_reach {
                        return (true, cut_recursive_edge);
                    }
                }
                (false, cut_recursive_edge)
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) => (true, false),
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {
                (false, false)
            }
        }
    }

    fn expr_may_reach_external_call(
        &mut self,
        expr: &'hir Expr<'hir>,
        seen: &mut HashSet<FunctionId>,
    ) -> (bool, bool) {
        match &expr.kind {
            ExprKind::Call(callee, args, opts) => {
                let mut cut_recursive_edge = false;
                let (callee_may_reach, callee_cut_recursive_edge) =
                    self.expr_may_reach_external_call(callee, seen);
                cut_recursive_edge |= callee_cut_recursive_edge;
                if callee_may_reach {
                    return (true, cut_recursive_edge);
                }
                if let Some(opts) = opts {
                    for opt in opts.args {
                        let (opt_may_reach, opt_cut_recursive_edge) =
                            self.expr_may_reach_external_call(&opt.value, seen);
                        cut_recursive_edge |= opt_cut_recursive_edge;
                        if opt_may_reach {
                            return (true, cut_recursive_edge);
                        }
                    }
                }
                for arg in args.exprs() {
                    let (arg_may_reach, arg_cut_recursive_edge) =
                        self.expr_may_reach_external_call(arg, seen);
                    cut_recursive_edge |= arg_cut_recursive_edge;
                    if arg_may_reach {
                        return (true, cut_recursive_edge);
                    }
                }
                if is_state_mutating_external_call(
                    self.gcx,
                    self.hir,
                    callee,
                    args.len(),
                    self.enclosing_contract,
                ) {
                    return (true, cut_recursive_edge);
                }

                let internal: Vec<_> = resolved_internal_function_ids(self.hir, callee).collect();
                for func_id in internal {
                    let (func_may_reach, func_cut_recursive_edge) =
                        self.helper_may_reach_external_call_inner(func_id, seen);
                    cut_recursive_edge |= func_cut_recursive_edge;
                    if func_may_reach {
                        return (true, cut_recursive_edge);
                    }
                }

                for func_id in resolved_super_function_ids(
                    self.hir,
                    self.enclosing_contract,
                    callee,
                    args.len(),
                ) {
                    let (func_may_reach, func_cut_recursive_edge) =
                        self.helper_may_reach_external_call_inner(func_id, seen);
                    cut_recursive_edge |= func_cut_recursive_edge;
                    if func_may_reach {
                        return (true, cut_recursive_edge);
                    }
                }

                (false, cut_recursive_edge)
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                Self::any_may_reach_external_call([
                    self.expr_may_reach_external_call(lhs, seen),
                    self.expr_may_reach_external_call(rhs, seen),
                ])
            }
            ExprKind::Unary(_, inner)
            | ExprKind::Delete(inner)
            | ExprKind::Payable(inner)
            | ExprKind::Member(inner, _) => self.expr_may_reach_external_call(inner, seen),
            ExprKind::Index(base, index) => Self::any_may_reach_external_call([
                self.expr_may_reach_external_call(base, seen),
                index
                    .map(|index| self.expr_may_reach_external_call(index, seen))
                    .unwrap_or((false, false)),
            ]),
            ExprKind::Slice(base, start, end) => Self::any_may_reach_external_call([
                self.expr_may_reach_external_call(base, seen),
                start
                    .map(|start| self.expr_may_reach_external_call(start, seen))
                    .unwrap_or((false, false)),
                end.map(|end| self.expr_may_reach_external_call(end, seen))
                    .unwrap_or((false, false)),
            ]),
            ExprKind::Ternary(cond, then_expr, else_expr) => Self::any_may_reach_external_call([
                self.expr_may_reach_external_call(cond, seen),
                self.expr_may_reach_external_call(then_expr, seen),
                self.expr_may_reach_external_call(else_expr, seen),
            ]),
            ExprKind::Array(exprs) => self.exprs_may_reach_external_call(exprs, seen),
            ExprKind::Tuple(exprs) => {
                let mut cut_recursive_edge = false;
                for expr in exprs.iter().copied().flatten() {
                    let (expr_may_reach, expr_cut_recursive_edge) =
                        self.expr_may_reach_external_call(expr, seen);
                    cut_recursive_edge |= expr_cut_recursive_edge;
                    if expr_may_reach {
                        return (true, cut_recursive_edge);
                    }
                }
                (false, cut_recursive_edge)
            }
            ExprKind::Ident(_)
            | ExprKind::Lit(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::YulMember(..)
            | ExprKind::Err(_) => (false, false),
        }
    }

    fn exprs_may_reach_external_call(
        &mut self,
        exprs: &'hir [Expr<'hir>],
        seen: &mut HashSet<FunctionId>,
    ) -> (bool, bool) {
        let mut cut_recursive_edge = false;
        for expr in exprs {
            let (expr_may_reach, expr_cut_recursive_edge) =
                self.expr_may_reach_external_call(expr, seen);
            cut_recursive_edge |= expr_cut_recursive_edge;
            if expr_may_reach {
                return (true, cut_recursive_edge);
            }
        }
        (false, cut_recursive_edge)
    }

    fn any_may_reach_external_call(
        results: impl IntoIterator<Item = (bool, bool)>,
    ) -> (bool, bool) {
        let mut cut_recursive_edge = false;
        for (may_reach, result_cut_recursive_edge) in results {
            cut_recursive_edge |= result_cut_recursive_edge;
            if may_reach {
                return (true, cut_recursive_edge);
            }
        }
        (false, cut_recursive_edge)
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
