use super::Ecrecover;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_exit_call, is_inc_dec, is_require_or_assert, loop_update, tuple_elems},
    },
};
use alloy_primitives::{U256, uint};
use solar::{
    ast::{BinOpKind, ElementaryType, UnOpKind},
    interface::{Span, Symbol, data_structures::Never},
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

impl<'gcx> LateLintPass<'gcx> for Ecrecover {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        let Some(body) = func.body else { return };
        let mut analyzer = Analyzer {
            gcx,
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

/// A tracked place: a whole variable, or a struct field reached from a variable (`sig.s`).
///
/// A field that is only ever read has no `values` entry and resolves to `ValueId::Initial(key)`,
/// so the key carries an epoch that [`FlowState::set`] bumps whenever the whole base variable is
/// reassigned; otherwise a guard on `sig.s` would survive `sig = other;`. Writes through memory
/// or storage references also reset the fields of other variables in the same location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ValueKey {
    Var(VariableId),
    Field(VariableId, Symbol, u32),
}

impl ValueKey {
    const fn var(self) -> VariableId {
        match self {
            Self::Var(var) | Self::Field(var, ..) => var,
        }
    }
}

/// Symbolic identity of a value: a place's incoming value or the result of an assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ValueId {
    Initial(ValueKey),
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
    values: HashMap<ValueKey, ValueId>,
    /// Values proven to be a canonical (low) `s`.
    low_s: HashSet<ValueId>,
    pending: HashMap<ValueId, Vec<PendingRecovery>>,
    /// Current field epoch per variable, see [`ValueKey`].
    field_epoch: HashMap<VariableId, u32>,
}

impl FlowState {
    fn value(&self, key: ValueKey) -> ValueId {
        self.values.get(&key).copied().unwrap_or(ValueId::Initial(key))
    }

    fn field_key(&self, var: VariableId, field: Symbol) -> ValueKey {
        ValueKey::Field(var, field, self.field_epoch.get(&var).copied().unwrap_or(0))
    }

    /// Sets the value of `key`. Setting a whole variable drops its tracked fields and starts a
    /// new field epoch.
    fn set(&mut self, key: ValueKey, value: ValueId) {
        if let ValueKey::Var(var) = key {
            self.reset_fields(var);
        }
        self.values.insert(key, value);
    }

    fn reset_fields(&mut self, var: VariableId) {
        self.values.retain(|key, _| !matches!(key, ValueKey::Field(base, ..) if *base == var));
        *self.field_epoch.entry(var).or_insert(0) += 1;
    }

    /// Variables with a tracked place or a proven initial value.
    fn tracked_vars(&self) -> HashSet<VariableId> {
        let initial = self.low_s.iter().filter_map(|value| match value {
            ValueId::Initial(key) => Some(key.var()),
            ValueId::Assigned(_) => None,
        });
        self.values.keys().map(|key| key.var()).chain(initial).collect()
    }

    fn add_pending(&mut self, value: ValueId, recovery: PendingRecovery) {
        let recoveries = self.pending.entry(value).or_default();
        if !recoveries.contains(&recovery) {
            recoveries.push(recovery);
        }
    }
}

/// A place written by an assignment, paired with the expression it receives.
type Pair<'gcx> = (Option<ValueKey>, Option<&'gcx Expr<'gcx>>);

/// The `s` argument of an `ecrecover` call.
type Signature<'gcx> = &'gcx Expr<'gcx>;

struct Analyzer<'gcx> {
    gcx: Gcx<'gcx>,
    returns: &'gcx [VariableId],
    state: FlowState,
    next_value: u32,
    hits: Vec<Span>,
    /// `ecrecover` calls whose result is being stored into a local. Their recovery is captured
    /// here instead of being reported at the call site.
    deferred: HashMap<ExprId, Option<PendingRecovery>>,
    /// States reaching `break`/`continue` or the end of the innermost loop body.
    loop_exits: Vec<FlowState>,
    /// Update statement of the innermost `for` loop, which `continue` still executes.
    loop_next: Option<&'gcx Stmt<'gcx>>,
}

impl<'gcx> Analyzer<'gcx> {
    const fn fresh_value(&mut self) -> ValueId {
        self.next_value += 1;
        ValueId::Assigned(self.next_value)
    }

    fn join(&mut self, left: FlowState, right: FlowState) -> FlowState {
        // Keep the highest epoch so a field read after the join never reuses an identity that
        // was reset in either branch.
        let mut field_epoch = left.field_epoch.clone();
        for (var, epoch) in &right.field_epoch {
            let entry = field_epoch.entry(*var).or_insert(0);
            *entry = (*entry).max(*epoch);
        }
        let mut joined = FlowState {
            low_s: left.low_s.intersection(&right.low_s).copied().collect(),
            field_epoch,
            ..FlowState::default()
        };
        let mut merged = HashMap::new();
        let keys: HashSet<_> = left.values.keys().chain(right.values.keys()).copied().collect();
        for key in keys {
            let (l, r) = (left.value(key), right.value(key));
            let value = if l == r {
                l
            } else {
                let value = *merged.entry((l, r)).or_insert_with(|| self.fresh_value());
                if left.low_s.contains(&l) && right.low_s.contains(&r) {
                    joined.low_s.insert(value);
                }
                value
            };
            joined.values.insert(key, value);
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
            self.use_value(self.state.value(ValueKey::Var(var)));
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
            _ => self.place_key(expr).map(|key| self.state.value(key)),
        }
    }

    /// The tracked place an expression denotes: a variable or a `var.field` member, through
    /// parens and `uint256(..)`/`bytes32(..)` casts.
    fn place_key(&self, expr: &Expr<'_>) -> Option<ValueKey> {
        match &expr.peel_parens().kind {
            ExprKind::Call(callee, args, _) if is_transparent_cast(callee) && args.len() == 1 => {
                self.place_key(args.exprs().next()?)
            }
            ExprKind::Member(base, field) => {
                var_of(base).map(|var| self.state.field_key(var, field.name))
            }
            _ => var_of(expr).map(ValueKey::Var),
        }
    }

    /// Memory and storage variables passed by reference to an internal function (including the
    /// receiver of a `using for` call), which the callee may write through.
    fn reference_args(
        &self,
        callee: &'gcx Expr<'gcx>,
        args: &'gcx hir::CallArgs<'gcx>,
    ) -> Vec<VariableId> {
        let callee = callee.peel_parens();
        let Some(TyKind::Fn(function)) = self.gcx.type_of_expr(callee.id).map(|ty| ty.kind) else {
            return Vec::new();
        };
        if !function.is_internal() {
            return Vec::new();
        }
        let receiver = match &callee.kind {
            ExprKind::Member(base, _)
                if self.gcx.resolved_callee(callee.id).is_some_and(|c| c.attached) =>
            {
                Some(*base)
            }
            _ => None,
        };
        receiver
            .into_iter()
            .chain(args.exprs())
            .filter_map(var_of)
            .filter(|&var| {
                matches!(
                    self.gcx.hir.variable(var).data_location,
                    Some(hir::DataLocation::Memory | hir::DataLocation::Storage)
                )
            })
            .collect()
    }

    fn is_local(&self, var: VariableId) -> bool {
        self.gcx.hir.variable(var).is_local_variable()
    }

    /// The call expression and signature argument of a builtin `ecrecover` call.
    fn ecrecover_call(
        &self,
        expr: &'gcx Expr<'gcx>,
    ) -> Option<(&'gcx Expr<'gcx>, Signature<'gcx>)> {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        let callee = self.gcx.resolved_builtin(callee.peel_parens());
        (callee == Some(Builtin::EcRecover) && args.len() == 4)
            .then_some(expr)
            .zip(args.exprs().nth(3))
    }

    fn pending_recovery(&self, expr: &'gcx Expr<'gcx>) -> Option<PendingRecovery> {
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
    fn result_calls(&self, expr: &'gcx Expr<'gcx>, out: &mut Vec<ExprId>) {
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

    fn is_proven_low_s(&self, expr: &'gcx Expr<'gcx>) -> bool {
        self.const_value(expr).is_some_and(|value| value <= SECP256K1_HALF_ORDER)
            || self.current_value(expr).is_some_and(|value| self.state.low_s.contains(&value))
            || matches!(&expr.peel_parens().kind, ExprKind::Ternary(cond, then, otherwise)
                if self.live_arms(cond, *then, *otherwise).all(|arm| self.is_proven_low_s(arm)))
    }

    /// The `(value, proven low)` a variable takes when assigned `rhs`.
    fn assigned(&self, rhs: Option<&'gcx Expr<'gcx>>) -> (Option<ValueId>, bool) {
        (
            rhs.and_then(|rhs| self.current_value(rhs)),
            rhs.is_some_and(|rhs| self.is_proven_low_s(rhs)),
        )
    }

    fn assign(&mut self, key: ValueKey, (value, low_s): (Option<ValueId>, bool)) {
        let value = value.unwrap_or_else(|| self.fresh_value());
        if low_s {
            self.state.low_s.insert(value);
        }
        if let ValueKey::Field(var, field, _) = key {
            // Writing through a memory or storage reference may write any other variable of
            // that location too.
            if let Some(location) = self.aliasable_location(var) {
                for other in self.state.tracked_vars() {
                    if other != var && self.aliasable_location(other) == Some(location) {
                        self.state.reset_fields(other);
                    }
                }
            }
            let key = self.state.field_key(var, field);
            self.state.set(key, value);
        } else {
            self.state.set(key, value);
        }
    }

    /// The data location through which `var` may alias other variables.
    fn aliasable_location(&self, var: VariableId) -> Option<hir::DataLocation> {
        let var = self.gcx.hir.variable(var);
        if var.kind.is_state() && var.mutability.is_none() {
            return Some(hir::DataLocation::Storage);
        }
        var.data_location
            .filter(|loc| matches!(loc, hir::DataLocation::Memory | hir::DataLocation::Storage))
    }

    /// Pairs every place written by `lhs` with the expression it receives.
    fn pairs(
        &self,
        lhs: &'gcx Expr<'gcx>,
        rhs: Option<&'gcx Expr<'gcx>>,
        out: &mut Vec<Pair<'gcx>>,
    ) {
        let Some(elems) = tuple_elems(lhs) else { return out.push((self.place_key(lhs), rhs)) };
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
    fn assign_pairs(&mut self, pairs: &[Pair<'gcx>]) {
        let assigned: Vec<_> =
            pairs.iter().filter_map(|(key, rhs)| Some(((*key)?, self.assigned(*rhs)))).collect();
        for (key, assigned) in assigned {
            self.assign(key, assigned);
        }
    }

    fn assign_lhs(&mut self, lhs: &'gcx Expr<'gcx>, rhs: Option<&'gcx Expr<'gcx>>) {
        let mut pairs = Vec::new();
        self.pairs(lhs, rhs, &mut pairs);
        self.assign_pairs(&pairs);
    }

    /// Models a statement-level store of `rhs` into `pairs`. Recoveries stored into locals stay
    /// pending until the local is read or the signature validated; any other destination
    /// observes the result immediately.
    fn store(&mut self, pairs: &[Pair<'gcx>], rhs: Option<&'gcx Expr<'gcx>>) {
        let mut calls = Vec::new();
        for &(key, rhs) in pairs {
            let local = key.filter(|key| self.is_local(key.var()));
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
        let local_target = matches!(pairs, [(Some(key), _)] if self.is_local(key.var()));
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
        for (call, key) in calls {
            if let (Some(key), Some(Some(recovery))) = (key, self.deferred.remove(&call)) {
                let value = self.state.value(key);
                self.state.add_pending(value, recovery);
            }
        }
    }

    fn store_expr(&mut self, lhs: &'gcx Expr<'gcx>, rhs: &'gcx Expr<'gcx>) {
        let mut pairs = Vec::new();
        self.pairs(lhs, Some(rhs), &mut pairs);
        self.store(&pairs, Some(rhs));
    }

    /// Visits an lvalue without treating the written variables as reads.
    fn visit_lhs(&mut self, lhs: &'gcx Expr<'gcx>) {
        if let Some(elems) = tuple_elems(lhs) {
            for elem in elems.iter().flatten() {
                self.visit_lhs(elem);
            }
        } else if self.place_key(lhs).is_none() {
            let _ = self.visit_expr(lhs);
        }
    }

    /// Forgets mutable state variables and storage pointers, which any state-mutating call may
    /// have written.
    fn invalidate_mutable_state(&mut self) {
        for var in self.state.tracked_vars() {
            let variable = self.gcx.hir.variable(var);
            if (variable.kind.is_state() && variable.mutability.is_none())
                || variable.data_location == Some(hir::DataLocation::Storage)
            {
                self.assign(ValueKey::Var(var), (None, false));
            }
        }
    }

    fn has_side_effect(&self, expr: &'gcx Expr<'gcx>) -> bool {
        SideEffects(self).visit_expr(expr).is_break()
    }

    fn assume(&mut self, predicate: &'gcx Expr<'gcx>, negate: bool) {
        self.add_facts(predicate, negate);
        self.validate_pending();
    }

    fn add_facts(&mut self, predicate: &'gcx Expr<'gcx>, negate: bool) {
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
        cond: &'gcx Expr<'gcx>,
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

    fn run_block(&mut self, stmts: &'gcx [Stmt<'gcx>]) -> bool {
        stmts.iter().all(|stmt| self.run_stmt(stmt))
    }

    /// Runs `stmt`; returns `false` when control cannot continue past it.
    fn run_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> bool {
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
                    self.run_stmt(next);
                }
                self.loop_exits.push(self.state.clone());
                false
            }
            StmtKind::DeclSingle(var) => {
                let init = self.gcx.hir.variable(*var).initializer;
                self.store(&[(Some(ValueKey::Var(*var)), init)], init);
                true
            }
            StmtKind::DeclMulti(vars, init) => {
                let pairs: Vec<_> = match tuple_elems(init) {
                    Some(elems) => vars
                        .iter()
                        .zip(elems)
                        .map(|(var, rhs)| (var.map(ValueKey::Var), *rhs))
                        .collect(),
                    None => vars.iter().map(|var| (var.map(ValueKey::Var), None)).collect(),
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

    fn run_loop(&mut self, block: &'gcx hir::Block<'gcx>, source: LoopSource<'gcx>) -> bool {
        let next = loop_update(source);
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

    fn run_loop_body(&mut self, block: &'gcx hir::Block<'gcx>) {
        if self.run_block(block.stmts) && self.loop_next.is_none_or(|next| self.run_stmt(next)) {
            self.loop_exits.push(self.state.clone());
        }
    }
}

impl<'gcx> Visit<'gcx> for Analyzer<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
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
                if let Some(key) = self.place_key(target) {
                    self.assign(key, (None, true));
                }
            }
            ExprKind::Unary(op, target) if is_inc_dec(op.kind) => {
                let _ = self.walk_expr(expr);
                if let Some(key) = self.place_key(target) {
                    self.assign(key, (None, false));
                }
            }
            ExprKind::Call(callee, args, _) => {
                let _ = self.walk_expr(expr);
                if call_may_mutate_state(self.gcx, callee) {
                    self.invalidate_mutable_state();
                }
                for var in self.reference_args(callee, args) {
                    self.assign(ValueKey::Var(var), (None, false));
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
struct SideEffects<'a, 'gcx>(&'a Analyzer<'gcx>);

impl<'gcx> Visit<'gcx> for SideEffects<'_, 'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.0.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        match &expr.kind {
            ExprKind::Assign(..) | ExprKind::Delete(_) => ControlFlow::Break(()),
            ExprKind::Unary(op, _) if is_inc_dec(op.kind) => ControlFlow::Break(()),
            ExprKind::Call(callee, ..) if call_may_mutate_state(self.0.gcx, callee) => {
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

fn call_may_mutate_state(gcx: Gcx<'_>, callee: &Expr<'_>) -> bool {
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
                gcx.hir.function(*id).state_mutability <= StateMutability::View
            }
            _ => false,
        }),
        _ => true,
    }
}
