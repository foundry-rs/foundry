use super::ControlledDelegatecall;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            arg_for_param, branch_always_exits, count_placeholders, do_while_user_stmts,
            expr_is_address, function_ids, has_side_effect, is_address_like_cast,
            is_loop_termination_if, is_require_or_assert, loop_update, stmts_before_placeholder,
            stmts_break_or_continue, tuple_elems, unique, var_is_address_like,
        },
    },
};
use solar::{
    ast::{BinOpKind, LitKind, UnOpKind},
    interface::{Span, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, ElementaryType, Expr, ExprKind, FunctionKind, ItemId, LoopSource, Res, Stmt,
            StmtKind, TypeKind, VariableId, Visit,
        },
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    CONTROLLED_DELEGATECALL,
    Severity::High,
    "controlled-delegatecall",
    "delegatecall target is not provably trusted"
);

/// How many levels of no-argument helper functions are inlined when checking a target.
const HELPER_DEPTH: u8 = 3;

impl<'gcx> LateLintPass<'gcx> for ControlledDelegatecall {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        let Some(body) = func.body else { return };
        let mut analyzer = Analyzer::new(gcx);
        for modifier in func.modifiers {
            analyzer.safe_vars.extend(modifier_safe_vars(gcx, modifier));
        }
        let _ = analyzer.visit_stmts(body.stmts);
        for span in analyzer.hits {
            ctx.emit(&CONTROLLED_DELEGATECALL, span);
        }
    }
}

/// Flow-sensitive walk tracking which local address variables provably hold a trusted target.
///
/// `visit_stmt` breaks when control cannot fall through to the next statement.
struct Analyzer<'gcx> {
    gcx: Gcx<'gcx>,
    safe_vars: HashSet<VariableId>,
    /// Every variable written during the walk.
    assigned: HashSet<VariableId>,
    /// Per enclosing loop, the states at each `break`/`continue`.
    loop_exits: Vec<Vec<HashSet<VariableId>>>,
    hits: Vec<Span>,
}

fn intersect(a: &HashSet<VariableId>, b: &HashSet<VariableId>) -> HashSet<VariableId> {
    a.intersection(b).copied().collect()
}

impl<'gcx> Analyzer<'gcx> {
    fn new(gcx: Gcx<'gcx>) -> Self {
        Self {
            gcx,
            safe_vars: HashSet::new(),
            assigned: HashSet::new(),
            loop_exits: Vec::new(),
            hits: Vec::new(),
        }
    }

    fn visit_stmts(&mut self, stmts: &'gcx [Stmt<'gcx>]) -> ControlFlow<()> {
        stmts.iter().try_for_each(|stmt| self.visit_stmt(stmt))
    }

    fn is_trusted_target(&self, expr: &'gcx Expr<'gcx>) -> bool {
        self.is_trusted_target_inner(expr, HELPER_DEPTH)
    }

    fn is_trusted_target_inner(&self, expr: &'gcx Expr<'gcx>, depth: u8) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Address(_) => true,
                LitKind::Number(n) => n.is_zero(),
                _ => false,
            },
            ExprKind::Ident(reses) => reses.iter().any(|res| match res {
                Res::Builtin(builtin) => builtin.name() == sym::this,
                Res::Item(ItemId::Variable(vid)) => {
                    let var = self.gcx.hir.variable(*vid);
                    (var.is_constant() && var_is_address_like(var)) || self.safe_vars.contains(vid)
                }
                _ => false,
            }),
            ExprKind::Call(callee, args, _) if is_cast(callee) => {
                args.exprs().next().is_some_and(|arg| self.is_trusted_target_inner(arg, depth))
            }
            ExprKind::Payable(inner) => self.is_trusted_target_inner(inner, depth),
            ExprKind::Ternary(_, if_true, if_false) => {
                self.is_trusted_target_inner(if_true, depth)
                    && self.is_trusted_target_inner(if_false, depth)
            }
            ExprKind::Assign(_, _, rhs) => self.is_trusted_target_inner(rhs, depth),
            ExprKind::Call(callee, args, _) => {
                depth > 0
                    && args.exprs().next().is_none()
                    && no_arg_helper_return(self.gcx, callee)
                        .is_some_and(|ret| self.is_trusted_target_inner(ret, depth - 1))
            }
            _ => false,
        }
    }

    /// Local or constant address-like variable: the only kind a comparison can vouch for.
    fn is_trusted_fact_target(&self, var: VariableId) -> bool {
        let variable = self.gcx.hir.variable(var);
        (!variable.kind.is_state() || variable.is_constant()) && var_is_address_like(variable)
    }

    /// Records a write to `var`; it is safe afterwards only if local, address-like and `trusted`.
    fn assign(&mut self, var: VariableId, trusted: bool) {
        self.assigned.insert(var);
        self.safe_vars.remove(&var);
        let variable = self.gcx.hir.variable(var);
        if trusted && !variable.kind.is_state() && var_is_address_like(variable) {
            self.safe_vars.insert(var);
        }
    }

    fn assign_expr(&mut self, lhs: &'gcx Expr<'gcx>, rhs: Option<&'gcx Expr<'gcx>>) {
        if let Some(var) = underlying_var(lhs) {
            self.assign(var, rhs.is_some_and(|rhs| self.is_trusted_target(rhs)));
        }
    }

    fn handle_assign(
        &mut self,
        lhs: &'gcx Expr<'gcx>,
        op: Option<hir::BinOp>,
        rhs: &'gcx Expr<'gcx>,
    ) {
        let rhs = op.is_none().then_some(rhs);
        let Some(lhs_elems) = tuple_elems(lhs) else { return self.assign_expr(lhs, rhs) };
        let rhs_elems = rhs.and_then(tuple_elems);
        for (i, lhs_elem) in lhs_elems.iter().enumerate() {
            if let Some(lhs_elem) = lhs_elem {
                let rhs_elem = rhs_elems.and_then(|elems| elems.get(i).copied().flatten());
                self.assign_expr(lhs_elem, rhs_elem);
            }
        }
    }

    fn is_controlled_delegatecall(&self, expr: &'gcx Expr<'gcx>) -> bool {
        let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return false };
        let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return false };
        member.name == kw::Delegatecall
            && expr_is_address(self.gcx, receiver)
            && !self.is_trusted_target(receiver)
    }

    /// Learns which variables are trusted when `pred` evaluates to `!negate`.
    fn add_facts(&mut self, pred: &'gcx Expr<'gcx>, negate: bool) {
        if !has_side_effect(pred) {
            self.add_facts_unchecked(pred, negate);
        }
    }

    fn add_facts_unchecked(&mut self, pred: &'gcx Expr<'gcx>, negate: bool) {
        match &pred.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) => {
                let (eq, and_op, or_op) = if negate {
                    (BinOpKind::Ne, BinOpKind::Or, BinOpKind::And)
                } else {
                    (BinOpKind::Eq, BinOpKind::And, BinOpKind::Or)
                };
                if op.kind == and_op {
                    self.add_facts_unchecked(lhs, negate);
                    self.add_facts_unchecked(rhs, negate);
                } else if op.kind == or_op {
                    // Only facts established by both disjuncts hold.
                    let baseline = self.safe_vars.clone();
                    self.add_facts_unchecked(lhs, negate);
                    let lhs_added = std::mem::replace(&mut self.safe_vars, baseline.clone());
                    self.add_facts_unchecked(rhs, negate);
                    let rhs_added = std::mem::replace(&mut self.safe_vars, baseline);
                    self.safe_vars.extend(intersect(&lhs_added, &rhs_added));
                } else if op.kind == eq {
                    for (safe, candidate) in [(lhs, rhs), (rhs, lhs)] {
                        if self.is_trusted_target(safe)
                            && let Some(var) = underlying_var(candidate)
                            && self.is_trusted_fact_target(var)
                        {
                            self.safe_vars.insert(var);
                        }
                    }
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.add_facts_unchecked(inner, !negate);
            }
            _ => {}
        }
    }

    /// Visits `arm` under the assumption that `cond == !negate`, returning the resulting state when
    /// the arm falls through.
    fn visit_arm(
        &mut self,
        cond: &'gcx Expr<'gcx>,
        negate: bool,
        arm: impl FnOnce(&mut Self) -> ControlFlow<()>,
    ) -> Option<HashSet<VariableId>> {
        self.add_facts(cond, negate);
        arm(self).is_continue().then(|| self.safe_vars.clone())
    }

    /// Joins the states of two arms; `None` marks an arm that does not fall through.
    fn join(
        &mut self,
        a: Option<HashSet<VariableId>>,
        b: Option<HashSet<VariableId>>,
    ) -> ControlFlow<()> {
        match (a, b) {
            (Some(a), Some(b)) => self.safe_vars = intersect(&a, &b),
            (Some(state), None) | (None, Some(state)) => self.safe_vars = state,
            (None, None) => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }
}

impl<'gcx> Visit<'gcx> for Analyzer<'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<()> {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.visit_stmts(block.stmts)
            }
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);
                let baseline = self.safe_vars.clone();
                let then_state = self.visit_arm(cond, false, |this| this.visit_stmt(then));
                self.safe_vars = baseline;
                let else_state = self.visit_arm(cond, true, |this| match else_ {
                    Some(else_) => this.visit_stmt(else_),
                    None => ControlFlow::Continue(()),
                });
                self.join(then_state, else_state)
            }
            StmtKind::Loop(block, LoopSource::DoWhile)
                if !stmts_break_or_continue(do_while_user_stmts(block.stmts)) =>
            {
                // Without `break`/`continue` the body runs straight through once, then the
                // lowered `if (!cond) break;` condition is evaluated.
                self.visit_stmts(do_while_user_stmts(block.stmts))?;
                if let Some(last) = block.stmts.last()
                    && is_loop_termination_if(last)
                    && let StmtKind::If(cond, ..) = &last.kind
                {
                    let _ = self.visit_expr(cond);
                }
                ControlFlow::Continue(())
            }
            StmtKind::Loop(block, source) => {
                // The state after the loop is what holds on every way out: never entering the
                // body, each `break`/`continue`, and falling off the end of an iteration.
                self.loop_exits.push(vec![self.safe_vars.clone()]);
                let falls_through = self.visit_stmts(block.stmts).is_continue()
                    && loop_update(*source)
                        .is_none_or(|update| self.visit_stmt(update).is_continue());
                let mut exits = self.loop_exits.pop().expect("loop frame");
                if falls_through {
                    exits.push(self.safe_vars.clone());
                }
                self.safe_vars =
                    exits.iter().skip(1).fold(exits[0].clone(), |a, b| intersect(&a, b));
                ControlFlow::Continue(())
            }
            StmtKind::Break | StmtKind::Continue => {
                if let Some(exits) = self.loop_exits.last_mut() {
                    exits.push(self.safe_vars.clone());
                }
                ControlFlow::Break(())
            }
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let outer = self.safe_vars.clone();
                let mut joined = None;
                for clause in stmt_try.clauses {
                    self.safe_vars = outer.clone();
                    if self.visit_stmts(clause.block.stmts).is_continue() {
                        joined = Some(match joined {
                            Some(state) => intersect(&state, &self.safe_vars),
                            None => self.safe_vars.clone(),
                        });
                    }
                }
                self.safe_vars = joined.unwrap_or(outer);
                ControlFlow::Continue(())
            }
            StmtKind::Err(_) => {
                self.safe_vars.clear();
                ControlFlow::Continue(())
            }
            StmtKind::DeclSingle(var) => {
                let init = self.gcx.hir.variable(*var).initializer;
                self.assign(*var, init.is_some_and(|init| self.is_trusted_target(init)));
                self.walk_stmt(stmt)
            }
            StmtKind::DeclMulti(vars, init) => {
                let inits = tuple_elems(init);
                for (i, var) in vars.iter().enumerate() {
                    if let Some(var) = var {
                        let init = inits.and_then(|elems| elems.get(i).copied().flatten());
                        self.assign(*var, init.is_some_and(|init| self.is_trusted_target(init)));
                    }
                }
                self.walk_stmt(stmt)
            }
            _ => {
                let _ = self.walk_stmt(stmt);
                if branch_always_exits(stmt) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }
        }
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        if self.is_controlled_delegatecall(expr) {
            self.hits.push(expr.span);
        }
        match &expr.kind {
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let _ = self.visit_expr(lhs);
                let skipped_rhs = self.safe_vars.clone();
                let ran_rhs =
                    self.visit_arm(lhs, op.kind == BinOpKind::Or, |this| this.visit_expr(rhs));
                self.join(Some(skipped_rhs), ran_rhs)
            }
            ExprKind::Ternary(cond, if_true, if_false) => {
                let _ = self.visit_expr(cond);
                let baseline = self.safe_vars.clone();
                let true_state = self.visit_arm(cond, false, |this| this.visit_expr(if_true));
                self.safe_vars = baseline;
                let false_state = self.visit_arm(cond, true, |this| this.visit_expr(if_false));
                self.join(true_state, false_state)
            }
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let _ = self.walk_expr(expr);
                let mut args = args.exprs();
                if let Some(cond) = args.next()
                    && !args.any(has_side_effect)
                {
                    self.add_facts(cond, false);
                }
                ControlFlow::Continue(())
            }
            ExprKind::Assign(lhs, op, rhs) => {
                let _ = self.walk_expr(expr);
                self.handle_assign(lhs, *op, rhs);
                ControlFlow::Continue(())
            }
            ExprKind::Delete(target) => {
                // `delete` zeroes the target, and the zero address is trusted.
                if let Some(var) = underlying_var(target) {
                    self.assign(var, true);
                }
                self.walk_expr(expr)
            }
            _ => self.walk_expr(expr),
        }
    }
}

/// The variable a bare identifier refers to, looking through parens, `payable(...)` and
/// address-like or numeric casts.
fn underlying_var(expr: &Expr<'_>) -> Option<VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(Res::as_variable),
        ExprKind::Call(callee, args, _) if is_cast(callee) => {
            args.exprs().next().and_then(underlying_var)
        }
        ExprKind::Payable(inner) => underlying_var(inner),
        _ => None,
    }
}

/// `address(..)`, `IFoo(..)`, `uintN(..)`, `intN(..)` or `bytes(..)` cast head.
fn is_cast(callee: &Expr<'_>) -> bool {
    is_address_like_cast(callee)
        || matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(hir::Type {
                kind: TypeKind::Elementary(
                    ElementaryType::Int(_) | ElementaryType::UInt(_) | ElementaryType::Bytes
                ),
                ..
            })
        )
}

/// The expression returned by a non-virtual, non-overriding, parameterless helper whose body is a
/// single `return <expr>;` or `<ret> = <expr>;` (optionally followed by a bare `return;`).
fn no_arg_helper_return<'gcx>(
    gcx: Gcx<'gcx>,
    callee: &'gcx Expr<'gcx>,
) -> Option<&'gcx Expr<'gcx>> {
    let fid = unique(function_ids(callee))?;
    let func = gcx.hir.function(fid);
    if func.virtual_ || func.override_ || !func.parameters.is_empty() {
        return None;
    }
    let body = func.body?;
    let stmts = match body.stmts.split_last() {
        Some((last, rest)) if matches!(last.kind, StmtKind::Return(None)) => rest,
        _ => body.stmts,
    };
    let [stmt] = stmts else { return None };
    match &stmt.kind {
        StmtKind::Return(Some(expr)) => Some(expr),
        StmtKind::Expr(expr) => match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, None, rhs)
                if func.returns.len() == 1 && underlying_var(lhs) == Some(func.returns[0]) =>
            {
                Some(rhs)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Caller variables proven trusted by the statements a modifier runs before `_`: a parameter that
/// is bound to the variable, never reassigned in the prefix, and safe when `_` is reached.
fn modifier_safe_vars<'gcx>(
    gcx: Gcx<'gcx>,
    invocation: &'gcx hir::Modifier<'gcx>,
) -> Vec<VariableId> {
    let Some(fid) = invocation.id.as_function() else { return Vec::new() };
    let modifier = gcx.hir.function(fid);
    let Some(body) = modifier.body else { return Vec::new() };
    let mut prefix = Vec::new();
    if modifier.kind != FunctionKind::Modifier
        || count_placeholders(body.stmts) != 1
        || stmts_before_placeholder(body.stmts, &mut prefix).is_none()
    {
        return Vec::new();
    }
    let bindings: Vec<_> = modifier
        .parameters
        .iter()
        .filter_map(|&param| {
            let arg = arg_for_param(&gcx.hir, modifier, param, &invocation.args)?;
            Some((param, underlying_var(arg)?))
        })
        .collect();
    if bindings.is_empty() {
        return Vec::new();
    }

    let mut analyzer = Analyzer::new(gcx);
    let _ = prefix.iter().try_for_each(|stmt| analyzer.visit_stmt(stmt));
    bindings
        .into_iter()
        .filter(|&(param, caller_var)| {
            !analyzer.assigned.contains(&param)
                && analyzer.safe_vars.contains(&param)
                && analyzer.is_trusted_fact_target(caller_var)
        })
        .map(|(_, caller_var)| caller_var)
        .collect()
}
