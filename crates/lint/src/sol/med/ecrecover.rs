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
        eval::{ConstValue, ConstantEvaluator},
        hir::{
            self, ExprKind, ItemId, LoopSource, Res, StateMutability, StmtKind, TypeKind, Visit,
        },
        ty::TyKind,
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
        let mut analyzer = Analyzer::new(gcx, hir, func.returns);
        let mut falls_through = true;
        for stmt in body.stmts {
            let _ = analyzer.visit_stmt(stmt);
            if analyzer.stmt_always_exits(stmt) {
                falls_through = false;
                break;
            }
        }
        if falls_through {
            analyzer.use_return_values();
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

#[derive(Clone, Copy, Default)]
struct AssignedValue {
    value: Option<ValueId>,
    low_s: bool,
}

#[derive(Clone, Copy)]
struct PendingRecovery {
    signature: Option<ValueId>,
    span: Span,
}

#[derive(Clone, Default)]
struct FlowState {
    values: HashMap<hir::VariableId, ValueId>,
    low_s: HashSet<ValueId>,
    pending: HashMap<ValueId, Vec<PendingRecovery>>,
}

#[derive(Clone, Default)]
struct LoopEffects {
    values: HashSet<hir::VariableId>,
    mutable_state: bool,
    all_values: bool,
}

impl LoopEffects {
    fn merge(&mut self, other: Self) {
        self.values.extend(other.values);
        self.mutable_state |= other.mutable_state;
        self.all_values |= other.all_values;
    }
}

impl FlowState {
    fn value(&self, var: hir::VariableId) -> ValueId {
        self.values.get(&var).copied().unwrap_or(ValueId::Initial(var))
    }
}

struct Analyzer<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    returns: &'hir [hir::VariableId],
    state: FlowState,
    next_value: u32,
    hits: Vec<Span>,
    ignored_reads: HashSet<ValueId>,
    deferred_calls: HashSet<hir::ExprId>,
    stored_results: HashSet<hir::ExprId>,
}

impl<'hir> Analyzer<'hir> {
    fn new(gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>, returns: &'hir [hir::VariableId]) -> Self {
        Self {
            gcx,
            hir,
            returns,
            state: FlowState::default(),
            next_value: 0,
            hits: Vec::new(),
            ignored_reads: HashSet::new(),
            deferred_calls: HashSet::new(),
            stored_results: HashSet::new(),
        }
    }

    const fn fresh_value(&mut self) -> ValueId {
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

        for var in &vars {
            let var = *var;
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
        for var in vars {
            let value = joined.value(var);
            for recovery in left
                .pending
                .get(&left.value(var))
                .into_iter()
                .flatten()
                .chain(right.pending.get(&right.value(var)).into_iter().flatten())
                .copied()
            {
                let recoveries = joined.pending.entry(value).or_default();
                if !recoveries.iter().any(|existing| {
                    existing.span == recovery.span && existing.signature == recovery.signature
                }) {
                    recoveries.push(recovery);
                }
            }
        }
        joined
    }

    fn emit_hit(&mut self, span: Span) {
        if !self.hits.contains(&span) {
            self.hits.push(span);
        }
    }

    fn record_pending(&mut self, var: hir::VariableId, recovery: PendingRecovery) {
        let value = self.state.value(var);
        let recoveries = self.state.pending.entry(value).or_default();
        if !recoveries.iter().any(|existing| {
            existing.span == recovery.span && existing.signature == recovery.signature
        }) {
            recoveries.push(recovery);
        }
    }

    fn use_value(&mut self, value: ValueId) {
        if self.ignored_reads.contains(&value) {
            return;
        }
        if let Some(recoveries) = self.state.pending.remove(&value) {
            for recovery in recoveries {
                self.emit_hit(recovery.span);
            }
        }
    }

    fn use_return_values(&mut self) {
        let values: Vec<_> = self.returns.iter().map(|var| self.state.value(*var)).collect();
        for value in values {
            self.use_value(value);
        }
    }

    fn validate_pending(&mut self) {
        let low_s = &self.state.low_s;
        self.state.pending.retain(|_, recoveries| {
            recoveries.retain(|recovery| {
                !recovery.signature.is_some_and(|signature| low_s.contains(&signature))
            });
            !recoveries.is_empty()
        });
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

    fn deferable_target(&self, expr: &'hir hir::Expr<'hir>) -> Option<hir::VariableId> {
        underlying_var(expr).filter(|var| self.hir.variable(*var).is_local_variable())
    }

    fn ecrecover_call_id(&self, expr: &'hir hir::Expr<'hir>) -> Option<hir::ExprId> {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        (is_ecrecover_builtin(self.gcx, callee) && args.len() == 4).then_some(expr.id)
    }

    fn visit_stored_expr(&mut self, expr: &'hir hir::Expr<'hir>, store_locally: bool) {
        let ignored_reads = self.ignored_reads.clone();
        let deferred_calls = self.deferred_calls.clone();
        let stored_results = self.stored_results.clone();
        if store_locally {
            // Local copies are not observable, and `ecrecover` itself is pure. Keep the
            // recovered value pending until it is validated or actually read.
            if let Some(value) = self.current_value(expr) {
                self.ignored_reads.insert(value);
            }
            if let Some(call) = self.ecrecover_call_id(expr) {
                self.deferred_calls.insert(call);
            }
            if matches!(expr.peel_parens().kind, ExprKind::Assign(..)) {
                self.stored_results.insert(expr.peel_parens().id);
            }
        }
        let _ = self.visit_expr(expr);
        self.ignored_reads = ignored_reads;
        self.deferred_calls = deferred_calls;
        self.stored_results = stored_results;
    }

    fn visit_discarded_expr(&mut self, expr: &'hir hir::Expr<'hir>) {
        let stored_results = self.stored_results.clone();
        if matches!(expr.peel_parens().kind, ExprKind::Assign(..)) {
            self.stored_results.insert(expr.peel_parens().id);
        }
        let _ = self.visit_expr(expr);
        self.stored_results = stored_results;
    }

    fn visit_assignment_lhs(&mut self, lhs: &'hir hir::Expr<'hir>) {
        let ignored_reads = self.ignored_reads.clone();
        let mut vars = HashSet::new();
        collect_lhs_vars(lhs, &mut vars);
        for var in vars {
            self.ignored_reads.insert(self.state.value(var));
        }
        let _ = self.visit_expr(lhs);
        self.ignored_reads = ignored_reads;
    }

    fn const_value(&self, expr: &'hir hir::Expr<'hir>) -> Option<U256> {
        if let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind
            && is_transparent_signature_cast(callee)
            && args.len() == 1
        {
            return args.exprs().next().and_then(|arg| self.const_value(arg));
        }
        if self.gcx.builtin_member(expr.peel_parens().id) == Some(Builtin::TypeMax)
            && let Some(ty) = self.gcx.type_of_expr(expr.peel_parens().id)
            && let TyKind::Elementary(ElementaryType::UInt(size)) = ty.kind
        {
            return Some(U256::MAX >> (256 - size.bits()));
        }
        if let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind
            && matches!(op.kind, BinOpKind::Add | BinOpKind::Sub | BinOpKind::Mul)
        {
            let lhs = self.const_value(lhs)?;
            let rhs = self.const_value(rhs)?;
            return match op.kind {
                BinOpKind::Add => Some(lhs.wrapping_add(rhs)),
                BinOpKind::Sub => Some(lhs.wrapping_sub(rhs)),
                BinOpKind::Mul => Some(lhs.wrapping_mul(rhs)),
                _ => unreachable!(),
            };
        }
        if let Some(value) =
            ConstantEvaluator::new(self.gcx).try_eval(expr).ok().and_then(|value| value.as_u256())
        {
            return Some(value);
        }
        None
    }

    fn const_bool(&self, expr: &'hir hir::Expr<'hir>) -> Option<bool> {
        match ConstantEvaluator::new(self.gcx).try_eval_value(expr).ok()? {
            ConstValue::Bool(value) => Some(value),
            _ => None,
        }
    }

    fn stmt_always_exits(&self, stmt: &'hir hir::Stmt<'hir>) -> bool {
        match &stmt.kind {
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                block.stmts.iter().any(|stmt| self.stmt_always_exits(stmt))
            }
            StmtKind::If(condition, then_stmt, else_stmt) => match self.const_bool(condition) {
                Some(true) => self.stmt_always_exits(then_stmt),
                Some(false) => else_stmt.is_some_and(|else_stmt| self.stmt_always_exits(else_stmt)),
                None => {
                    self.stmt_always_exits(then_stmt)
                        && else_stmt.is_some_and(|else_stmt| self.stmt_always_exits(else_stmt))
                }
            },
            _ => branch_always_exits(stmt),
        }
    }

    fn is_proven_low_s(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.const_value(expr).is_some_and(|value| value <= SECP256K1_HALF_ORDER)
            || self.current_value(expr).is_some_and(|value| self.state.low_s.contains(&value))
            || matches!(
                &expr.peel_parens().kind,
                ExprKind::Ternary(_, then_expr, else_expr)
                    if self.is_proven_low_s(then_expr) && self.is_proven_low_s(else_expr)
            )
    }

    fn assigned_value(&self, rhs: Option<&'hir hir::Expr<'hir>>) -> AssignedValue {
        AssignedValue {
            value: rhs.and_then(|rhs| self.current_value(rhs)),
            low_s: rhs.is_some_and(|rhs| self.is_proven_low_s(rhs)),
        }
    }

    fn assign_var_value(&mut self, var: hir::VariableId, assigned: AssignedValue) {
        let value = assigned.value.unwrap_or_else(|| self.fresh_value());
        if assigned.low_s {
            self.state.low_s.insert(value);
        }
        self.state.values.insert(var, value);
    }

    fn assign_var(&mut self, var: hir::VariableId, rhs: Option<&'hir hir::Expr<'hir>>) {
        self.assign_var_value(var, self.assigned_value(rhs));
    }

    fn assign_lhs(&mut self, lhs: &'hir hir::Expr<'hir>, rhs: Option<&'hir hir::Expr<'hir>>) {
        let mut assignments = Vec::new();
        self.collect_assignments(lhs, rhs, &mut assignments);
        for (var, assigned) in assignments {
            self.assign_var_value(var, assigned);
        }
    }

    fn collect_assignments(
        &self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: Option<&'hir hir::Expr<'hir>>,
        assignments: &mut Vec<(hir::VariableId, AssignedValue)>,
    ) {
        if let ExprKind::Tuple(lhs_elems) = &lhs.peel_parens().kind {
            let rhs_elems = match rhs.map(|rhs| &rhs.peel_parens().kind) {
                Some(ExprKind::Tuple(elems)) => Some(*elems),
                _ => None,
            };
            for (index, lhs) in lhs_elems.iter().enumerate() {
                let Some(lhs) = lhs else { continue };
                let rhs = rhs_elems.and_then(|elems| elems.get(index)).copied().flatten();
                self.collect_assignments(lhs, rhs, assignments);
            }
        } else if let Some(var) = underlying_var(lhs) {
            assignments.push((var, self.assigned_value(rhs)));
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
        self.invalidate_var(var);
    }

    fn invalidate_var(&mut self, var: hir::VariableId) {
        let value = self.fresh_value();
        self.state.values.insert(var, value);
    }

    fn tracked_vars(&self) -> HashSet<hir::VariableId> {
        self.state
            .values
            .keys()
            .copied()
            .chain(self.state.low_s.iter().filter_map(|value| match value {
                ValueId::Initial(var) => Some(*var),
                ValueId::Assigned(_) => None,
            }))
            .collect()
    }

    fn invalidate_mutable_state(&mut self) {
        let vars: Vec<_> = self
            .tracked_vars()
            .into_iter()
            .filter(|var| {
                let var = self.hir.variable(*var);
                var.kind.is_state() && !var.is_constant() && !var.is_immutable()
            })
            .collect();
        for var in vars {
            self.invalidate_var(var);
        }
    }

    fn invalidate_loop_carried(&mut self, block: &'hir hir::Block<'hir>, source: LoopSource) {
        if matches!(source, LoopSource::DoWhile)
            && block.stmts.last().is_some_and(|stmt| {
                matches!(
                    &stmt.kind,
                    StmtKind::If(condition, _, _) if self.const_bool(condition) == Some(false)
                )
            })
        {
            return;
        }

        let mut backedges = Vec::new();
        if let Some(fallthrough) =
            self.collect_loop_effects_stmts(block.stmts, LoopEffects::default(), &mut backedges)
        {
            backedges.push(fallthrough);
        }
        let Some(mut effects) = backedges.into_iter().reduce(|mut left, right| {
            left.merge(right);
            left
        }) else {
            return;
        };
        if matches!(source, LoopSource::For)
            && let Some(next) = for_loop_next_expr(block)
        {
            self.add_expr_effects(next, &mut effects);
        }

        if effects.all_values {
            effects.values.extend(self.tracked_vars());
        }
        if effects.mutable_state {
            self.invalidate_mutable_state();
        }
        for var in effects.values {
            self.invalidate_var(var);
        }
    }

    fn collect_loop_effects_stmts(
        &self,
        stmts: &'hir [hir::Stmt<'hir>],
        mut effects: LoopEffects,
        backedges: &mut Vec<LoopEffects>,
    ) -> Option<LoopEffects> {
        for stmt in stmts {
            effects = self.collect_loop_effects_stmt(stmt, effects, backedges)?;
        }
        Some(effects)
    }

    fn collect_loop_effects_stmt(
        &self,
        stmt: &'hir hir::Stmt<'hir>,
        mut effects: LoopEffects,
        backedges: &mut Vec<LoopEffects>,
    ) -> Option<LoopEffects> {
        match &stmt.kind {
            StmtKind::Break => None,
            StmtKind::Continue => {
                backedges.push(effects);
                None
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.collect_loop_effects_stmts(block.stmts, effects, backedges)
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                self.add_expr_effects(condition, &mut effects);
                match self.const_bool(condition) {
                    Some(true) => self.collect_loop_effects_stmt(then_stmt, effects, backedges),
                    Some(false) => {
                        if let Some(else_stmt) = else_stmt {
                            self.collect_loop_effects_stmt(else_stmt, effects, backedges)
                        } else {
                            Some(effects)
                        }
                    }
                    None => {
                        let after_then =
                            self.collect_loop_effects_stmt(then_stmt, effects.clone(), backedges);
                        let after_else = if let Some(else_stmt) = else_stmt {
                            self.collect_loop_effects_stmt(else_stmt, effects, backedges)
                        } else {
                            Some(effects)
                        };
                        merge_loop_effect_paths(after_then, after_else)
                    }
                }
            }
            StmtKind::Try(stmt_try) => {
                self.add_expr_effects(&stmt_try.expr, &mut effects);
                let mut fallthrough = None;
                for clause in stmt_try.clauses {
                    let clause_effects = self.collect_loop_effects_stmts(
                        clause.block.stmts,
                        effects.clone(),
                        backedges,
                    );
                    fallthrough = merge_loop_effect_paths(fallthrough, clause_effects);
                }
                fallthrough
            }
            StmtKind::Loop(..) => {
                self.add_stmt_effects(stmt, &mut effects);
                Some(effects)
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.add_expr_effects(expr, &mut effects);
                }
                None
            }
            StmtKind::Revert(expr) => {
                self.add_expr_effects(expr, &mut effects);
                None
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Err(_) => {
                effects.all_values = true;
                Some(effects)
            }
            _ => {
                self.add_stmt_effects(stmt, &mut effects);
                (!self.stmt_always_exits(stmt)).then_some(effects)
            }
        }
    }

    fn add_stmt_effects(&self, stmt: &'hir hir::Stmt<'hir>, effects: &mut LoopEffects) {
        match &stmt.kind {
            StmtKind::DeclSingle(var) => {
                if let Some(initializer) = self.hir.variable(*var).initializer {
                    self.add_expr_effects(initializer, effects);
                }
            }
            StmtKind::DeclMulti(_, initializer)
            | StmtKind::Emit(initializer)
            | StmtKind::Revert(initializer)
            | StmtKind::Expr(initializer) => self.add_expr_effects(initializer, effects),
            StmtKind::Return(Some(expr)) => self.add_expr_effects(expr, effects),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.add_stmt_effects(stmt, effects);
                }
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                effects.all_values = true;
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                self.add_expr_effects(condition, effects);
                self.add_stmt_effects(then_stmt, effects);
                if let Some(else_stmt) = else_stmt {
                    self.add_stmt_effects(else_stmt, effects);
                }
            }
            StmtKind::Try(stmt_try) => {
                self.add_expr_effects(&stmt_try.expr, effects);
                for clause in stmt_try.clauses {
                    for stmt in clause.block.stmts {
                        self.add_stmt_effects(stmt, effects);
                    }
                }
            }
            StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder => {}
        }
    }

    fn add_expr_effects(&self, expr: &'hir hir::Expr<'hir>, effects: &mut LoopEffects) {
        match &expr.peel_parens().kind {
            ExprKind::Assign(lhs, _, rhs) => {
                self.add_expr_effects(lhs, effects);
                self.add_expr_effects(rhs, effects);
                collect_lhs_vars(lhs, &mut effects.values);
            }
            ExprKind::Delete(target) => {
                self.add_expr_effects(target, effects);
                collect_lhs_vars(target, &mut effects.values);
            }
            ExprKind::Unary(op, target) => {
                self.add_expr_effects(target, effects);
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) {
                    collect_lhs_vars(target, &mut effects.values);
                }
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.add_expr_effects(expr, effects);
                }
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.add_expr_effects(lhs, effects);
                self.add_expr_effects(rhs, effects);
            }
            ExprKind::Call(callee, args, options) => {
                self.add_expr_effects(callee, effects);
                for arg in args.exprs() {
                    self.add_expr_effects(arg, effects);
                }
                if let Some(options) = options {
                    for arg in options.args {
                        self.add_expr_effects(&arg.value, effects);
                    }
                }
                effects.mutable_state |= call_may_mutate_state(self.gcx, self.hir, callee);
            }
            ExprKind::Index(base, index) => {
                self.add_expr_effects(base, effects);
                if let Some(index) = index {
                    self.add_expr_effects(index, effects);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.add_expr_effects(base, effects);
                if let Some(start) = start {
                    self.add_expr_effects(start, effects);
                }
                if let Some(end) = end {
                    self.add_expr_effects(end, effects);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::YulMember(base, _) => {
                self.add_expr_effects(base, effects);
            }
            ExprKind::Ternary(condition, then_expr, else_expr) => {
                self.add_expr_effects(condition, effects);
                self.add_expr_effects(then_expr, effects);
                self.add_expr_effects(else_expr, effects);
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().flatten() {
                    self.add_expr_effects(expr, effects);
                }
            }
            ExprKind::Lit(_)
            | ExprKind::Ident(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::Err(_) => {}
        }
    }

    fn add_facts(&mut self, predicate: &'hir hir::Expr<'hir>, negate: bool) {
        if expr_has_fact_side_effect(self.gcx, self.hir, predicate) {
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

    fn assume(&mut self, predicate: &'hir hir::Expr<'hir>, negate: bool) {
        self.add_facts(predicate, negate);
        self.validate_pending();
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

    fn pending_recovery(&self, expr: &'hir hir::Expr<'hir>) -> Option<PendingRecovery> {
        let expr = expr.peel_parens();
        let ExprKind::Call(callee, args, _) = &expr.kind else { return None };
        if !is_ecrecover_builtin(self.gcx, callee) || args.len() != 4 {
            return None;
        }
        let signature = args.exprs().nth(3)?;
        (!self.is_proven_low_s(signature))
            .then(|| PendingRecovery { signature: self.current_value(signature), span: expr.span })
    }

    fn join_all(&mut self, states: Vec<FlowState>) -> Option<FlowState> {
        states.into_iter().reduce(|left, right| self.join(left, right))
    }

    fn visit_loop_stmts(
        &mut self,
        stmts: &'hir [hir::Stmt<'hir>],
        exits: &mut Vec<FlowState>,
    ) -> Option<FlowState> {
        for stmt in stmts {
            self.visit_loop_stmt(stmt, exits)?;
        }
        Some(self.snapshot())
    }

    fn visit_loop_stmt(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        exits: &mut Vec<FlowState>,
    ) -> Option<FlowState> {
        match &stmt.kind {
            StmtKind::Break | StmtKind::Continue => {
                exits.push(self.snapshot());
                None
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.visit_loop_stmts(block.stmts, exits)
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                let constant = self.const_bool(condition);
                let _ = self.visit_expr(condition);
                if let Some(value) = constant {
                    self.assume(condition, !value);
                    return if value {
                        self.visit_loop_stmt(then_stmt, exits)
                    } else if let Some(else_stmt) = else_stmt {
                        self.visit_loop_stmt(else_stmt, exits)
                    } else {
                        Some(self.snapshot())
                    };
                }
                let baseline = self.snapshot();

                self.assume(condition, false);
                let after_then = self.visit_loop_stmt(then_stmt, exits);

                self.restore(baseline);
                self.assume(condition, true);
                let after_else = if let Some(else_stmt) = else_stmt {
                    self.visit_loop_stmt(else_stmt, exits)
                } else {
                    Some(self.snapshot())
                };

                let joined = match (after_then, after_else) {
                    (Some(then_state), Some(else_state)) => self.join(then_state, else_state),
                    (Some(state), None) | (None, Some(state)) => state,
                    (None, None) => return None,
                };
                self.restore(joined.clone());
                Some(joined)
            }
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let after_call = self.snapshot();
                let mut fallthrough = Vec::new();
                for clause in stmt_try.clauses {
                    // Argument expressions execute in the caller even when the external call
                    // reverts, so caught paths must retain their effects. Keeping the call's
                    // effects too is conservative for mutable state.
                    self.restore(after_call.clone());
                    if let Some(state) = self.visit_loop_stmts(clause.block.stmts, exits) {
                        fallthrough.push(state);
                    }
                }
                let joined = self.join_all(fallthrough)?;
                self.restore(joined.clone());
                Some(joined)
            }
            _ => {
                let _ = self.visit_stmt(stmt);
                (!self.stmt_always_exits(stmt)).then(|| self.snapshot())
            }
        }
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
                    if self.stmt_always_exits(stmt) {
                        break;
                    }
                }
                return ControlFlow::Continue(());
            }
            StmtKind::If(condition, then_stmt, else_stmt) => {
                let constant = self.const_bool(condition);
                let _ = self.visit_expr(condition);
                if let Some(value) = constant {
                    self.assume(condition, !value);
                    if value {
                        let _ = self.visit_stmt(then_stmt);
                    } else if let Some(else_stmt) = else_stmt {
                        let _ = self.visit_stmt(else_stmt);
                    }
                    return ControlFlow::Continue(());
                }
                let baseline = self.snapshot();

                self.assume(condition, false);
                let _ = self.visit_stmt(then_stmt);
                let then_exits = self.stmt_always_exits(then_stmt);
                let after_then = self.snapshot();

                self.restore(baseline);
                self.assume(condition, true);
                let else_exits = if let Some(else_stmt) = else_stmt {
                    let _ = self.visit_stmt(else_stmt);
                    self.stmt_always_exits(else_stmt)
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
                self.invalidate_loop_carried(block, *source);
                let mut exits = Vec::new();
                if let Some(fallthrough) = self.visit_loop_stmts(block.stmts, &mut exits) {
                    exits.push(fallthrough);
                }
                let joined = self.join_all(exits).unwrap_or(baseline);
                self.restore(joined);
                return ControlFlow::Continue(());
            }
            StmtKind::Try(stmt_try) => {
                let _ = self.visit_expr(&stmt_try.expr);
                let after_call = self.snapshot();
                let mut fallthrough = Vec::new();
                for clause in stmt_try.clauses {
                    self.restore(after_call.clone());
                    for stmt in clause.block.stmts {
                        let _ = self.visit_stmt(stmt);
                        if self.stmt_always_exits(stmt) {
                            break;
                        }
                    }
                    if !clause.block.stmts.iter().any(|stmt| self.stmt_always_exits(stmt)) {
                        fallthrough.push(self.snapshot());
                    }
                }
                let joined = fallthrough
                    .into_iter()
                    .reduce(|left, right| self.join(left, right))
                    .unwrap_or(after_call);
                self.restore(joined);
                return ControlFlow::Continue(());
            }
            StmtKind::DeclSingle(var) => {
                let init = self.hir.variable(*var).initializer;
                if let Some(init) = init {
                    self.visit_stored_expr(init, true);
                }
                self.assign_var(*var, init);
                if let Some(recovery) = init.and_then(|init| self.pending_recovery(init)) {
                    self.record_pending(*var, recovery);
                }
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
            StmtKind::Expr(expr) => {
                self.visit_discarded_expr(expr);
                return ControlFlow::Continue(());
            }
            StmtKind::Err(_) | StmtKind::AssemblyBlock(_) => {
                self.state = FlowState::default();
            }
            StmtKind::Return(None) => {
                self.use_return_values();
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if matches!(expr.kind, ExprKind::Ident(_))
            && let Some(value) = self.current_value(expr)
        {
            self.use_value(value);
        }
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::And | BinOpKind::Or)
        {
            let _ = self.visit_expr(lhs);
            let skipped_rhs = self.snapshot();
            self.assume(lhs, op.kind == BinOpKind::Or);
            let _ = self.visit_expr(rhs);
            let ran_rhs = self.snapshot();
            let joined = self.join(skipped_rhs, ran_rhs);
            self.restore(joined);
            return ControlFlow::Continue(());
        }
        if let ExprKind::Ternary(condition, then_expr, else_expr) = &expr.kind {
            let _ = self.visit_expr(condition);
            let baseline = self.snapshot();
            self.assume(condition, false);
            let _ = self.visit_expr(then_expr);
            let after_then = self.snapshot();
            self.restore(baseline);
            self.assume(condition, true);
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
                    self.assume(condition, false);
                }
                return result;
            }
            ExprKind::Assign(lhs, op, rhs) => {
                if op.is_none() {
                    let target = self.deferable_target(lhs);
                    let result_is_stored = self.stored_results.contains(&expr.peel_parens().id);
                    self.visit_assignment_lhs(lhs);
                    self.visit_stored_expr(rhs, target.is_some());
                    let recovery = target.and_then(|_| self.pending_recovery(rhs));
                    self.assign_lhs(lhs, Some(rhs));
                    if let Some((target, recovery)) = target.zip(recovery) {
                        self.record_pending(target, recovery);
                    }
                    if let Some(target) = target
                        && !result_is_stored
                    {
                        self.use_value(self.state.value(target));
                    }
                    return ControlFlow::Continue(());
                }
                let result = self.walk_expr(expr);
                self.assign_lhs(lhs, None);
                return result;
            }
            ExprKind::Delete(target) => {
                self.visit_assignment_lhs(target);
                self.mark_deleted(target);
                return ControlFlow::Continue(());
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
        if let ExprKind::Call(callee, _, _) = &expr.kind
            && call_may_mutate_state(self.gcx, self.hir, callee)
        {
            self.invalidate_mutable_state();
        }
        if let Some(recovery) = self.pending_recovery(expr)
            && !self.deferred_calls.contains(&expr.peel_parens().id)
        {
            self.emit_hit(recovery.span);
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

fn collect_lhs_vars(expr: &hir::Expr<'_>, vars: &mut HashSet<hir::VariableId>) {
    if let ExprKind::Tuple(exprs) = &expr.peel_parens().kind {
        for expr in exprs.iter().flatten() {
            collect_lhs_vars(expr, vars);
        }
    } else if let Some(var) = underlying_var(expr) {
        vars.insert(var);
    }
}

fn for_loop_next_expr<'hir>(block: &'hir hir::Block<'hir>) -> Option<&'hir hir::Expr<'hir>> {
    let [stmt] = block.stmts else { return None };
    let stmt = match &stmt.kind {
        StmtKind::If(_, then_stmt, _) => *then_stmt,
        _ => stmt,
    };
    let StmtKind::Block(inner) = &stmt.kind else { return None };
    if inner.span != block.span {
        return None;
    }
    let [_, next] = inner.stmts else { return None };
    let StmtKind::Expr(next) = &next.kind else { return None };
    Some(*next)
}

fn merge_loop_effect_paths(
    left: Option<LoopEffects>,
    right: Option<LoopEffects>,
) -> Option<LoopEffects> {
    match (left, right) {
        (Some(mut left), Some(right)) => {
            left.merge(right);
            Some(left)
        }
        (Some(effects), None) | (None, Some(effects)) => Some(effects),
        (None, None) => None,
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

fn is_ecrecover_builtin(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    if let Some(resolved) = gcx.resolved_callee(callee.peel_parens().id) {
        return matches!(resolved.res, Res::Builtin(Builtin::EcRecover));
    }
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| matches!(res, Res::Builtin(Builtin::EcRecover)))
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

fn call_may_mutate_state(gcx: Gcx<'_>, hir: &hir::Hir<'_>, callee: &hir::Expr<'_>) -> bool {
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
            Res::Item(ItemId::Function(function)) => {
                hir.function(*function).state_mutability <= StateMutability::View
            }
            _ => false,
        }),
        _ => true,
    }
}

fn expr_has_fact_side_effect(gcx: Gcx<'_>, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Assign(..) | ExprKind::Delete(_) => true,
        ExprKind::Unary(op, inner) => {
            matches!(
                op.kind,
                UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
            ) || expr_has_fact_side_effect(gcx, hir, inner)
        }
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_has_fact_side_effect(gcx, hir, expr))
        }
        ExprKind::Binary(lhs, _, rhs) => {
            expr_has_fact_side_effect(gcx, hir, lhs) || expr_has_fact_side_effect(gcx, hir, rhs)
        }
        ExprKind::Call(callee, args, options) => {
            call_may_mutate_state(gcx, hir, callee)
                || expr_has_fact_side_effect(gcx, hir, callee)
                || args.exprs().any(|expr| expr_has_fact_side_effect(gcx, hir, expr))
                || options.is_some_and(|options| {
                    options.args.iter().any(|arg| expr_has_fact_side_effect(gcx, hir, &arg.value))
                })
        }
        ExprKind::Index(base, index) => {
            expr_has_fact_side_effect(gcx, hir, base)
                || index.is_some_and(|expr| expr_has_fact_side_effect(gcx, hir, expr))
        }
        ExprKind::Slice(base, start, end) => {
            expr_has_fact_side_effect(gcx, hir, base)
                || start.is_some_and(|expr| expr_has_fact_side_effect(gcx, hir, expr))
                || end.is_some_and(|expr| expr_has_fact_side_effect(gcx, hir, expr))
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) | ExprKind::YulMember(base, _) => {
            expr_has_fact_side_effect(gcx, hir, base)
        }
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            expr_has_fact_side_effect(gcx, hir, condition)
                || expr_has_fact_side_effect(gcx, hir, then_expr)
                || expr_has_fact_side_effect(gcx, hir, else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_has_fact_side_effect(gcx, hir, expr))
        }
        ExprKind::Lit(_)
        | ExprKind::Ident(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}
