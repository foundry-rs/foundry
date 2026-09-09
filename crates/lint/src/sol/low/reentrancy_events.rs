use super::{
    ReentrancyEvents,
    calls_loop::{is_state_mutating_external_call, resolved_super_function_ids},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache, for_each_child, is_exit_call,
            loop_stmts, loop_update, resolved_internal_function_ids,
        },
    },
};
use solar::{
    interface::Span,
    sema::{
        Gcx,
        hir::{
            self, BinOpKind, Block, ContractId, Expr, ExprKind, Function, FunctionId, LoopSource,
            Stmt, StmtKind,
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

impl<'gcx> LateLintPass<'gcx> for ReentrancyEvents {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        let Some(body) = func.body else { return };
        Analyzer::new(ctx, gcx, func.contract).analyze_callable(func, body, false);
    }
}

type Placeholder<'gcx> = Option<(&'gcx [hir::Modifier<'gcx>], usize, Block<'gcx>)>;

/// How control can leave a piece of code. Each exit kind records whether an external call was
/// seen on some path reaching it; `None` means no path exits that way. Aborting paths
/// (`revert`, `require(false)`, ...) are simply absent, so they cannot taint later statements.
#[derive(Clone, Copy, Debug, Default)]
struct Exits {
    fallthrough: Option<bool>,
    return_: Option<bool>,
    break_: Option<bool>,
    continue_: Option<bool>,
}

impl Exits {
    fn fallthrough(tainted: bool) -> Self {
        Self { fallthrough: Some(tainted), ..Self::default() }
    }

    fn return_(tainted: bool) -> Self {
        Self { return_: Some(tainted), ..Self::default() }
    }

    fn break_(tainted: bool) -> Self {
        Self { break_: Some(tainted), ..Self::default() }
    }

    fn continue_(tainted: bool) -> Self {
        Self { continue_: Some(tainted), ..Self::default() }
    }

    fn merge(&mut self, other: Self) {
        self.fallthrough = join(self.fallthrough, other.fallthrough);
        self.return_ = join(self.return_, other.return_);
        self.break_ = join(self.break_, other.break_);
        self.continue_ = join(self.continue_, other.continue_);
    }

    /// State of the paths that return to the caller normally.
    fn normal(self) -> Option<bool> {
        join(self.fallthrough, self.return_)
    }
}

/// Joins two path states: reachable if either is, tainted if either is.
fn join(lhs: Option<bool>, rhs: Option<bool>) -> Option<bool> {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => Some(lhs || rhs),
        _ => lhs.or(rhs),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    external_call_seen: bool,
    suppress_inline_reports: bool,
}

struct Analyzer<'ctx, 's, 'c, 'gcx> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'gcx>,
    /// Contract being analysed; `this.f()` and `super.f()` resolve against it, also inside
    /// inlined helpers (runtime `this`).
    enclosing_contract: Option<ContractId>,
    call_stack: Vec<FunctionId>,
    inline_cache: HelperAnalysisCache<InlineCallKey, Exits>,
    /// Whether a helper can ever perform an external call; used where a recursive edge is cut.
    external_call_reachability: HashMap<FunctionId, bool>,
    /// Set when a reachability walk hit a recursive edge, so a negative result is inconclusive.
    reachability_cut: bool,
    emitted: HashSet<Span>,
    /// Inside a helper entered with a clean state: the helper's own pass reports its emits.
    suppress_inline_reports: bool,
    /// Set by an inlined callee with no normal exit, so the enclosing statement aborts.
    expr_aborted: bool,
}

impl<'ctx, 's, 'c, 'gcx> Analyzer<'ctx, 's, 'c, 'gcx> {
    fn new(
        ctx: &'ctx LintContext<'s, 'c>,
        gcx: Gcx<'gcx>,
        enclosing_contract: Option<ContractId>,
    ) -> Self {
        Self {
            ctx,
            gcx,
            enclosing_contract,
            call_stack: Vec::new(),
            inline_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            external_call_reachability: HashMap::new(),
            reachability_cut: false,
            emitted: HashSet::new(),
            suppress_inline_reports: false,
            expr_aborted: false,
        }
    }

    fn analyze_callable(
        &mut self,
        func: &'gcx Function<'gcx>,
        body: Block<'gcx>,
        entry: bool,
    ) -> Exits {
        self.analyze_modifier_chain(func.modifiers, 0, body, entry)
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'gcx [hir::Modifier<'gcx>],
        index: usize,
        body: Block<'gcx>,
        mut entry: bool,
    ) -> Exits {
        let Some(modifier) = modifiers.get(index) else {
            return self.analyze_block(body, None, entry);
        };
        for arg in modifier.args.exprs() {
            self.expr_aborted = false;
            self.analyze_expr(arg, &mut entry);
            // An aborting argument means the modifier, and so the body, is never entered.
            if self.expr_aborted {
                return Exits::default();
            }
        }
        // A modifier may legitimately appear several times in the chain (`m(false) m(true)`),
        // so duplicates are not skipped; `index` strictly increases, and recursion through
        // internal calls is handled by `analyze_internal_call`.
        let Some((modifier_id, modifier_body)) =
            modifier.id.as_function().and_then(|id| Some((id, self.gcx.hir.function(id).body?)))
        else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, entry);
        };
        self.call_stack.push(modifier_id);
        let exits = self.analyze_block(modifier_body, Some((modifiers, index + 1, body)), entry);
        self.call_stack.pop();
        exits
    }

    /// Analyzes one loop iteration: the body, then the `for` update on the paths that complete it.
    fn analyze_iteration(
        &mut self,
        block: Block<'gcx>,
        source: LoopSource<'gcx>,
        placeholder: Placeholder<'gcx>,
        entry: bool,
    ) -> Exits {
        let mut exits = self.analyze_block(block, placeholder, entry);
        if let (Some(update), Some(state)) = (loop_update(source), exits.fallthrough.take()) {
            exits.merge(self.analyze_stmt(update, placeholder, state));
        }
        exits
    }

    fn analyze_block(
        &mut self,
        block: Block<'gcx>,
        placeholder: Placeholder<'gcx>,
        mut entry: bool,
    ) -> Exits {
        let mut exits = Exits::default();
        for stmt in block.stmts {
            let stmt_exits = self.analyze_stmt(stmt, placeholder, entry);
            exits.merge(Exits { fallthrough: None, ..stmt_exits });
            // Only the fallthrough state reaches the next statement; without it the rest is dead.
            let Some(next) = stmt_exits.fallthrough else { return exits };
            entry = next;
        }
        exits.fallthrough = Some(entry);
        exits
    }

    fn analyze_stmt(
        &mut self,
        stmt: &'gcx Stmt<'gcx>,
        placeholder: Placeholder<'gcx>,
        mut entry: bool,
    ) -> Exits {
        self.expr_aborted = false;
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
                    self.analyze_expr(init, &mut entry);
                }
                self.unless_aborted(Exits::fallthrough(entry))
            }
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) => {
                self.analyze_expr(expr, &mut entry);
                if is_exit_call(expr) {
                    return Exits::default();
                }
                self.unless_aborted(Exits::fallthrough(entry))
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, placeholder, entry)
            }
            StmtKind::Emit(expr) => {
                // Event arguments are evaluated before emitting, so an external call in them
                // taints this emit too, and an aborting argument makes it unreachable.
                self.analyze_expr(expr, &mut entry);
                if self.expr_aborted {
                    return Exits::default();
                }
                if entry && !self.suppress_inline_reports && self.emitted.insert(stmt.span) {
                    self.ctx.emit(&REENTRANCY_EVENTS, stmt.span);
                }
                Exits::fallthrough(entry)
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, &mut entry);
                Exits::default()
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, &mut entry);
                }
                self.unless_aborted(Exits::return_(entry))
            }
            StmtKind::Break => Exits::break_(entry),
            StmtKind::Continue => Exits::continue_(entry),
            StmtKind::Loop(block, source) => {
                // Two passes suffice: the one-bit state can only go from clean to tainted around
                // the back-edge, so a second pass from the merged entry catches emits tainted
                // only on later iterations. `emitted` dedupes the diagnostics.
                let first = self.analyze_iteration(block, source, placeholder, entry);
                let back_edge =
                    entry || first.fallthrough.unwrap_or(false) || first.continue_.unwrap_or(false);
                let body = if back_edge == entry {
                    first
                } else {
                    self.analyze_iteration(block, source, placeholder, back_edge)
                };
                // Zero iterations, the end of the body, `break` and `continue` all reach the exit.
                let post = entry
                    || body.fallthrough.unwrap_or(false)
                    || body.break_.unwrap_or(false)
                    || body.continue_.unwrap_or(false);
                Exits { fallthrough: Some(post), return_: body.return_, ..Exits::default() }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, &mut entry);
                if self.expr_aborted {
                    return Exits::default();
                }
                let mut exits = self.analyze_stmt(then_stmt, placeholder, entry);
                exits.merge(match else_stmt {
                    Some(else_stmt) => self.analyze_stmt(else_stmt, placeholder, entry),
                    None => Exits::fallthrough(entry),
                });
                exits
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, &mut entry);
                if self.expr_aborted {
                    return Exits::default();
                }
                let mut exits = Exits::default();
                for clause in try_stmt.clauses {
                    exits.merge(self.analyze_block(clause.block, placeholder, entry));
                }
                exits
            }
            StmtKind::Placeholder => match placeholder {
                Some((modifiers, index, body)) => {
                    self.analyze_modifier_chain(modifiers, index, body, entry)
                }
                None => Exits::fallthrough(entry),
            },
            // Inline assembly can call out and log; conservatively taint.
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                Exits::fallthrough(true)
            }
        }
    }

    fn unless_aborted(&self, exits: Exits) -> Exits {
        if self.expr_aborted { Exits::default() } else { exits }
    }

    fn analyze_expr(&mut self, expr: &'gcx Expr<'gcx>, tainted: &mut bool) {
        match &expr.kind {
            ExprKind::Call(callee, args, _) => {
                for_each_child(expr, &mut |child| self.analyze_expr(child, tainted));
                if is_state_mutating_external_call(self.gcx, callee) {
                    *tainted = true;
                }
                // Follow internal helpers and `super` dispatch so their external calls taint the
                // caller too.
                for func_id in self.callees(callee, args.len()) {
                    self.analyze_internal_call(func_id, tainted);
                }
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                // The RHS is conditional: model it on a fork so its taint joins the result next
                // to the short-circuit path, and an aborting RHS only drops the non-short-circuit
                // path.
                self.analyze_expr(lhs, tainted);
                let lhs_aborted = std::mem::replace(&mut self.expr_aborted, false);
                let mut rhs_tainted = *tainted;
                self.analyze_expr(rhs, &mut rhs_tainted);
                let rhs_aborted = self.expr_aborted;
                self.expr_aborted = lhs_aborted;
                if !lhs_aborted && !rhs_aborted {
                    *tainted |= rhs_tainted;
                }
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.analyze_expr(cond, tainted);
                let cond_aborted = std::mem::replace(&mut self.expr_aborted, false);
                let mut then_tainted = *tainted;
                self.analyze_expr(then_expr, &mut then_tainted);
                let then_aborted = std::mem::replace(&mut self.expr_aborted, false);
                let mut else_tainted = *tainted;
                self.analyze_expr(else_expr, &mut else_tainted);
                let else_aborted = self.expr_aborted;
                // The ternary aborts iff the condition does or both branches do; aborting
                // branches drop their state.
                self.expr_aborted = cond_aborted || (then_aborted && else_aborted);
                if !(then_aborted && else_aborted) {
                    *tainted = (!then_aborted && then_tainted) || (!else_aborted && else_tainted);
                }
            }
            _ => for_each_child(expr, &mut |child| self.analyze_expr(child, tainted)),
        }
    }

    /// Internal functions and `super` targets a call through `callee` dispatches to.
    fn callees(&self, callee: &'gcx Expr<'gcx>, arg_count: usize) -> Vec<FunctionId> {
        resolved_internal_function_ids(&self.gcx.hir, callee)
            .chain(resolved_super_function_ids(
                self.gcx,
                self.enclosing_contract,
                callee,
                arg_count,
            ))
            .collect()
    }

    fn analyze_internal_call(&mut self, func_id: FunctionId, tainted: &mut bool) {
        if self.call_stack.contains(&func_id) {
            // Replace the cut recursive edge with the conservative "can this helper ever call
            // out?" summary so inline summaries stay stack-insensitive.
            *tainted |= self.helper_may_reach_external_call(func_id, &mut HashSet::new());
            return;
        }
        let func = self.gcx.hir.function(func_id);
        let Some(body) = func.body else { return };

        // Diagnostics inside a helper entered clean are left to the helper's own pass, which
        // avoids duplicate reports across callers.
        let suppress = self.suppress_inline_reports || !*tainted;
        let key = InlineCallKey {
            func_id,
            external_call_seen: *tainted,
            suppress_inline_reports: suppress,
        };
        if self.inline_cache.is_in_progress(&key) {
            return;
        }
        let summary = match self.inline_cache.get(&key) {
            Some(summary) => *summary,
            None => {
                let prev_suppress = std::mem::replace(&mut self.suppress_inline_reports, suppress);
                self.inline_cache.start(key);
                self.call_stack.push(func_id);
                let summary = self.analyze_callable(func, body, *tainted);
                self.call_stack.pop();
                self.inline_cache.finish(key, summary);
                self.suppress_inline_reports = prev_suppress;
                summary
            }
        };
        // The caller continues in the state of the normally returning paths; a callee without
        // any aborts the enclosing statement.
        match summary.normal() {
            Some(after) => *tainted = after,
            None => self.expr_aborted = true,
        }
    }

    /// Conservative summary of whether `func_id` can ever perform an external call.
    fn helper_may_reach_external_call(
        &mut self,
        func_id: FunctionId,
        seen: &mut HashSet<FunctionId>,
    ) -> bool {
        if let Some(&cached) = self.external_call_reachability.get(&func_id) {
            return cached;
        }
        if !seen.insert(func_id) {
            self.reachability_cut = true;
            return false;
        }
        let outer_cut = std::mem::replace(&mut self.reachability_cut, false);
        let func = self.gcx.hir.function(func_id);
        let may_reach = func.modifiers.iter().any(|modifier| {
            modifier.args.exprs().any(|arg| self.expr_may_reach_external_call(arg, seen))
                || modifier
                    .id
                    .as_function()
                    .is_some_and(|id| self.helper_may_reach_external_call(id, seen))
        }) || func.body.is_some_and(|body| {
            body.stmts.iter().any(|stmt| self.stmt_may_reach_external_call(stmt, seen))
        });
        seen.remove(&func_id);
        // A negative answer that relied on a cut recursive edge is not conclusive.
        if may_reach || !self.reachability_cut {
            self.external_call_reachability.insert(func_id, may_reach);
        }
        self.reachability_cut |= outer_cut;
        may_reach
    }

    fn stmt_may_reach_external_call(
        &mut self,
        stmt: &'gcx Stmt<'gcx>,
        seen: &mut HashSet<FunctionId>,
    ) -> bool {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => self
                .gcx
                .hir
                .variable(var_id)
                .initializer
                .is_some_and(|init| self.expr_may_reach_external_call(init, seen)),
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Expr(expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr) => self.expr_may_reach_external_call(expr, seen),
            StmtKind::Return(expr) => {
                expr.is_some_and(|expr| self.expr_may_reach_external_call(expr, seen))
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                block.stmts.iter().any(|stmt| self.stmt_may_reach_external_call(stmt, seen))
            }
            StmtKind::Loop(block, source) => {
                loop_stmts(block, source).any(|stmt| self.stmt_may_reach_external_call(stmt, seen))
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.expr_may_reach_external_call(cond, seen)
                    || self.stmt_may_reach_external_call(then_stmt, seen)
                    || else_stmt.is_some_and(|stmt| self.stmt_may_reach_external_call(stmt, seen))
            }
            StmtKind::Try(try_stmt) => {
                self.expr_may_reach_external_call(&try_stmt.expr, seen)
                    || try_stmt.clauses.iter().any(|clause| {
                        clause
                            .block
                            .stmts
                            .iter()
                            .any(|stmt| self.stmt_may_reach_external_call(stmt, seen))
                    })
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) => true,
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {
                false
            }
        }
    }

    fn expr_may_reach_external_call(
        &mut self,
        expr: &'gcx Expr<'gcx>,
        seen: &mut HashSet<FunctionId>,
    ) -> bool {
        let mut reached = false;
        for_each_child(expr, &mut |child| {
            reached = reached || self.expr_may_reach_external_call(child, seen);
        });
        if reached {
            return true;
        }
        let ExprKind::Call(callee, args, _) = &expr.kind else { return false };
        is_state_mutating_external_call(self.gcx, callee)
            || self
                .callees(callee, args.len())
                .into_iter()
                .any(|id| self.helper_may_reach_external_call(id, seen))
    }
}
