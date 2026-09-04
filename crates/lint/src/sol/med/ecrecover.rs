use super::Ecrecover;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_exit_call, is_require_or_assert, tuple_elems},
    },
};
use alloy_primitives::{U256, uint};
use solar::{
    ast::{BinOpKind, ElementaryType, UnOpKind},
    interface::{Span, data_structures::Never},
    sema::{
        Gcx,
        builtins::Builtin,
        eval::ConstValue,
        hir::{
            self, Expr, ExprId, ExprKind, ItemId, LoopSource, Res, StateMutability, Stmt, StmtKind,
            TypeKind, VariableId, Visit,
        },
        ty::TyKind,
    },
};
use std::{
    collections::{HashMap, HashSet},
    mem,
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
        let mut analyzer = Analyzer {
            gcx,
            hir,
            returns: func.returns,
            state: FlowState::default(),
            next_value: 0,
            hits: Vec::new(),
            deferred: HashMap::new(),
            loop_exits: Vec::new(),
            loop_next: None,
        };
        if analyzer.run_block(body.stmts) {
            analyzer.use_return_values();
        }
        for span in analyzer.hits {
            ctx.emit(&ECRECOVER, span);
        }
    }
}

/// Symbolic identity of a value: a variable's incoming value or the result of an assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ValueId {
    Initial(VariableId),
    Assigned(u32),
}

/// An `ecrecover` result held in a local that has been neither observed nor validated yet.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PendingRecovery {
    signature: Option<ValueId>,
    span: Span,
}

#[derive(Clone, Default)]
struct FlowState {
    values: HashMap<VariableId, ValueId>,
    /// Values proven to be a canonical (low) `s`.
    low_s: HashSet<ValueId>,
    pending: HashMap<ValueId, Vec<PendingRecovery>>,
}

impl FlowState {
    fn value(&self, var: VariableId) -> ValueId {
        self.values.get(&var).copied().unwrap_or(ValueId::Initial(var))
    }

    fn add_pending(&mut self, value: ValueId, recovery: PendingRecovery) {
        let recoveries = self.pending.entry(value).or_default();
        if !recoveries.contains(&recovery) {
            recoveries.push(recovery);
        }
    }
}

/// A variable written by an assignment, paired with the expression it receives.
type Pair<'hir> = (Option<VariableId>, Option<&'hir Expr<'hir>>);

/// The `s` argument of an `ecrecover` call.
type Signature<'hir> = &'hir Expr<'hir>;

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    returns: &'hir [VariableId],
    state: FlowState,
    next_value: u32,
    hits: Vec<Span>,
    /// `ecrecover` calls whose result is being stored into a local. Their recovery is captured
    /// here instead of being reported at the call site.
    deferred: HashMap<ExprId, Option<PendingRecovery>>,
    /// States reaching `break`/`continue` or the end of the innermost loop body.
    loop_exits: Vec<FlowState>,
    /// Update expression of the innermost `for` loop, which `continue` still executes.
    loop_next: Option<&'hir Expr<'hir>>,
}

impl<'hir> Analyzer<'hir> {
    const fn fresh_value(&mut self) -> ValueId {
        self.next_value += 1;
        ValueId::Assigned(self.next_value)
    }

    fn join(&mut self, left: FlowState, right: FlowState) -> FlowState {
        let mut joined = FlowState {
            low_s: left.low_s.intersection(&right.low_s).copied().collect(),
            ..FlowState::default()
        };
        let mut merged = HashMap::new();
        let vars: HashSet<_> = left.values.keys().chain(right.values.keys()).copied().collect();
        for var in vars {
            let (l, r) = (left.value(var), right.value(var));
            let value = if l == r {
                l
            } else {
                let value = *merged.entry((l, r)).or_insert_with(|| self.fresh_value());
                if left.low_s.contains(&l) && right.low_s.contains(&r) {
                    joined.low_s.insert(value);
                }
                value
            };
            joined.values.insert(var, value);
            for recovery in left.pending.get(&l).into_iter().chain(right.pending.get(&r)).flatten()
            {
                joined.add_pending(value, *recovery);
            }
        }
        joined
    }

    /// Continues from the join of `states`; returns `false` when no path continues.
    fn join_all(&mut self, states: Vec<FlowState>) -> bool {
        let Some(joined) = states.into_iter().reduce(|l, r| self.join(l, r)) else { return false };
        self.state = joined;
        true
    }

    fn emit_hit(&mut self, span: Span) {
        if !self.hits.contains(&span) {
            self.hits.push(span);
        }
    }

    fn use_value(&mut self, value: ValueId) {
        for recovery in self.state.pending.remove(&value).unwrap_or_default() {
            self.emit_hit(recovery.span);
        }
    }

    fn use_return_values(&mut self) {
        for &var in self.returns {
            self.use_value(self.state.value(var));
        }
    }

    fn use_all_pending(&mut self) {
        for recoveries in mem::take(&mut self.state.pending).into_values() {
            for recovery in recoveries {
                self.emit_hit(recovery.span);
            }
        }
    }

    /// Drops pending recoveries whose signature has since been proven canonical.
    fn validate_pending(&mut self) {
        let low_s = &self.state.low_s;
        self.state.pending.retain(|_, recoveries| {
            recoveries.retain(|r| !r.signature.is_some_and(|s| low_s.contains(&s)));
            !recoveries.is_empty()
        });
    }

    fn current_value(&self, expr: &Expr<'_>) -> Option<ValueId> {
        match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, None, _) => self.current_value(lhs),
            _ => var_of(expr).map(|var| self.state.value(var)),
        }
    }

    fn is_local(&self, var: VariableId) -> bool {
        self.hir.variable(var).is_local_variable()
    }

    /// The call expression and signature argument of a builtin `ecrecover` call.
    fn ecrecover_call(
        &self,
        expr: &'hir Expr<'hir>,
    ) -> Option<(&'hir Expr<'hir>, Signature<'hir>)> {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        let callee = self.gcx.resolved_builtin(callee.peel_parens());
        (callee == Some(Builtin::EcRecover) && args.len() == 4)
            .then_some(expr)
            .zip(args.exprs().nth(3))
    }

    fn pending_recovery(&self, expr: &'hir Expr<'hir>) -> Option<PendingRecovery> {
        let (call, signature) = self.ecrecover_call(expr)?;
        (!self.is_proven_low_s(signature))
            .then(|| PendingRecovery { signature: self.current_value(signature), span: call.span })
    }

    /// The arms of `cond ? then : otherwise` that may execute.
    fn live_arms<T>(&self, cond: &Expr<'_>, then: T, otherwise: T) -> impl Iterator<Item = T> {
        match self.const_bool(cond) {
            Some(true) => [Some(then), None],
            Some(false) => [None, Some(otherwise)],
            None => [Some(then), Some(otherwise)],
        }
        .into_iter()
        .flatten()
    }

    /// `ecrecover` calls whose result becomes the value of `expr`.
    fn result_calls(&self, expr: &'hir Expr<'hir>, out: &mut Vec<ExprId>) {
        let expr = expr.peel_parens();
        if self.ecrecover_call(expr).is_some() {
            out.push(expr.id);
        }
        match &expr.kind {
            ExprKind::Ternary(cond, then, otherwise) => {
                for arm in self.live_arms(cond, *then, *otherwise) {
                    self.result_calls(arm, out);
                }
            }
            ExprKind::Assign(_, None, rhs) => self.result_calls(rhs, out),
            _ => {}
        }
    }

    fn const_value(&self, expr: &Expr<'_>) -> Option<U256> {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Call(callee, args, _) if is_transparent_cast(callee) && args.len() == 1 => {
                self.const_value(args.exprs().next()?)
            }
            // Fold arithmetic with wrapping semantics so `unchecked` bounds evaluate.
            ExprKind::Binary(lhs, op, rhs)
                if matches!(op.kind, BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul) =>
            {
                let (lhs, rhs) = (self.const_value(lhs)?, self.const_value(rhs)?);
                Some(match op.kind {
                    BinOpKind::Add => lhs.wrapping_add(rhs),
                    BinOpKind::Sub => lhs.wrapping_sub(rhs),
                    _ => lhs.wrapping_mul(rhs),
                })
            }
            _ if self.gcx.resolved_builtin(expr) == Some(Builtin::TypeMax) => {
                let TyKind::Elementary(ElementaryType::UInt(size)) =
                    self.gcx.type_of_expr(expr.id)?.kind
                else {
                    return None;
                };
                Some(U256::MAX >> (256 - size.bits()))
            }
            _ => self.gcx.try_eval_const(expr).ok()?.as_u256(),
        }
    }

    fn const_bool(&self, expr: &Expr<'_>) -> Option<bool> {
        match self.gcx.try_eval_const_value(expr).ok()? {
            ConstValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn is_proven_low_s(&self, expr: &'hir Expr<'hir>) -> bool {
        self.const_value(expr).is_some_and(|value| value <= SECP256K1_HALF_ORDER)
            || self.current_value(expr).is_some_and(|value| self.state.low_s.contains(&value))
            || matches!(&expr.peel_parens().kind, ExprKind::Ternary(cond, then, otherwise)
                if self.live_arms(cond, *then, *otherwise).all(|arm| self.is_proven_low_s(arm)))
    }

    /// The `(value, proven low)` a variable takes when assigned `rhs`.
    fn assigned(&self, rhs: Option<&'hir Expr<'hir>>) -> (Option<ValueId>, bool) {
        (
            rhs.and_then(|rhs| self.current_value(rhs)),
            rhs.is_some_and(|rhs| self.is_proven_low_s(rhs)),
        )
    }

    fn assign(&mut self, var: VariableId, (value, low_s): (Option<ValueId>, bool)) {
        let value = value.unwrap_or_else(|| self.fresh_value());
        if low_s {
            self.state.low_s.insert(value);
        }
        self.state.values.insert(var, value);
    }

    /// Pairs every variable written by `lhs` with the expression it receives.
    fn pairs(
        &self,
        lhs: &'hir Expr<'hir>,
        rhs: Option<&'hir Expr<'hir>>,
        out: &mut Vec<Pair<'hir>>,
    ) {
        let Some(elems) = tuple_elems(lhs) else { return out.push((var_of(lhs), rhs)) };
        let rhs_elems = rhs.and_then(tuple_elems);
        for (i, lhs) in elems.iter().enumerate() {
            let rhs = rhs_elems.and_then(|elems| elems.get(i).copied().flatten());
            match lhs {
                Some(lhs) => self.pairs(lhs, rhs, out),
                None => out.push((None, rhs)),
            }
        }
    }

    /// Assigns every pair, reading all right-hand sides first so tuple swaps are exact.
    fn assign_pairs(&mut self, pairs: &[Pair<'hir>]) {
        let assigned: Vec<_> =
            pairs.iter().filter_map(|(var, rhs)| Some(((*var)?, self.assigned(*rhs)))).collect();
        for (var, assigned) in assigned {
            self.assign(var, assigned);
        }
    }

    fn assign_lhs(&mut self, lhs: &'hir Expr<'hir>, rhs: Option<&'hir Expr<'hir>>) {
        let mut pairs = Vec::new();
        self.pairs(lhs, rhs, &mut pairs);
        self.assign_pairs(&pairs);
    }

    /// Models a statement-level store of `rhs` into `pairs`. Recoveries stored into locals stay
    /// pending until the local is read or the signature validated; any other destination
    /// observes the result immediately.
    fn store(&mut self, pairs: &[Pair<'hir>], rhs: Option<&'hir Expr<'hir>>) {
        let mut calls = Vec::new();
        for &(var, rhs) in pairs {
            let local = var.filter(|var| self.is_local(*var));
            let mut result_calls = Vec::new();
            if let Some(rhs) = rhs {
                self.result_calls(rhs, &mut result_calls);
            }
            for call in result_calls {
                match local {
                    Some(_) => self.deferred.insert(call, None),
                    None => self.deferred.remove(&call),
                };
                calls.push((call, local));
            }
        }
        let local_target = matches!(pairs, [(Some(var), _)] if self.is_local(*var));
        if let Some(rhs) = rhs {
            match &rhs.peel_parens().kind {
                ExprKind::Assign(lhs, None, inner) if local_target => self.store_expr(lhs, inner),
                // Copying a variable into a local does not observe its value.
                _ if local_target && self.current_value(rhs).is_some() => {}
                _ => {
                    let _ = self.visit_expr(rhs);
                }
            }
        }
        self.assign_pairs(pairs);
        for (call, var) in calls {
            if let (Some(var), Some(Some(recovery))) = (var, self.deferred.remove(&call)) {
                let value = self.state.value(var);
                self.state.add_pending(value, recovery);
            }
        }
    }

    fn store_expr(&mut self, lhs: &'hir Expr<'hir>, rhs: &'hir Expr<'hir>) {
        let mut pairs = Vec::new();
        self.pairs(lhs, Some(rhs), &mut pairs);
        self.store(&pairs, Some(rhs));
    }

    /// Visits an lvalue without treating the written variables as reads.
    fn visit_lhs(&mut self, lhs: &'hir Expr<'hir>) {
        if let Some(elems) = tuple_elems(lhs) {
            for elem in elems.iter().flatten() {
                self.visit_lhs(elem);
            }
        } else if var_of(lhs).is_none() {
            let _ = self.visit_expr(lhs);
        }
    }

    fn invalidate_mutable_state(&mut self) {
        let initial = self.state.low_s.iter().filter_map(|value| match value {
            ValueId::Initial(var) => Some(*var),
            ValueId::Assigned(_) => None,
        });
        let vars: Vec<_> = self
            .state
            .values
            .keys()
            .copied()
            .chain(initial)
            .filter(|&var| {
                let var = self.hir.variable(var);
                var.kind.is_state() && var.mutability.is_none()
            })
            .collect();
        for var in vars {
            self.assign(var, (None, false));
        }
    }

    fn has_side_effect(&self, expr: &'hir Expr<'hir>) -> bool {
        SideEffects(self).visit_expr(expr).is_break()
    }

    fn assume(&mut self, predicate: &'hir Expr<'hir>, negate: bool) {
        self.add_facts(predicate, negate);
        self.validate_pending();
    }

    fn add_facts(&mut self, predicate: &'hir Expr<'hir>, negate: bool) {
        // Facts are derived from the current values, which a side effect may have replaced.
        if self.has_side_effect(predicate) {
            return;
        }
        match &predicate.peel_parens().kind {
            ExprKind::Ternary(cond, then, otherwise) => {
                if let Some(value) = self.const_bool(cond) {
                    self.add_facts(if value { then } else { otherwise }, negate);
                }
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
                self.add_facts(inner, !negate);
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let is_and = op.kind == BinOpKind::And;
                // A constant operand either decides the result or defers to the other operand.
                for (side, other) in [(lhs, rhs), (rhs, lhs)] {
                    if let Some(value) = self.const_bool(side) {
                        if is_and == value {
                            self.add_facts(other, negate);
                        }
                        return;
                    }
                }
                if is_and == negate {
                    // Only facts established by both operands hold.
                    let baseline = self.state.low_s.clone();
                    self.add_facts(lhs, negate);
                    let from_lhs = mem::replace(&mut self.state.low_s, baseline.clone());
                    self.add_facts(rhs, negate);
                    let from_rhs = mem::replace(&mut self.state.low_s, baseline);
                    self.state.low_s.extend(from_lhs.intersection(&from_rhs));
                } else {
                    self.add_facts(lhs, negate);
                    self.add_facts(rhs, negate);
                }
            }
            ExprKind::Binary(lhs, op, rhs) => {
                let op = if negate { negate_comparison(op.kind) } else { op.kind };
                for (candidate, bound, op) in [(lhs, rhs, op), (rhs, lhs, reverse_comparison(op))] {
                    let (Some(value), Some(bound)) =
                        (self.current_value(candidate), self.const_value(bound))
                    else {
                        continue;
                    };
                    let proves_low = match op {
                        BinOpKind::Lt => bound <= SECP256K1_HALF_ORDER + U256::from(1),
                        BinOpKind::Le | BinOpKind::Eq => bound <= SECP256K1_HALF_ORDER,
                        _ => false,
                    };
                    if proves_low {
                        self.state.low_s.insert(value);
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs `then` and `otherwise` under the respective assumptions on the already visited
    /// `cond`, then continues from the join of the arms that did not exit.
    fn branch(
        &mut self,
        cond: &'hir Expr<'hir>,
        then: impl FnOnce(&mut Self) -> bool,
        otherwise: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        if let Some(value) = self.const_bool(cond) {
            self.assume(cond, !value);
            return if value { then(self) } else { otherwise(self) };
        }
        let baseline = self.state.clone();
        self.assume(cond, false);
        let then_live = then(self);
        let after_then = mem::replace(&mut self.state, baseline);
        self.assume(cond, true);
        let else_live = otherwise(self);
        if then_live && else_live {
            let after_else = mem::take(&mut self.state);
            self.state = self.join(after_then, after_else);
        } else if then_live {
            self.state = after_then;
        }
        then_live || else_live
    }

    fn run_block(&mut self, stmts: &'hir [Stmt<'hir>]) -> bool {
        stmts.iter().all(|stmt| self.run_stmt(stmt))
    }

    /// Runs `stmt`; returns `false` when control cannot continue past it.
    fn run_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => self.run_block(block.stmts),
            StmtKind::If(cond, then, otherwise) => {
                let _ = self.visit_expr(cond);
                self.branch(
                    cond,
                    |this| this.run_stmt(then),
                    |this| otherwise.is_none_or(|otherwise| this.run_stmt(otherwise)),
                )
            }
            StmtKind::Loop(block, source) => self.run_loop(block, *source),
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let after_call = self.state.clone();
                let mut live = Vec::new();
                for clause in stmt_try.clauses {
                    self.state = after_call.clone();
                    if self.run_block(clause.block.stmts) {
                        live.push(self.state.clone());
                    }
                }
                self.join_all(live)
            }
            StmtKind::Break | StmtKind::Continue => {
                if matches!(stmt.kind, StmtKind::Continue)
                    && let Some(next) = self.loop_next
                {
                    let _ = self.visit_expr(next);
                }
                self.loop_exits.push(self.state.clone());
                false
            }
            StmtKind::DeclSingle(var) => {
                let init = self.hir.variable(*var).initializer;
                self.store(&[(Some(*var), init)], init);
                true
            }
            StmtKind::DeclMulti(vars, init) => {
                let pairs: Vec<_> = match tuple_elems(init) {
                    Some(elems) => vars.iter().zip(elems).map(|(var, rhs)| (*var, *rhs)).collect(),
                    None => vars.iter().map(|var| (*var, None)).collect(),
                };
                self.store(&pairs, Some(init));
                true
            }
            StmtKind::Expr(expr) => {
                match &expr.peel_parens().kind {
                    ExprKind::Assign(lhs, None, rhs) => self.store_expr(lhs, rhs),
                    _ => {
                        let _ = self.visit_expr(expr);
                    }
                }
                !is_exit_call(expr)
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Err(_) => {
                // Inline assembly is opaque and may observe any local before changing it. Flush
                // deferred recoveries before discarding facts so assembly cannot hide a warning.
                self.use_all_pending();
                self.state = FlowState::default();
                true
            }
            StmtKind::Return(None) => {
                self.use_return_values();
                false
            }
            StmtKind::Return(Some(expr)) | StmtKind::Revert(expr) => {
                let _ = self.visit_expr(expr);
                false
            }
            _ => {
                let _ = self.walk_stmt(stmt);
                true
            }
        }
    }

    fn run_loop(&mut self, block: &'hir hir::Block<'hir>, source: LoopSource) -> bool {
        let next = matches!(source, LoopSource::ForWithUpdate)
            .then(|| for_loop_next_expr(block))
            .flatten();
        let outer_exits = mem::take(&mut self.loop_exits);
        let outer_next = mem::replace(&mut self.loop_next, next);
        // `do { .. } while (false)` runs exactly once. Any other loop may carry the effects of
        // one iteration into the next, so run the body once silently and start over from the
        // join of the entry state with every state reaching the back edge.
        let single_iteration = matches!(source, LoopSource::DoWhile)
            && matches!(block.stmts.last().map(|stmt| &stmt.kind),
                Some(StmtKind::If(cond, ..)) if self.const_bool(cond) == Some(false));
        if !single_iteration {
            let entry = self.state.clone();
            let hits = self.hits.len();
            self.run_loop_body(block);
            self.hits.truncate(hits);
            let mut states = mem::take(&mut self.loop_exits);
            states.push(entry);
            self.join_all(states);
        }
        self.run_loop_body(block);
        let exits = mem::replace(&mut self.loop_exits, outer_exits);
        self.loop_next = outer_next;
        self.join_all(exits)
    }

    fn run_loop_body(&mut self, block: &'hir hir::Block<'hir>) {
        if self.run_block(block.stmts) {
            self.loop_exits.push(self.state.clone());
        }
    }
}

impl<'hir> Visit<'hir> for Analyzer<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Never> {
        match &expr.kind {
            ExprKind::Ident(_) => {
                if let Some(value) = self.current_value(expr) {
                    self.use_value(value);
                }
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let _ = self.visit_expr(lhs);
                let run_rhs = |this: &mut Self| {
                    let _ = this.visit_expr(rhs);
                    true
                };
                let skip_rhs = |_: &mut Self| true;
                if op.kind == BinOpKind::And {
                    self.branch(lhs, run_rhs, skip_rhs);
                } else {
                    self.branch(lhs, skip_rhs, run_rhs);
                }
            }
            ExprKind::Ternary(cond, then, otherwise) => {
                let _ = self.visit_expr(cond);
                self.branch(
                    cond,
                    |this| {
                        let _ = this.visit_expr(then);
                        true
                    },
                    |this| {
                        let _ = this.visit_expr(otherwise);
                        true
                    },
                );
            }
            ExprKind::Call(callee, args, _) if is_require_or_assert(callee) => {
                let _ = self.walk_expr(expr);
                if let Some(cond) = args.exprs().next() {
                    self.assume(cond, false);
                }
            }
            ExprKind::Assign(lhs, None, rhs) => {
                self.visit_lhs(lhs);
                let _ = self.visit_expr(rhs);
                self.assign_lhs(lhs, Some(rhs));
            }
            ExprKind::Assign(lhs, Some(_), _) => {
                let _ = self.walk_expr(expr);
                self.assign_lhs(lhs, None);
            }
            ExprKind::Delete(target) => {
                self.visit_lhs(target);
                if let Some(var) = var_of(target) {
                    self.assign(var, (None, true));
                }
            }
            ExprKind::Unary(op, target) if is_inc_dec(op.kind) => {
                let _ = self.walk_expr(expr);
                if let Some(var) = var_of(target) {
                    self.assign(var, (None, false));
                }
            }
            ExprKind::Call(callee, ..) => {
                let _ = self.walk_expr(expr);
                if call_may_mutate_state(self.gcx, self.hir, callee) {
                    self.invalidate_mutable_state();
                }
                if let Some(recovery) = self.pending_recovery(expr) {
                    match self.deferred.get_mut(&expr.id) {
                        Some(captured) => *captured = Some(recovery),
                        None => self.emit_hit(recovery.span),
                    }
                }
            }
            _ => {
                let _ = self.walk_expr(expr);
            }
        }
        ControlFlow::Continue(())
    }
}

/// Finds writes and state-mutating calls that run when an expression is evaluated.
struct SideEffects<'a, 'hir>(&'a Analyzer<'hir>);

impl<'hir> Visit<'hir> for SideEffects<'_, 'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.0.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<()> {
        match &expr.kind {
            ExprKind::Assign(..) | ExprKind::Delete(_) => ControlFlow::Break(()),
            ExprKind::Unary(op, _) if is_inc_dec(op.kind) => ControlFlow::Break(()),
            ExprKind::Call(callee, ..) if call_may_mutate_state(self.0.gcx, self.0.hir, callee) => {
                ControlFlow::Break(())
            }
            ExprKind::Ternary(cond, then, otherwise) => {
                self.visit_expr(cond)?;
                for arm in self.0.live_arms(cond, *then, *otherwise) {
                    self.visit_expr(arm)?;
                }
                ControlFlow::Continue(())
            }
            ExprKind::Binary(lhs, op, _)
                if matches!(op.kind, BinOpKind::And | BinOpKind::Or)
                    && self
                        .0
                        .const_bool(lhs)
                        .is_some_and(|value| (op.kind == BinOpKind::And) != value) =>
            {
                self.visit_expr(lhs)
            }
            _ => self.walk_expr(expr),
        }
    }
}

/// The variable an expression denotes, through parens and `uint256(..)`/`bytes32(..)` casts.
fn var_of(expr: &Expr<'_>) -> Option<VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(Res::as_variable),
        ExprKind::Call(callee, args, _) if is_transparent_cast(callee) && args.len() == 1 => {
            args.exprs().next().and_then(var_of)
        }
        _ => None,
    }
}

/// The `<next>` expression of a lowered `for (..; ..; <next>)` loop body.
fn for_loop_next_expr<'hir>(block: &'hir hir::Block<'hir>) -> Option<&'hir Expr<'hir>> {
    let [stmt] = block.stmts else { return None };
    let stmt = match &stmt.kind {
        StmtKind::If(_, then, _) => *then,
        _ => stmt,
    };
    let StmtKind::Block(inner) = &stmt.kind else { return None };
    match inner.stmts {
        [_, Stmt { kind: StmtKind::Expr(next), .. }] if inner.span == block.span => Some(next),
        _ => None,
    }
}

fn is_transparent_cast(callee: &Expr<'_>) -> bool {
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

const fn is_inc_dec(op: UnOpKind) -> bool {
    matches!(op, UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec)
}

const fn negate_comparison(op: BinOpKind) -> BinOpKind {
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

const fn reverse_comparison(op: BinOpKind) -> BinOpKind {
    match op {
        BinOpKind::Lt => BinOpKind::Gt,
        BinOpKind::Le => BinOpKind::Ge,
        BinOpKind::Gt => BinOpKind::Lt,
        BinOpKind::Ge => BinOpKind::Le,
        _ => op,
    }
}

fn call_may_mutate_state(gcx: Gcx<'_>, hir: &hir::Hir<'_>, callee: &Expr<'_>) -> bool {
    let callee = callee.peel_parens();
    if matches!(callee.kind, ExprKind::Type(_)) {
        return false;
    }
    if let Some(ty) = gcx.type_of_expr(callee.id)
        && let TyKind::Fn(function) = ty.peel_refs().kind
    {
        return function.state_mutability > StateMutability::View;
    }
    match &callee.kind {
        ExprKind::Ident(reses) => !reses.iter().all(|res| match res {
            Res::Builtin(_) => true,
            Res::Item(ItemId::Function(id)) => {
                hir.function(*id).state_mutability <= StateMutability::View
            }
            _ => false,
        }),
        _ => true,
    }
}
