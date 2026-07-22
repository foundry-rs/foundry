use super::ReentrancyEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            helper_cache::{DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache},
            primitives::{branch_always_exits, is_require_or_assert},
        },
    },
};
use alloy_primitives::U256;
use solar::{
    ast::{
        BinOpKind, DataLocation, ElementaryType, LitKind, StateMutability, StrKind, TypeSize,
        UnOpKind, Visibility,
    },
    interface::{Span, kw, sym},
    sema::{
        Gcx, Ty,
        hir::{
            self, CallArgs, CallArgsKind, ExprKind, FunctionId, ItemId, Res, StmtKind, VariableId,
        },
        ty::{TyFnKind, TyKind},
    },
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const REENTRANCY_GAS_STIPEND: u64 = 2_300;

declare_forge_lint!(
    REENTRANCY_BALANCE,
    Severity::High,
    "reentrancy-balance",
    "external call can be reentered before a stale contract balance is checked"
);

declare_forge_lint!(
    REENTRANCY_ETH,
    Severity::High,
    "reentrancy-eth",
    "state read before ETH transfer is written after the transfer"
);

declare_forge_lint!(
    REENTRANCY_NO_ETH,
    Severity::Med,
    "reentrancy-no-eth",
    "state read before external call is written after the call"
);

impl<'hir> LateLintPass<'hir> for ReentrancyEth {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !is_entry_point(func) {
            return;
        }

        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, gcx, hir, func);
        if !analyzer.has_enabled_lints() {
            return;
        }
        let mut state = FlowState::default();
        analyzer.analyze_callable(func, body, &mut state);
    }
}

fn is_entry_point(func: &hir::Function<'_>) -> bool {
    if matches!(func.state_mutability, StateMutability::Pure | StateMutability::View) {
        return false;
    }
    if func.is_constructor() {
        return false;
    }
    if func.is_special() {
        return true;
    }
    func.kind.is_function() && matches!(func.visibility, Visibility::Public | Visibility::External)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct FlowState {
    state_reads: BTreeSet<VariableId>,
    pending_calls: Vec<PendingCall>,
    internal_function_targets: BTreeMap<VariableId, BTreeSet<FunctionId>>,
    self_address_local_paths: BTreeMap<VariableId, PathAlternatives>,
    balance_locals: BTreeSet<VariableId>,
    balance_local_paths: BTreeMap<VariableId, PathAlternatives>,
    balance_comparison_locals: BTreeMap<VariableId, Vec<Span>>,
    pending_balance_calls: Vec<PendingBalanceCall>,
    invalidated_balance_guards: BTreeSet<VariableId>,
    path_predicates: PathPredicates,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PendingCall {
    span: Span,
    kind: ReentrantCallKind,
    state_reads: BTreeSet<VariableId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PendingBalanceCall {
    span: Span,
    stale_locals: BTreeSet<VariableId>,
    paths: PathAlternatives,
}

type PathPredicates = BTreeMap<VariableId, bool>;
type PathAlternatives = BTreeSet<PathPredicates>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ReentrantCallKind {
    Eth,
    NoEth,
}

impl FlowState {
    fn push_read(&mut self, var_id: VariableId) {
        self.state_reads.insert(var_id);
    }

    fn push_call(&mut self, span: Span, kind: ReentrantCallKind) {
        if self.state_reads.is_empty() {
            return;
        }

        if let Some(existing) =
            self.pending_calls.iter_mut().find(|call| call.span == span && call.kind == kind)
        {
            existing.state_reads.extend(self.state_reads.iter().copied());
        } else {
            self.pending_calls.push(PendingCall {
                span,
                kind,
                state_reads: self.state_reads.clone(),
            });
        }
    }

    fn push_balance_call(&mut self, span: Span) {
        let stale_locals = self
            .balance_locals
            .iter()
            .filter(|var_id| {
                self.balance_local_paths
                    .get(var_id)
                    .is_some_and(|paths| paths_compatible_with(paths, &self.path_predicates))
            })
            .copied()
            .collect::<BTreeSet<_>>();
        if stale_locals.is_empty() {
            return;
        }
        let paths = [self.path_predicates.clone()].into_iter().collect::<PathAlternatives>();

        if let Some(existing) = self.pending_balance_calls.iter_mut().find(|call| call.span == span)
        {
            existing.stale_locals.extend(stale_locals);
            existing.paths.extend(paths);
        } else {
            self.pending_balance_calls.push(PendingBalanceCall { span, stale_locals, paths });
        }
    }
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    emitted: HashSet<Span>,
    emitted_balance: HashSet<Span>,
    call_stack: Vec<FunctionId>,
    inline_cache: HelperAnalysisCache<InlineCallKey, InlineCallResult>,
    recursive_cut_frontiers: HashMap<RecursiveFrontierKey, Vec<FunctionId>>,
    direct_internal_calls: HashMap<FunctionId, Vec<FunctionId>>,
    reentrancy_eth_enabled: bool,
    reentrancy_no_eth_enabled: bool,
    reentrancy_balance_enabled: bool,
    balance_only_analysis: bool,
    call_balance_values: HashMap<Span, Vec<BalanceValue>>,
    return_collectors: Vec<ReturnCollector>,
    active_balance_guards: Vec<VariableId>,
    balance_reentry_lock: Option<VariableId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    /// First active function that can cut recursion from this callee.
    recursive_cut: Option<FunctionId>,
    balance_only: bool,
    active_balance_guards: Vec<VariableId>,
    parameter_predicates: Vec<Option<(VariableId, bool)>>,
    state: FlowState,
}

type ModifierContinuation<'hir> =
    (&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>, Option<VariableId>);

#[derive(Clone, Debug)]
struct InlineCallResult {
    state: FlowState,
    returns: Vec<BalanceValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RecursiveFrontierKey {
    func_id: FunctionId,
    active_call_stack: Vec<FunctionId>,
}

#[derive(Clone, Debug, Default)]
struct BalanceValue {
    balance_dependent: bool,
    balance_paths: PathAlternatives,
    self_address_paths: PathAlternatives,
    stale_calls: HashSet<Span>,
    stale_comparisons: Vec<Span>,
}

#[derive(Clone, Copy, Debug)]
enum BalanceQuery {
    Current(Span),
    Stale(Span),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockValue {
    Bool(bool),
    Number(U256),
}

#[derive(Debug)]
struct ReturnCollector {
    func_id: FunctionId,
    values: Vec<BalanceValue>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(
        ctx: &'ctx LintContext<'s, 'c>,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        entry: &'hir hir::Function<'hir>,
    ) -> Self {
        let reentrancy_balance_enabled = ctx.is_lint_enabled(REENTRANCY_BALANCE.id);
        Self {
            ctx,
            gcx,
            hir,
            emitted: HashSet::new(),
            emitted_balance: HashSet::new(),
            call_stack: Vec::new(),
            inline_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            recursive_cut_frontiers: HashMap::new(),
            direct_internal_calls: HashMap::new(),
            reentrancy_eth_enabled: ctx.is_lint_enabled(REENTRANCY_ETH.id),
            reentrancy_no_eth_enabled: ctx.is_lint_enabled(REENTRANCY_NO_ETH.id),
            reentrancy_balance_enabled,
            balance_only_analysis: false,
            call_balance_values: HashMap::new(),
            return_collectors: Vec::new(),
            active_balance_guards: Vec::new(),
            balance_reentry_lock: reentrancy_balance_enabled
                .then(|| balance_reentry_lock(gcx, hir, entry))
                .flatten(),
        }
    }

    const fn has_enabled_lints(&self) -> bool {
        self.reentrancy_eth_enabled
            || self.reentrancy_no_eth_enabled
            || self.reentrancy_balance_enabled
    }

    fn analyze_callable(
        &mut self,
        func: &'hir hir::Function<'hir>,
        body: hir::Block<'hir>,
        state: &mut FlowState,
    ) -> bool {
        self.analyze_modifier_chain(func.modifiers, 0, body, state)
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'hir [hir::Modifier<'hir>],
        index: usize,
        body: hir::Block<'hir>,
        state: &mut FlowState,
    ) -> bool {
        let Some(modifier) = modifiers.get(index) else {
            return self.analyze_block(body, None, state);
        };

        for arg in modifier.args.exprs() {
            self.analyze_expr(arg, state);
        }

        let Some(modifier_id) = modifier.id.as_function() else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, state);
        };

        if self.call_stack.contains(&modifier_id) {
            return self.analyze_modifier_chain(modifiers, index + 1, body, state);
        }

        let modifier_func = self.hir.function(modifier_id);
        let Some(modifier_body) = modifier_func.body else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, state);
        };

        self.seed_balance_parameters(modifier_func, &modifier.args, state);
        self.call_stack.push(modifier_id);
        let balance_guard = self
            .reentrancy_balance_enabled
            .then(|| standard_reentrancy_guard_lock(self.hir, modifier_func))
            .flatten();
        let continuation = Some((modifiers, index + 1, body, balance_guard));
        let falls_through = self.analyze_block(modifier_body, continuation, state);
        self.call_stack.pop();
        self.clear_function_balance_locals(modifier_id, state);
        falls_through
    }

    fn analyze_block(
        &mut self,
        block: hir::Block<'hir>,
        placeholder: Option<ModifierContinuation<'hir>>,
        state: &mut FlowState,
    ) -> bool {
        for stmt in block.stmts {
            if !self.analyze_stmt(stmt, placeholder, state) {
                return false;
            }
        }
        true
    }

    fn analyze_stmt(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        placeholder: Option<ModifierContinuation<'hir>>,
        state: &mut FlowState,
    ) -> bool {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                let init = self.hir.variable(var_id).initializer;
                if let Some(init) = init {
                    self.analyze_expr(init, state);
                    self.update_internal_function_target(state, var_id, init);
                    if self.reentrancy_balance_enabled {
                        self.update_balance_local(state, var_id, Some(init), false);
                    }
                }
                if self.reentrancy_balance_enabled {
                    self.update_self_address_local(state, var_id, init);
                }
                true
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr, state);
                if self.reentrancy_balance_enabled {
                    self.update_balance_vars(state, vars.iter().copied(), expr);
                    self.update_self_address_vars(state, vars.iter().copied(), expr);
                }
                true
            }
            StmtKind::Expr(expr) => {
                self.analyze_expr(expr, state);
                true
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, placeholder, state)
            }
            StmtKind::Emit(expr) => {
                self.analyze_expr(expr, state);
                true
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, state);
                false
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.analyze_expr(expr, state);
                }
                if self.reentrancy_balance_enabled {
                    self.record_return(expr, state);
                }
                false
            }
            StmtKind::Break | StmtKind::Continue => false,
            StmtKind::Loop(block, _) => {
                let before_loop = state.clone();
                let mut body_state = state.clone();
                self.analyze_block(block, placeholder, &mut body_state);
                // One bounded second iteration exposes loop-carried balance checks while leaving
                // the established ETH and no-ETH analysis unchanged.
                let second_iteration = self.reentrancy_balance_enabled.then(|| {
                    let mut second_iteration = body_state.balance_only();
                    self.analyze_with_only_balance(|this| {
                        this.analyze_block(block, placeholder, &mut second_iteration);
                    });
                    second_iteration
                });
                state.clear();
                state.merge(&before_loop);
                state.merge(&body_state);
                let mut path_predicates = common_path_predicates(
                    &before_loop.path_predicates,
                    &body_state.path_predicates,
                );
                if let Some(second_iteration) = second_iteration {
                    path_predicates =
                        common_path_predicates(&path_predicates, &second_iteration.path_predicates);
                    state.merge_balance(&second_iteration);
                }
                state.path_predicates = path_predicates;
                true
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, state);
                if self.reentrancy_balance_enabled
                    && (branch_stops_current_path(then_stmt)
                        || else_stmt.is_some_and(branch_stops_current_path))
                {
                    self.emit_balance_calls(cond, state);
                }

                let mut then_state = state.clone();
                let mut else_state = state.clone();
                let predicate = self
                    .reentrancy_balance_enabled
                    .then(|| path_predicate(self.hir, cond))
                    .flatten();
                let then_reachable =
                    predicate.is_none_or(|predicate| then_state.constrain_path(predicate));
                let else_reachable = predicate
                    .is_none_or(|(var_id, value)| else_state.constrain_path((var_id, !value)));
                let then_falls_through =
                    then_reachable && self.analyze_stmt(then_stmt, placeholder, &mut then_state);

                let else_falls_through = if !else_reachable {
                    false
                } else if let Some(else_stmt) = else_stmt {
                    self.analyze_stmt(else_stmt, placeholder, &mut else_state)
                } else {
                    true
                };

                state.clear();
                if then_falls_through {
                    state.merge(&then_state);
                }
                if else_falls_through {
                    state.merge(&else_state);
                }
                state.path_predicates = match (then_falls_through, else_falls_through) {
                    (true, true) => common_path_predicates(
                        &then_state.path_predicates,
                        &else_state.path_predicates,
                    ),
                    (true, false) => then_state.path_predicates,
                    (false, true) => else_state.path_predicates,
                    (false, false) => PathPredicates::new(),
                };

                then_falls_through || else_falls_through
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, state);

                let mut merged = FlowState::default();
                let mut path_predicates = None;
                let mut any_falls_through = false;
                for clause in try_stmt.clauses {
                    let mut clause_state = state.clone();
                    let falls_through =
                        self.analyze_block(clause.block, placeholder, &mut clause_state);
                    if falls_through {
                        merged.merge(&clause_state);
                        path_predicates = Some(match path_predicates {
                            Some(path_predicates) => common_path_predicates(
                                &path_predicates,
                                &clause_state.path_predicates,
                            ),
                            None => clause_state.path_predicates.clone(),
                        });
                        any_falls_through = true;
                    }
                }

                *state = merged;
                state.path_predicates = path_predicates.unwrap_or_default();
                any_falls_through
            }
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body, balance_guard)) = placeholder {
                    if let Some(lock_var) = balance_guard {
                        state.invalidated_balance_guards.remove(&lock_var);
                        self.active_balance_guards.push(lock_var);
                    }
                    let falls_through = self.analyze_modifier_chain(modifiers, index, body, state);
                    if balance_guard.is_some() {
                        self.active_balance_guards.pop();
                    }
                    falls_through
                } else {
                    true
                }
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) => {
                state.invalidated_balance_guards.extend(self.active_balance_guards.iter().copied());
                state.internal_function_targets.clear();
                state.self_address_local_paths.clear();
                true
            }
            StmtKind::Err(_) => true,
        }
    }

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                if op.is_some() {
                    self.analyze_expr(lhs, state);
                }
                self.analyze_expr(rhs, state);
                self.analyze_lhs_indices(lhs, state);
                let written_vars = state_write_lhs_vars(self.hir, lhs);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
                    self.invalidate_balance_guards(state, &written_vars);
                }
                forget_path_predicates(state, local_write_lhs_vars(self.hir, lhs));
                if let Some(var_id) = lhs_local_var(self.hir, lhs) {
                    if op.is_none() {
                        self.update_internal_function_target(state, var_id, rhs);
                    } else {
                        state.internal_function_targets.remove(&var_id);
                    }
                }
                if self.reentrancy_balance_enabled {
                    self.update_balance_assignment(state, lhs, rhs, op.is_some());
                    self.update_self_address_assignment(state, lhs, rhs, op.is_some());
                }
            }
            ExprKind::Delete(inner) => {
                self.analyze_lhs_indices(inner, state);
                let written_vars = state_write_lhs_vars(self.hir, inner);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
                    self.invalidate_balance_guards(state, &written_vars);
                }
                forget_path_predicates(state, local_write_lhs_vars(self.hir, inner));
                if let Some(var_id) = lhs_local_var(self.hir, inner) {
                    state.internal_function_targets.remove(&var_id);
                }
                if self.reentrancy_balance_enabled
                    && let Some(var_id) = lhs_local_var(self.hir, inner)
                {
                    self.update_balance_local(state, var_id, None, false);
                    self.update_self_address_local(state, var_id, None);
                }
            }
            ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) =>
            {
                self.analyze_expr(inner, state);
                let written_vars = state_write_lhs_vars(self.hir, inner);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
                    self.invalidate_balance_guards(state, &written_vars);
                }
                forget_path_predicates(state, local_write_lhs_vars(self.hir, inner));
                if self.reentrancy_balance_enabled
                    && let Some(var_id) = lhs_local_var(self.hir, inner)
                {
                    self.update_self_address_local(state, var_id, None);
                }
            }
            ExprKind::Unary(_, inner) => {
                self.analyze_expr(inner, state);
            }
            ExprKind::Call(callee, args, opts) => {
                let uses_delegate_context = call_uses_delegate_context(self.gcx, callee);
                let mut operands = Vec::with_capacity(1 + args.len() + usize::from(opts.is_some()));
                operands.push(*callee);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        operands.push(&opt.value);
                    }
                }
                for arg in args.exprs() {
                    operands.push(arg);
                }

                let before_operands = state.clone();
                for operand in &operands {
                    self.analyze_expr(operand, state);
                }
                // Solidity does not specify operand evaluation order. Reversing the operands
                // covers both relative orders for each pair without changing the shared
                // reentrancy analysis.
                if self.reentrancy_balance_enabled && operands.len() > 1 {
                    let mut reverse_state = before_operands.balance_only();
                    self.analyze_with_only_balance(|this| {
                        for operand in operands.iter().rev() {
                            this.analyze_expr(operand, &mut reverse_state);
                        }
                    });
                    state.merge_balance(&reverse_state);
                }

                if self.reentrancy_balance_enabled
                    && is_require_or_assert(callee)
                    && let Some(cond) = args.exprs().next()
                {
                    self.emit_balance_calls(cond, state);
                }

                for func_id in self.resolved_internal_function_ids(callee, state) {
                    let returns = self.analyze_internal_call(func_id, args, state);
                    self.merge_call_balance_values(expr.span, returns);
                }
                if !state.state_reads.is_empty()
                    && let Some(kind) = self.reentrant_call_kind(callee, args, *opts)
                {
                    state.push_call(expr.span, kind);
                }
                if self.reentrancy_balance_enabled
                    && is_balance_reentrant_call(self.gcx, self.hir, callee, args, *opts)
                    && !self.balance_guard_blocks_call(state, callee)
                {
                    state.push_balance_call(expr.span);
                }
                if uses_delegate_context {
                    state
                        .invalidated_balance_guards
                        .extend(self.active_balance_guards.iter().copied());
                }
            }
            ExprKind::Binary(lhs, op, rhs)
                if self.reentrancy_balance_enabled
                    && matches!(op.kind, BinOpKind::And | BinOpKind::Or) =>
            {
                self.analyze_expr(lhs, state);

                let rhs_outcome = op.kind == BinOpKind::And;
                let mut short_state = state.clone();
                let short_reachable =
                    constrain_boolean_outcome(self.hir, lhs, !rhs_outcome, &mut short_state);
                let mut rhs_state = state.clone();
                let rhs_reachable =
                    constrain_boolean_outcome(self.hir, lhs, rhs_outcome, &mut rhs_state);
                if rhs_reachable {
                    self.analyze_expr(rhs, &mut rhs_state);
                }

                state.clear();
                if short_reachable {
                    state.merge(&short_state);
                }
                if rhs_reachable {
                    state.merge(&rhs_state);
                }
                state.path_predicates = match (short_reachable, rhs_reachable) {
                    (true, true) => common_path_predicates(
                        &short_state.path_predicates,
                        &rhs_state.path_predicates,
                    ),
                    (true, false) => short_state.path_predicates,
                    (false, true) => rhs_state.path_predicates,
                    (false, false) => PathPredicates::new(),
                };
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.analyze_expr(lhs, state);
                self.analyze_expr(rhs, state);
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
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.analyze_expr(cond, state);

                let mut true_state = state.clone();
                let mut false_state = state.clone();
                let predicate = self
                    .reentrancy_balance_enabled
                    .then(|| path_predicate(self.hir, cond))
                    .flatten();
                let true_reachable =
                    predicate.is_none_or(|predicate| true_state.constrain_path(predicate));
                let false_reachable = predicate
                    .is_none_or(|(var_id, value)| false_state.constrain_path((var_id, !value)));
                if true_reachable {
                    self.analyze_expr(true_expr, &mut true_state);
                }
                if false_reachable {
                    self.analyze_expr(false_expr, &mut false_state);
                }

                state.clear();
                if true_reachable {
                    state.merge(&true_state);
                }
                if false_reachable {
                    state.merge(&false_state);
                }
                state.path_predicates = match (true_reachable, false_reachable) {
                    (true, true) => common_path_predicates(
                        &true_state.path_predicates,
                        &false_state.path_predicates,
                    ),
                    (true, false) => true_state.path_predicates,
                    (false, true) => false_state.path_predicates,
                    (false, false) => PathPredicates::new(),
                };
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
            ExprKind::Member(base, _) | ExprKind::Payable(base) => {
                self.analyze_expr(base, state);
            }
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
            ExprKind::Ident(reses) => {
                for &res in *reses {
                    if let Res::Item(ItemId::Variable(var_id)) = res
                        && self.hir.variable(var_id).kind.is_state()
                    {
                        state.push_read(var_id);
                    }
                }
            }
            ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(
        &mut self,
        func_id: FunctionId,
        args: &CallArgs<'hir>,
        state: &mut FlowState,
    ) -> Vec<BalanceValue> {
        if self.call_stack.contains(&func_id) {
            return Vec::new();
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return Vec::new() };

        if self.reentrancy_balance_enabled {
            self.seed_balance_parameters(func, args, state);
        }
        let parameter_predicates = if self.reentrancy_balance_enabled {
            func.parameters
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    argument_for_parameter(self.hir, args, func.parameters, index)
                        .and_then(|arg| path_predicate(self.hir, arg))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let key = InlineCallKey {
            func_id,
            recursive_cut: self.first_recursive_cut(func_id),
            balance_only: self.balance_only_analysis,
            active_balance_guards: self.active_balance_guards.clone(),
            parameter_predicates: parameter_predicates.clone(),
            state: state.clone(),
        };
        if self.inline_cache.is_in_progress(&key) {
            self.clear_function_balance_locals(func_id, state);
            return Vec::new();
        }
        if let Some(cached) = self.inline_cache.get(&key) {
            let cached = cached.clone();
            *state =
                if self.balance_only_analysis { cached.state.balance_only() } else { cached.state };
            return cached.returns;
        }

        let mut after = state.clone();
        self.inline_cache.start(key.clone());
        if self.reentrancy_balance_enabled {
            self.return_collectors.push(ReturnCollector {
                func_id,
                values: vec![BalanceValue::default(); func.returns.len()],
            });
        }
        self.call_stack.push(func_id);
        let falls_through = self.analyze_callable(func, body, &mut after);
        self.call_stack.pop();

        let mut returns = if self.reentrancy_balance_enabled {
            if falls_through {
                self.record_return(None, &after);
            }
            self.return_collectors.pop().expect("return collector is active").values
        } else {
            Vec::new()
        };
        remap_return_paths(self.hir, func_id, func.parameters, &parameter_predicates, &mut returns);
        self.clear_function_balance_locals(func_id, &mut after);
        if self.balance_only_analysis {
            after = after.balance_only();
        }

        self.inline_cache
            .finish(key, InlineCallResult { state: after.clone(), returns: returns.clone() });
        *state = after;
        returns
    }

    fn resolved_internal_function_ids(
        &self,
        callee: &'hir hir::Expr<'hir>,
        state: &FlowState,
    ) -> BTreeSet<FunctionId> {
        if let Some(var_id) = lhs_local_var(self.hir, callee)
            && let Some(targets) = state.internal_function_targets.get(&var_id)
        {
            return targets.clone();
        }
        match &callee.peel_parens().kind {
            ExprKind::Ident(_) => {}
            ExprKind::Member(base, _) if is_super(base) => {}
            _ => return BTreeSet::new(),
        }
        let Some(ty) = self.gcx.type_of_expr(callee.peel_parens().id) else {
            return BTreeSet::new();
        };
        let TyKind::Fn(function) = ty.kind else { return BTreeSet::new() };
        function.is_internal().then_some(function.function_id).flatten().into_iter().collect()
    }

    fn update_internal_function_target(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        value: &'hir hir::Expr<'hir>,
    ) {
        let targets = self.resolved_internal_function_ids(value, state);
        state.internal_function_targets.remove(&var_id);
        if !targets.is_empty() {
            state.internal_function_targets.insert(var_id, targets);
        }
    }

    fn merge_call_balance_values(&mut self, span: Span, values: Vec<BalanceValue>) {
        let stored = self.call_balance_values.entry(span).or_default();
        if stored.len() < values.len() {
            stored.resize_with(values.len(), BalanceValue::default);
        }
        for (stored, value) in stored.iter_mut().zip(values) {
            stored.balance_dependent |= value.balance_dependent;
            stored.balance_paths.extend(value.balance_paths);
            stored.self_address_paths.extend(value.self_address_paths);
            stored.stale_calls.extend(value.stale_calls);
            extend_unique(&mut stored.stale_comparisons, value.stale_comparisons);
        }
    }

    fn analyze_with_only_balance<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let reentrancy_eth_enabled = self.reentrancy_eth_enabled;
        let reentrancy_no_eth_enabled = self.reentrancy_no_eth_enabled;
        let balance_only_analysis = self.balance_only_analysis;
        self.reentrancy_eth_enabled = false;
        self.reentrancy_no_eth_enabled = false;
        self.balance_only_analysis = true;
        let result = f(self);
        self.reentrancy_eth_enabled = reentrancy_eth_enabled;
        self.reentrancy_no_eth_enabled = reentrancy_no_eth_enabled;
        self.balance_only_analysis = balance_only_analysis;
        result
    }

    fn first_recursive_cut(&mut self, func_id: FunctionId) -> Option<FunctionId> {
        let active_call_stack = self.call_stack.iter().copied().collect::<BTreeSet<_>>();
        if active_call_stack.is_empty() {
            return None;
        }

        let active_call_stack = active_call_stack.into_iter().collect::<Vec<_>>();
        let key = RecursiveFrontierKey { func_id, active_call_stack };
        if let Some(frontier) = self.recursive_cut_frontiers.get(&key) {
            return frontier.first().copied();
        }

        let active_call_stack = key.active_call_stack.iter().copied().collect::<BTreeSet<_>>();
        let mut seen = HashSet::new();
        let cut = self.first_recursive_cut_function(func_id, &active_call_stack, &mut seen);
        self.recursive_cut_frontiers.insert(key, cut.into_iter().collect::<Vec<_>>());
        cut
    }

    fn first_recursive_cut_function(
        &mut self,
        func_id: FunctionId,
        active_call_stack: &BTreeSet<FunctionId>,
        seen: &mut HashSet<FunctionId>,
    ) -> Option<FunctionId> {
        if !seen.insert(func_id) {
            return None;
        }

        for callee_id in self.direct_internal_calls(func_id) {
            if active_call_stack.contains(&callee_id) {
                return Some(callee_id);
            }
            if let Some(cut) = self.first_recursive_cut_function(callee_id, active_call_stack, seen)
            {
                return Some(cut);
            }
        }
        None
    }

    fn direct_internal_calls(&mut self, func_id: FunctionId) -> Vec<FunctionId> {
        if let Some(calls) = self.direct_internal_calls.get(&func_id) {
            return calls.clone();
        }

        let mut calls = BTreeSet::new();
        let func = self.hir.function(func_id);
        for modifier in func.modifiers {
            for arg in modifier.args.exprs() {
                self.collect_direct_internal_calls_expr(arg, &mut calls);
            }
            if let Some(modifier_id) = modifier.id.as_function() {
                calls.insert(modifier_id);
            }
        }
        if let Some(body) = func.body {
            self.collect_direct_internal_calls_block(body, &mut calls);
        }

        let calls = calls.into_iter().collect::<Vec<_>>();
        self.direct_internal_calls.insert(func_id, calls.clone());
        calls
    }

    fn collect_direct_internal_calls_block(
        &mut self,
        block: hir::Block<'hir>,
        calls: &mut BTreeSet<FunctionId>,
    ) {
        for stmt in block.stmts {
            self.collect_direct_internal_calls_stmt(stmt, calls);
        }
    }

    fn collect_direct_internal_calls_stmt(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        calls: &mut BTreeSet<FunctionId>,
    ) {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.collect_direct_internal_calls_expr(init, calls);
                }
            }
            StmtKind::DeclMulti(_, expr)
            | StmtKind::Expr(expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr) => {
                self.collect_direct_internal_calls_expr(expr, calls);
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.collect_direct_internal_calls_expr(expr, calls);
                }
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
                self.collect_direct_internal_calls_block(block, calls);
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.collect_direct_internal_calls_expr(cond, calls);
                self.collect_direct_internal_calls_stmt(then_stmt, calls);
                if let Some(else_stmt) = else_stmt {
                    self.collect_direct_internal_calls_stmt(else_stmt, calls);
                }
            }
            StmtKind::Try(try_stmt) => {
                self.collect_direct_internal_calls_expr(&try_stmt.expr, calls);
                for clause in try_stmt.clauses {
                    self.collect_direct_internal_calls_block(clause.block, calls);
                }
            }
            StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Placeholder
            | StmtKind::AssemblyBlock(_)
            | StmtKind::Switch(_)
            | StmtKind::Err(_) => {}
        }
    }

    fn collect_direct_internal_calls_expr(
        &mut self,
        expr: &'hir hir::Expr<'hir>,
        calls: &mut BTreeSet<FunctionId>,
    ) {
        match &expr.kind {
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.collect_direct_internal_calls_expr(lhs, calls);
                self.collect_direct_internal_calls_expr(rhs, calls);
            }
            ExprKind::Unary(_, inner)
            | ExprKind::Delete(inner)
            | ExprKind::Member(inner, _)
            | ExprKind::Payable(inner) => {
                self.collect_direct_internal_calls_expr(inner, calls);
            }
            ExprKind::Call(callee, args, opts) => {
                self.collect_direct_internal_calls_expr(callee, calls);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        self.collect_direct_internal_calls_expr(&opt.value, calls);
                    }
                }
                for arg in args.exprs() {
                    self.collect_direct_internal_calls_expr(arg, calls);
                }
                for func_id in self.resolved_internal_function_ids(callee, &FlowState::default()) {
                    calls.insert(func_id);
                }
            }
            ExprKind::Index(base, index) => {
                self.collect_direct_internal_calls_expr(base, calls);
                if let Some(index) = index {
                    self.collect_direct_internal_calls_expr(index, calls);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.collect_direct_internal_calls_expr(base, calls);
                if let Some(start) = start {
                    self.collect_direct_internal_calls_expr(start, calls);
                }
                if let Some(end) = end {
                    self.collect_direct_internal_calls_expr(end, calls);
                }
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.collect_direct_internal_calls_expr(cond, calls);
                self.collect_direct_internal_calls_expr(true_expr, calls);
                self.collect_direct_internal_calls_expr(false_expr, calls);
            }
            ExprKind::Array(exprs) => {
                for expr in *exprs {
                    self.collect_direct_internal_calls_expr(expr, calls);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.collect_direct_internal_calls_expr(expr, calls);
                }
            }
            ExprKind::Ident(_)
            | ExprKind::Lit(_)
            | ExprKind::New(_)
            | ExprKind::TypeCall(_)
            | ExprKind::Type(_)
            | ExprKind::YulMember(..)
            | ExprKind::Err(_) => {}
        }
    }

    fn analyze_lhs_indices(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Index(base, index) => {
                self.analyze_lhs_indices(base, state);
                if let Some(index) = index {
                    self.analyze_expr(index, state);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_lhs_indices(base, state);
                if let Some(start) = start {
                    self.analyze_expr(start, state);
                }
                if let Some(end) = end {
                    self.analyze_expr(end, state);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs_indices(base, state);
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.analyze_lhs_indices(expr, state);
                }
            }
            _ => {}
        }
    }

    fn emit_pending_calls(&mut self, state: &FlowState, written_vars: &[VariableId]) {
        for call in &state.pending_calls {
            let (lint, msg_prefix) = match call.kind {
                ReentrantCallKind::Eth => {
                    (&REENTRANCY_ETH, "uncapped ETH transfer can be reentered before")
                }
                ReentrantCallKind::NoEth => {
                    (&REENTRANCY_NO_ETH, "external call can be reentered before")
                }
            };
            if !self.ctx.is_lint_enabled(lint.id) || self.emitted.contains(&call.span) {
                continue;
            }

            if let Some(var_id) =
                written_vars.iter().find(|&&var_id| call.state_reads.contains(&var_id))
            {
                let name = self
                    .hir
                    .variable(*var_id)
                    .name
                    .map(|name| name.as_str().to_string())
                    .unwrap_or_else(|| "state".to_string());
                self.ctx.emit_with_msg(
                    lint,
                    call.span,
                    format!("{msg_prefix} `{name}` is updated"),
                );
                self.emitted.insert(call.span);
            }
        }
    }

    fn emit_balance_calls(&mut self, guard: &'hir hir::Expr<'hir>, state: &FlowState) {
        for call in &state.pending_balance_calls {
            if self.emitted_balance.contains(&call.span)
                || !self.guard_has_stale_balance_comparison(guard, call, state)
            {
                continue;
            }

            self.ctx.emit(&REENTRANCY_BALANCE, call.span);
            self.emitted_balance.insert(call.span);
        }
    }

    fn guard_has_stale_balance_comparison(
        &self,
        expr: &'hir hir::Expr<'hir>,
        call: &PendingBalanceCall,
        state: &FlowState,
    ) -> bool {
        match &expr.peel_parens().kind {
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                if self.guard_has_stale_balance_comparison(lhs, call, state) {
                    return true;
                }
                let mut rhs_state = state.clone();
                constrain_boolean_outcome(self.hir, lhs, op.kind == BinOpKind::And, &mut rhs_state)
                    && self.guard_has_stale_balance_comparison(rhs, call, &rhs_state)
            }
            ExprKind::Binary(lhs, op, rhs) => {
                let is_comparison = matches!(
                    op.kind,
                    BinOpKind::Lt
                        | BinOpKind::Le
                        | BinOpKind::Gt
                        | BinOpKind::Ge
                        | BinOpKind::Eq
                        | BinOpKind::Ne
                );
                let current_locals = state
                    .balance_locals
                    .difference(&call.stale_locals)
                    .copied()
                    .collect::<BTreeSet<_>>();
                (is_comparison
                    && ((self.expr_depends_on_balance(
                        lhs,
                        &current_locals,
                        BalanceQuery::Current(call.span),
                        state,
                    ) && self.expr_depends_on_balance(
                        rhs,
                        &call.stale_locals,
                        BalanceQuery::Stale(call.span),
                        state,
                    )) || (self.expr_depends_on_balance(
                        rhs,
                        &current_locals,
                        BalanceQuery::Current(call.span),
                        state,
                    ) && self.expr_depends_on_balance(
                        lhs,
                        &call.stale_locals,
                        BalanceQuery::Stale(call.span),
                        state,
                    ))))
                    || self.guard_has_stale_balance_comparison(lhs, call, state)
                    || self.guard_has_stale_balance_comparison(rhs, call, state)
            }
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => {
                self.guard_has_stale_balance_comparison(inner, call, state)
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.guard_has_stale_balance_comparison(cond, call, state)
                    || self.guard_has_stale_balance_comparison(true_expr, call, state)
                    || self.guard_has_stale_balance_comparison(false_expr, call, state)
            }
            ExprKind::Call(callee, args, opts)
                if opts.is_none()
                    && matches!(
                        callee.peel_parens().kind,
                        ExprKind::Type(_) | ExprKind::TypeCall(_)
                    ) =>
            {
                args.exprs().any(|arg| self.guard_has_stale_balance_comparison(arg, call, state))
            }
            ExprKind::Call(_, _, _) => {
                self.call_balance_values.get(&expr.peel_parens().span).is_some_and(|values| {
                    values.iter().any(|value| value.stale_comparisons.contains(&call.span))
                })
            }
            ExprKind::Ident(reses) => reses.iter().any(|res| {
                matches!(res, Res::Item(ItemId::Variable(var_id)) if state
                    .balance_comparison_locals
                    .get(var_id)
                    .is_some_and(|calls| calls.contains(&call.span)))
            }),
            _ => false,
        }
    }

    fn update_balance_local(
        &mut self,
        state: &mut FlowState,
        var_id: VariableId,
        value: Option<&'hir hir::Expr<'hir>>,
        reads_old_value: bool,
    ) {
        let value = value.map(|value| self.balance_dependency(value, state)).unwrap_or_default();
        self.update_balance_local_with_value(state, var_id, &value, reads_old_value);
    }

    fn update_self_address_local(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        value: Option<&hir::Expr<'_>>,
    ) {
        let paths = value
            .and_then(|value| self.self_address_dependencies(value, state).into_iter().next())
            .unwrap_or_default();
        self.update_self_address_local_with_paths(state, var_id, paths);
    }

    fn update_self_address_local_with_paths(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        paths: PathAlternatives,
    ) {
        state.self_address_local_paths.remove(&var_id);
        if !paths.is_empty() {
            state.self_address_local_paths.insert(var_id, paths);
        }
    }

    fn update_self_address_assignment(
        &self,
        state: &mut FlowState,
        lhs: &hir::Expr<'_>,
        rhs: &hir::Expr<'_>,
        reads_old_value: bool,
    ) {
        if let ExprKind::Tuple(lhs_values) = &lhs.peel_parens().kind {
            let values = self.self_address_dependencies(rhs, state);
            for (lhs, paths) in lhs_values.iter().zip(values) {
                if let Some(var_id) = lhs.and_then(|lhs| lhs_local_var(self.hir, lhs)) {
                    self.update_self_address_local_with_paths(state, var_id, paths);
                }
            }
        } else if let Some(var_id) = lhs_local_var(self.hir, lhs) {
            if reads_old_value {
                self.update_self_address_local_with_paths(state, var_id, PathAlternatives::new());
            } else {
                self.update_self_address_local(state, var_id, Some(rhs));
            }
        }
    }

    fn update_self_address_vars(
        &self,
        state: &mut FlowState,
        vars: impl Iterator<Item = Option<VariableId>>,
        value: &hir::Expr<'_>,
    ) {
        for (var_id, paths) in vars.zip(self.self_address_dependencies(value, state)) {
            if let Some(var_id) = var_id {
                self.update_self_address_local_with_paths(state, var_id, paths);
            }
        }
    }

    fn self_address_dependencies(
        &self,
        expr: &hir::Expr<'_>,
        state: &FlowState,
    ) -> Vec<PathAlternatives> {
        match &expr.peel_parens().kind {
            ExprKind::Tuple(exprs) => exprs
                .iter()
                .map(|expr| {
                    expr.map(|expr| self.self_address_path(expr, state)).unwrap_or_default()
                })
                .collect(),
            ExprKind::Call(_, _, _) => self
                .call_balance_values
                .get(&expr.span)
                .map(|values| values.iter().map(|value| value.self_address_paths.clone()).collect())
                .unwrap_or_else(|| vec![self.self_address_path(expr, state)]),
            _ => vec![self.self_address_path(expr, state)],
        }
    }

    fn self_address_path(&self, expr: &hir::Expr<'_>, state: &FlowState) -> PathAlternatives {
        let expr = expr.peel_parens();
        if let ExprKind::Call(_, _, _) = expr.kind
            && let Some(value) =
                self.call_balance_values.get(&expr.span).and_then(|values| values.first())
        {
            return constrain_paths(&value.self_address_paths, &state.path_predicates);
        }
        match &expr.kind {
            ExprKind::Payable(inner) => self.self_address_path(inner, state),
            ExprKind::Call(callee, args, opts)
                if opts.is_none() && is_address_type_expr(callee) && args.exprs().count() == 1 =>
            {
                args.exprs()
                    .next()
                    .map(|arg| self.self_address_path(arg, state))
                    .unwrap_or_default()
            }
            _ => self_address_paths(expr, state),
        }
    }

    fn self_balance_paths(&self, expr: &hir::Expr<'_>, state: &FlowState) -> PathAlternatives {
        let ExprKind::Member(base, member) = &expr.peel_parens().kind else {
            return PathAlternatives::new();
        };
        if member.as_str() != "balance" {
            return PathAlternatives::new();
        }
        self.self_address_path(base, state)
    }

    fn update_balance_assignment(
        &mut self,
        state: &mut FlowState,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        reads_old_value: bool,
    ) {
        if let ExprKind::Tuple(lhs_values) = &lhs.peel_parens().kind {
            let values = self.balance_dependencies(rhs, state);
            for (lhs, value) in lhs_values.iter().zip(values.iter()) {
                if let Some(var_id) = lhs.and_then(|lhs| lhs_local_var(self.hir, lhs)) {
                    self.update_balance_local_with_value(state, var_id, value, false);
                }
            }
        } else if let Some(var_id) = lhs_local_var(self.hir, lhs) {
            self.update_balance_local(state, var_id, Some(rhs), reads_old_value);
        }
    }

    fn update_balance_vars(
        &mut self,
        state: &mut FlowState,
        vars: impl Iterator<Item = Option<VariableId>>,
        value: &'hir hir::Expr<'hir>,
    ) {
        let values = self.balance_dependencies(value, state);
        for (var_id, value) in vars.zip(values.iter()) {
            if let Some(var_id) = var_id {
                self.update_balance_local_with_value(state, var_id, value, false);
            }
        }
    }

    fn update_balance_local_with_value(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        value: &BalanceValue,
        reads_old_value: bool,
    ) {
        let balance_dependent =
            value.balance_dependent || (reads_old_value && state.balance_locals.contains(&var_id));
        let mut balance_paths = value.balance_paths.clone();
        let mut stale_comparisons = value.stale_comparisons.clone();
        if reads_old_value {
            if let Some(old_paths) = state.balance_local_paths.get(&var_id) {
                balance_paths.extend(old_paths.iter().cloned());
            }
            if let Some(old_comparisons) = state.balance_comparison_locals.get(&var_id) {
                extend_unique(&mut stale_comparisons, old_comparisons.iter().copied());
            }
        }

        for call in &mut state.pending_balance_calls {
            let stale = value.stale_calls.contains(&call.span)
                || (reads_old_value && call.stale_locals.contains(&var_id));
            call.stale_locals.remove(&var_id);
            if stale {
                call.stale_locals.insert(var_id);
            }
        }

        state.balance_locals.remove(&var_id);
        state.balance_local_paths.remove(&var_id);
        state.balance_comparison_locals.remove(&var_id);
        if balance_dependent {
            state.balance_locals.insert(var_id);
            state.balance_local_paths.insert(var_id, balance_paths);
        }
        if !stale_comparisons.is_empty() {
            state.balance_comparison_locals.insert(var_id, stale_comparisons);
        }
    }

    fn balance_dependencies(
        &self,
        expr: &'hir hir::Expr<'hir>,
        state: &FlowState,
    ) -> Vec<BalanceValue> {
        match &expr.peel_parens().kind {
            ExprKind::Tuple(exprs) => exprs
                .iter()
                .map(|expr| {
                    expr.map(|expr| self.balance_dependency(expr, state)).unwrap_or_default()
                })
                .collect(),
            ExprKind::Call(_, _, _) => self
                .call_balance_values
                .get(&expr.span)
                .cloned()
                .unwrap_or_else(|| vec![self.balance_dependency(expr, state)]),
            _ => vec![self.balance_dependency(expr, state)],
        }
    }

    fn balance_dependency(&self, expr: &'hir hir::Expr<'hir>, state: &FlowState) -> BalanceValue {
        let balance_paths = self.expr_balance_paths(expr, state);
        let balance_dependent = !balance_paths.is_empty();
        let self_address_paths = self.self_address_path(expr, state);
        let stale_calls = state
            .pending_balance_calls
            .iter()
            .filter(|call| {
                self.expr_depends_on_balance(
                    expr,
                    &call.stale_locals,
                    BalanceQuery::Stale(call.span),
                    state,
                )
            })
            .map(|call| call.span)
            .collect();
        let stale_comparisons = state
            .pending_balance_calls
            .iter()
            .filter(|call| self.guard_has_stale_balance_comparison(expr, call, state))
            .map(|call| call.span)
            .collect();
        BalanceValue {
            balance_dependent,
            balance_paths,
            self_address_paths,
            stale_calls,
            stale_comparisons,
        }
    }

    fn expr_balance_paths(
        &self,
        expr: &'hir hir::Expr<'hir>,
        state: &FlowState,
    ) -> PathAlternatives {
        let expr = expr.peel_parens();
        let self_balance_paths = self.self_balance_paths(expr, state);
        if !self_balance_paths.is_empty() {
            return self_balance_paths;
        }

        match &expr.kind {
            ExprKind::Ident(reses) => {
                let mut paths = PathAlternatives::new();
                for var_id in reses.iter().filter_map(|res| match res {
                    Res::Item(ItemId::Variable(var_id)) => Some(var_id),
                    _ => None,
                }) {
                    if let Some(local_paths) = state.balance_local_paths.get(var_id) {
                        paths.extend(constrain_paths(local_paths, &state.path_predicates));
                    }
                }
                paths
            }
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => {
                self.expr_balance_paths(inner, state)
            }
            ExprKind::Binary(lhs, _, rhs) => {
                let mut paths = self.expr_balance_paths(lhs, state);
                paths.extend(self.expr_balance_paths(rhs, state));
                paths
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                let mut paths = self.expr_balance_paths(cond, state);
                paths.extend(self.expr_balance_paths(true_expr, state));
                paths.extend(self.expr_balance_paths(false_expr, state));
                paths
            }
            ExprKind::Call(callee, args, opts)
                if opts.is_none()
                    && matches!(
                        callee.peel_parens().kind,
                        ExprKind::Type(_) | ExprKind::TypeCall(_)
                    ) =>
            {
                let mut paths = PathAlternatives::new();
                for arg in args.exprs() {
                    paths.extend(self.expr_balance_paths(arg, state));
                }
                paths
            }
            ExprKind::Call(_, _, _) => {
                let mut paths = PathAlternatives::new();
                if let Some(values) = self.call_balance_values.get(&expr.span) {
                    for value in values {
                        paths.extend(constrain_paths(&value.balance_paths, &state.path_predicates));
                    }
                }
                paths
            }
            _ => PathAlternatives::new(),
        }
    }

    fn expr_depends_on_balance(
        &self,
        expr: &'hir hir::Expr<'hir>,
        locals: &BTreeSet<VariableId>,
        query: BalanceQuery,
        state: &FlowState,
    ) -> bool {
        let expr = expr.peel_parens();
        let self_balance_paths = self.self_balance_paths(expr, state);
        if !self_balance_paths.is_empty() {
            return match query {
                BalanceQuery::Current(call) => state
                    .pending_balance_calls
                    .iter()
                    .find(|pending| pending.span == call)
                    .is_some_and(|pending| {
                        path_alternatives_compatible(&self_balance_paths, &pending.paths)
                    }),
                BalanceQuery::Stale(_) => false,
            };
        }

        match &expr.kind {
            ExprKind::Ident(reses) => reses.iter().any(
                |res| matches!(res, Res::Item(ItemId::Variable(var_id)) if locals.contains(var_id)),
            ),
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => {
                self.expr_depends_on_balance(inner, locals, query, state)
            }
            ExprKind::Binary(lhs, _, rhs) => {
                self.expr_depends_on_balance(lhs, locals, query, state)
                    || self.expr_depends_on_balance(rhs, locals, query, state)
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.expr_depends_on_balance(cond, locals, query, state)
                    || self.expr_depends_on_balance(true_expr, locals, query, state)
                    || self.expr_depends_on_balance(false_expr, locals, query, state)
            }
            ExprKind::Call(callee, args, opts)
                if opts.is_none()
                    && matches!(
                        callee.peel_parens().kind,
                        ExprKind::Type(_) | ExprKind::TypeCall(_)
                    ) =>
            {
                args.exprs().any(|arg| self.expr_depends_on_balance(arg, locals, query, state))
            }
            ExprKind::Call(_, _, _) => {
                self.call_balance_values.get(&expr.span).is_some_and(|values| {
                    values.iter().any(|value| match query {
                        BalanceQuery::Current(call) => {
                            value.balance_dependent && !value.stale_calls.contains(&call)
                        }
                        BalanceQuery::Stale(call) => value.stale_calls.contains(&call),
                    })
                })
            }
            _ => false,
        }
    }

    fn seed_balance_parameters(
        &mut self,
        func: &'hir hir::Function<'hir>,
        args: &CallArgs<'hir>,
        state: &mut FlowState,
    ) {
        if !self.reentrancy_balance_enabled {
            return;
        }
        let values = func
            .parameters
            .iter()
            .enumerate()
            .map(|(index, &param)| {
                let argument = argument_for_parameter(self.hir, args, func.parameters, index);
                let value =
                    argument.map(|arg| self.balance_dependency(arg, state)).unwrap_or_default();
                let self_address_paths =
                    argument.map(|arg| self.self_address_path(arg, state)).unwrap_or_default();
                (param, value, self_address_paths)
            })
            .collect::<Vec<_>>();
        for (param, value, self_address_paths) in values {
            self.update_balance_local_with_value(state, param, &value, false);
            self.update_self_address_local_with_paths(state, param, self_address_paths);
        }
    }

    fn record_return(&mut self, expr: Option<&'hir hir::Expr<'hir>>, state: &FlowState) {
        let Some(func_id) = self.return_collectors.last().map(|collector| collector.func_id) else {
            return;
        };
        let func = self.hir.function(func_id);
        let values = if let Some(expr) = expr {
            self.balance_dependencies(expr, state)
        } else {
            func.returns
                .iter()
                .map(|var_id| {
                    let balance_dependent = state.balance_locals.contains(var_id);
                    let balance_paths =
                        state.balance_local_paths.get(var_id).cloned().unwrap_or_default();
                    let self_address_paths =
                        state.self_address_local_paths.get(var_id).cloned().unwrap_or_default();
                    let stale_calls = state
                        .pending_balance_calls
                        .iter()
                        .filter(|call| call.stale_locals.contains(var_id))
                        .map(|call| call.span)
                        .collect();
                    let stale_comparisons =
                        state.balance_comparison_locals.get(var_id).cloned().unwrap_or_default();
                    BalanceValue {
                        balance_dependent,
                        balance_paths,
                        self_address_paths,
                        stale_calls,
                        stale_comparisons,
                    }
                })
                .collect()
        };
        let collector = self.return_collectors.last_mut().expect("return collector is active");
        for (stored, value) in collector.values.iter_mut().zip(values) {
            stored.balance_dependent |= value.balance_dependent;
            stored.balance_paths.extend(value.balance_paths);
            stored.self_address_paths.extend(value.self_address_paths);
            stored.stale_calls.extend(value.stale_calls);
            extend_unique(&mut stored.stale_comparisons, value.stale_comparisons);
        }
    }

    fn clear_function_balance_locals(&self, func_id: FunctionId, state: &mut FlowState) {
        let belongs_to_function = |var_id: &VariableId| {
            self.hir.variable(*var_id).parent == Some(ItemId::Function(func_id))
        };
        state.internal_function_targets.retain(|var_id, _| !belongs_to_function(var_id));
        state.balance_locals.retain(|var_id| !belongs_to_function(var_id));
        state.self_address_local_paths.retain(|var_id, _| !belongs_to_function(var_id));
        state.balance_local_paths.retain(|var_id, _| !belongs_to_function(var_id));
        state.balance_comparison_locals.retain(|var_id, _| !belongs_to_function(var_id));
        state.path_predicates.retain(|var_id, _| !belongs_to_function(var_id));
        for call in &mut state.pending_balance_calls {
            call.stale_locals.retain(|var_id| !belongs_to_function(var_id));
        }
    }

    fn reentrant_call_kind(
        &self,
        callee: &'hir hir::Expr<'hir>,
        args: &CallArgs<'hir>,
        opts: Option<&hir::CallOptions<'hir>>,
    ) -> Option<ReentrantCallKind> {
        if self.reentrancy_eth_enabled && is_uncapped_value_call(self.hir, callee, opts) {
            return Some(ReentrantCallKind::Eth);
        }
        if self.reentrancy_no_eth_enabled
            && is_no_eth_reentrant_call(self.gcx, self.hir, callee, args, opts)
        {
            return Some(ReentrantCallKind::NoEth);
        }
        None
    }

    fn invalidate_balance_guards(&self, state: &mut FlowState, written_vars: &[VariableId]) {
        state.invalidated_balance_guards.extend(
            written_vars
                .iter()
                .filter(|var_id| self.active_balance_guards.contains(var_id))
                .copied(),
        );
    }

    fn balance_guard_blocks_call(&self, state: &FlowState, callee: &'hir hir::Expr<'hir>) -> bool {
        !call_uses_delegate_context(self.gcx, callee)
            && self.balance_reentry_lock.is_some_and(|reentry_lock| {
                self.active_balance_guards.iter().any(|lock_var| {
                    *lock_var == reentry_lock
                        && !state.invalidated_balance_guards.contains(lock_var)
                })
            })
    }
}

impl FlowState {
    fn clear(&mut self) {
        self.state_reads.clear();
        self.pending_calls.clear();
        self.internal_function_targets.clear();
        self.self_address_local_paths.clear();
        self.balance_locals.clear();
        self.balance_local_paths.clear();
        self.balance_comparison_locals.clear();
        self.pending_balance_calls.clear();
        self.invalidated_balance_guards.clear();
        self.path_predicates.clear();
    }

    fn merge(&mut self, other: &Self) {
        self.state_reads.extend(other.state_reads.iter().copied());
        for call in &other.pending_calls {
            if let Some(existing) = self
                .pending_calls
                .iter_mut()
                .find(|existing| existing.span == call.span && existing.kind == call.kind)
            {
                existing.state_reads.extend(call.state_reads.iter().copied());
            } else {
                self.pending_calls.push(call.clone());
            }
        }
        for (var_id, targets) in &other.internal_function_targets {
            self.internal_function_targets
                .entry(*var_id)
                .or_default()
                .extend(targets.iter().copied());
        }
        merge_balance_local_paths(
            &mut self.self_address_local_paths,
            &other.self_address_local_paths,
        );
        self.balance_locals.extend(other.balance_locals.iter().copied());
        merge_balance_local_paths(&mut self.balance_local_paths, &other.balance_local_paths);
        merge_comparison_locals(
            &mut self.balance_comparison_locals,
            &other.balance_comparison_locals,
        );
        self.invalidated_balance_guards.extend(other.invalidated_balance_guards.iter().copied());
        for call in &other.pending_balance_calls {
            if let Some(existing) =
                self.pending_balance_calls.iter_mut().find(|existing| existing.span == call.span)
            {
                existing.stale_locals.extend(call.stale_locals.iter().copied());
                existing.paths.extend(call.paths.iter().cloned());
            } else {
                self.pending_balance_calls.push(call.clone());
            }
        }
    }

    fn balance_only(&self) -> Self {
        Self {
            internal_function_targets: self.internal_function_targets.clone(),
            self_address_local_paths: self.self_address_local_paths.clone(),
            balance_locals: self.balance_locals.clone(),
            balance_local_paths: self.balance_local_paths.clone(),
            balance_comparison_locals: self.balance_comparison_locals.clone(),
            pending_balance_calls: self.pending_balance_calls.clone(),
            invalidated_balance_guards: self.invalidated_balance_guards.clone(),
            path_predicates: self.path_predicates.clone(),
            ..Self::default()
        }
    }

    fn merge_balance(&mut self, other: &Self) {
        merge_balance_local_paths(
            &mut self.self_address_local_paths,
            &other.self_address_local_paths,
        );
        for (var_id, targets) in &other.internal_function_targets {
            self.internal_function_targets
                .entry(*var_id)
                .or_default()
                .extend(targets.iter().copied());
        }
        self.balance_locals.extend(other.balance_locals.iter().copied());
        merge_balance_local_paths(&mut self.balance_local_paths, &other.balance_local_paths);
        merge_comparison_locals(
            &mut self.balance_comparison_locals,
            &other.balance_comparison_locals,
        );
        self.invalidated_balance_guards.extend(other.invalidated_balance_guards.iter().copied());
        for call in &other.pending_balance_calls {
            if let Some(existing) =
                self.pending_balance_calls.iter_mut().find(|existing| existing.span == call.span)
            {
                existing.stale_locals.extend(call.stale_locals.iter().copied());
                existing.paths.extend(call.paths.iter().cloned());
            } else {
                self.pending_balance_calls.push(call.clone());
            }
        }
    }

    fn constrain_path(&mut self, (var_id, value): (VariableId, bool)) -> bool {
        match self.path_predicates.get(&var_id) {
            Some(existing) => *existing == value,
            None => {
                self.path_predicates.insert(var_id, value);
                for paths in self.self_address_local_paths.values_mut() {
                    *paths = constrain_paths(paths, &self.path_predicates);
                }
                true
            }
        }
    }
}

fn path_predicate(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<(VariableId, bool)> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var_id = unique(reses.iter().filter_map(|res| match res {
                Res::Item(ItemId::Variable(var_id)) if !hir.variable(*var_id).kind.is_state() => {
                    Some(*var_id)
                }
                _ => None,
            }))?;
            Some((var_id, true))
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            path_predicate(hir, inner).map(|(var_id, value)| (var_id, !value))
        }
        _ => None,
    }
}

fn constrain_boolean_outcome(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    outcome: bool,
    state: &mut FlowState,
) -> bool {
    if let Some((var_id, value)) = path_predicate(hir, expr) {
        return state.constrain_path((var_id, if outcome { value } else { !value }));
    }
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs)
            if (outcome && op.kind == BinOpKind::And) || (!outcome && op.kind == BinOpKind::Or) =>
        {
            constrain_boolean_outcome(hir, lhs, outcome, state)
                && constrain_boolean_outcome(hir, rhs, outcome, state)
        }
        _ => true,
    }
}

fn common_path_predicates(lhs: &PathPredicates, rhs: &PathPredicates) -> PathPredicates {
    lhs.iter()
        .filter(|(var_id, value)| rhs.get(var_id) == Some(*value))
        .map(|(var_id, value)| (*var_id, *value))
        .collect()
}

fn paths_compatible(lhs: &PathPredicates, rhs: &PathPredicates) -> bool {
    lhs.iter().all(|(var_id, value)| rhs.get(var_id).is_none_or(|other| other == value))
}

fn path_alternatives_compatible(lhs: &PathAlternatives, rhs: &PathAlternatives) -> bool {
    lhs.iter().any(|lhs| rhs.iter().any(|rhs| paths_compatible(lhs, rhs)))
}

fn paths_compatible_with(paths: &PathAlternatives, active: &PathPredicates) -> bool {
    paths.iter().any(|path| paths_compatible(path, active))
}

fn constrain_paths(paths: &PathAlternatives, active: &PathPredicates) -> PathAlternatives {
    paths
        .iter()
        .filter(|path| paths_compatible(path, active))
        .map(|path| {
            let mut constrained = path.clone();
            constrained.extend(active.iter().map(|(var_id, value)| (*var_id, *value)));
            constrained
        })
        .collect()
}

fn remap_return_paths(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    parameters: &[VariableId],
    parameter_predicates: &[Option<(VariableId, bool)>],
    values: &mut [BalanceValue],
) {
    for value in values {
        value.balance_paths =
            remap_paths(hir, func_id, parameters, parameter_predicates, &value.balance_paths);
        value.self_address_paths =
            remap_paths(hir, func_id, parameters, parameter_predicates, &value.self_address_paths);
        value.balance_dependent = !value.balance_paths.is_empty();
    }
}

fn remap_paths(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    parameters: &[VariableId],
    parameter_predicates: &[Option<(VariableId, bool)>],
    paths: &PathAlternatives,
) -> PathAlternatives {
    paths
        .iter()
        .filter_map(|path| {
            let mut path = path.clone();
            for (&parameter, &argument) in parameters.iter().zip(parameter_predicates) {
                let Some(parameter_value) = path.remove(&parameter) else { continue };
                let Some((argument_var, argument_value)) = argument else { continue };
                let mapped_value = if parameter_value { argument_value } else { !argument_value };
                if path.get(&argument_var).is_some_and(|existing| *existing != mapped_value) {
                    return None;
                }
                path.insert(argument_var, mapped_value);
            }
            path.retain(|var_id, _| {
                hir.variable(*var_id).parent != Some(ItemId::Function(func_id))
            });
            Some(path)
        })
        .collect()
}

fn merge_balance_local_paths(
    stored: &mut BTreeMap<VariableId, PathAlternatives>,
    other: &BTreeMap<VariableId, PathAlternatives>,
) {
    for (var_id, paths) in other {
        stored.entry(*var_id).or_default().extend(paths.iter().cloned());
    }
}

fn merge_comparison_locals(
    stored: &mut BTreeMap<VariableId, Vec<Span>>,
    other: &BTreeMap<VariableId, Vec<Span>>,
) {
    for (var_id, comparisons) in other {
        extend_unique(stored.entry(*var_id).or_default(), comparisons.iter().copied());
    }
}

fn is_balance_reentrant_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    _args: &CallArgs<'hir>,
    opts: Option<&hir::CallOptions<'hir>>,
) -> bool {
    if !call_options_allow_reentrancy(hir, opts) {
        return false;
    }

    match &callee.peel_parens().kind {
        ExprKind::Member(receiver, _) if is_contract_receiver(gcx, receiver) => {
            external_call_can_reenter(gcx, callee)
        }
        ExprKind::Member(receiver, member)
            if is_address_like(gcx, hir, receiver)
                && matches!(
                    member.name,
                    kw::Call | kw::Callcode | kw::Delegatecall | kw::Staticcall
                ) =>
        {
            member.name != kw::Staticcall
        }
        ExprKind::Member(receiver, _) if is_super(receiver) => false,
        _ => external_call_can_reenter(gcx, callee),
    }
}

fn call_options_allow_reentrancy(hir: &hir::Hir<'_>, opts: Option<&hir::CallOptions<'_>>) -> bool {
    let Some(opts) = opts else { return true };
    let Some(gas) = opts.args.iter().find(|opt| opt.name.name == kw::Gas) else {
        return true;
    };
    let may_transfer_value = opts
        .args
        .iter()
        .find(|opt| opt.name.name == sym::value)
        .is_some_and(|value| !is_zero_value(hir, &value.value));
    let mut seen = BTreeSet::new();
    concrete_gas_cap(hir, &gas.value, &mut seen).is_none_or(|gas| {
        gas > U256::from(REENTRANCY_GAS_STIPEND) || (may_transfer_value && !gas.is_zero())
    })
}

fn concrete_gas_cap(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut BTreeSet<VariableId>,
) -> Option<U256> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Number(value) => Some(value),
            _ => None,
        },
        ExprKind::Ident(reses) => {
            let var_id = unique(reses.iter().filter_map(|res| match res {
                Res::Item(ItemId::Variable(var_id)) => Some(*var_id),
                _ => None,
            }))?;
            let var = hir.variable(var_id);
            if !var.is_constant() || !seen.insert(var_id) {
                return None;
            }
            concrete_gas_cap(hir, var.initializer?, seen)
        }
        ExprKind::Call(callee, args, opts)
            if opts.is_none()
                && matches!(
                    callee.peel_parens().kind,
                    ExprKind::Type(_) | ExprKind::TypeCall(_)
                )
                && args.exprs().count() == 1 =>
        {
            concrete_gas_cap(hir, args.exprs().next()?, seen)
        }
        _ => None,
    }
}

fn branch_stops_current_path(stmt: &hir::Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Break | StmtKind::Continue => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(branch_stops_current_path)
        }
        StmtKind::If(_, then_stmt, Some(else_stmt)) => {
            branch_stops_current_path(then_stmt) && branch_stops_current_path(else_stmt)
        }
        _ => branch_always_exits(stmt),
    }
}

fn standard_reentrancy_guard_lock(
    hir: &hir::Hir<'_>,
    modifier: &hir::Function<'_>,
) -> Option<VariableId> {
    if !matches!(modifier.kind, hir::FunctionKind::Modifier) || !modifier.modifiers.is_empty() {
        return None;
    }
    let body = modifier.body?;
    if body.stmts.iter().map(count_modifier_placeholders).sum::<usize>() != 1 {
        return None;
    }
    let mut activation_stmts = Vec::new();
    collect_stmts_before_unconditional_placeholder(body.stmts, &mut activation_stmts)?;
    let mut seen = BTreeSet::new();
    let (lock_var, entered) = guard_activation_from_stmt_refs(hir, &activation_stmts, &mut seen)?;
    let placeholder_index = body.stmts.iter().position(contains_unconditional_placeholder)?;
    let mut seen = BTreeSet::new();
    let (restored_var, restored) =
        guard_restoration_from_stmt(hir, body.stmts.get(placeholder_index + 1)?, &mut seen)?;
    (lock_var == restored_var && entered != restored).then_some(lock_var)
}

fn collect_stmts_before_unconditional_placeholder<'hir>(
    stmts: &'hir [hir::Stmt<'hir>],
    before: &mut Vec<&'hir hir::Stmt<'hir>>,
) -> Option<()> {
    for stmt in stmts {
        match stmt.kind {
            StmtKind::Placeholder => return Some(()),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block)
                if contains_unconditional_placeholder(stmt) =>
            {
                return collect_stmts_before_unconditional_placeholder(block.stmts, before);
            }
            _ => before.push(stmt),
        }
    }
    None
}

fn contains_unconditional_placeholder(stmt: &hir::Stmt<'_>) -> bool {
    match stmt.kind {
        StmtKind::Placeholder => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(contains_unconditional_placeholder)
        }
        _ => false,
    }
}

fn count_modifier_placeholders(stmt: &hir::Stmt<'_>) -> usize {
    match stmt.kind {
        StmtKind::Placeholder => 1,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().map(count_modifier_placeholders).sum()
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            count_modifier_placeholders(then_stmt)
                + else_stmt.map_or(0, count_modifier_placeholders)
        }
        StmtKind::Try(try_stmt) => try_stmt
            .clauses
            .iter()
            .flat_map(|clause| clause.block.stmts)
            .map(count_modifier_placeholders)
            .sum(),
        StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => 2,
        StmtKind::DeclSingle(_)
        | StmtKind::DeclMulti(_, _)
        | StmtKind::Emit(_)
        | StmtKind::Revert(_)
        | StmtKind::Return(_)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Expr(_) => 0,
    }
}

fn balance_reentry_lock<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    entry: &'hir hir::Function<'hir>,
) -> Option<VariableId> {
    let entry_id = hir.function_ids().find(|&id| std::ptr::eq(hir.function(id), entry))?;
    let defining_contract = entry.contract?;
    reentrancy_guard_locks(hir, entry).into_iter().find(|&lock_var| {
        let mut has_effective_deployment = false;
        for contract_id in hir.contract_ids() {
            let contract = hir.contract(contract_id);
            if !contract.can_be_deployed()
                || contract.is_abstract()
                || !contract.linearized_bases.contains(&defining_contract)
            {
                continue;
            }

            let interface = gcx.interface_functions(contract_id);
            let entry_is_effective = interface.iter().any(|function| function.id == entry_id)
                || contract.fallback == Some(entry_id)
                || contract.receive == Some(entry_id);
            if !entry_is_effective {
                continue;
            }
            has_effective_deployment = true;

            let ordinary_entries_are_guarded = interface.iter().all(|function| {
                let function = hir.function(function.id);
                matches!(function.state_mutability, StateMutability::Pure | StateMutability::View)
                    || function_has_reentrancy_guard(hir, function, lock_var)
            });
            let special_entries_are_guarded =
                [contract.fallback, contract.receive].into_iter().flatten().all(|function_id| {
                    function_has_reentrancy_guard(hir, hir.function(function_id), lock_var)
                });
            if !ordinary_entries_are_guarded || !special_entries_are_guarded {
                return false;
            }
        }
        has_effective_deployment
    })
}

fn reentrancy_guard_locks(hir: &hir::Hir<'_>, function: &hir::Function<'_>) -> Vec<VariableId> {
    function
        .modifiers
        .iter()
        .filter(|modifier| modifier.args.exprs().next().is_none())
        .filter_map(|modifier| modifier.id.as_function())
        .filter_map(|modifier_id| standard_reentrancy_guard_lock(hir, hir.function(modifier_id)))
        .collect()
}

fn function_has_reentrancy_guard(
    hir: &hir::Hir<'_>,
    function: &hir::Function<'_>,
    lock_var: VariableId,
) -> bool {
    reentrancy_guard_locks(hir, function).contains(&lock_var)
}

fn guard_activation_from_stmts(
    hir: &hir::Hir<'_>,
    stmts: &[hir::Stmt<'_>],
    seen: &mut BTreeSet<FunctionId>,
) -> Option<(VariableId, LockValue)> {
    let stmts = stmts.iter().collect::<Vec<_>>();
    guard_activation_from_stmt_refs(hir, &stmts, seen)
}

fn guard_activation_from_stmt_refs(
    hir: &hir::Hir<'_>,
    stmts: &[&hir::Stmt<'_>],
    seen: &mut BTreeSet<FunctionId>,
) -> Option<(VariableId, LockValue)> {
    let (activation, prefix) = stmts.split_last()?;
    if let Some((lock_var, entered)) = state_lock_assignment(hir, activation) {
        return prefix
            .iter()
            .any(|stmt| stmt_rejects_lock_value(hir, stmt, lock_var, entered))
            .then_some((lock_var, entered));
    }

    let helper_id = simple_internal_call(activation)?;
    if !seen.insert(helper_id) {
        return None;
    }
    let helper = hir.function(helper_id);
    let result = if helper.modifiers.is_empty() {
        guard_activation_from_stmts(hir, helper.body?.stmts, seen)
    } else {
        None
    };
    seen.remove(&helper_id);
    result
}

fn guard_restoration_from_stmt(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    seen: &mut BTreeSet<FunctionId>,
) -> Option<(VariableId, LockValue)> {
    if let Some(restoration) = state_lock_assignment(hir, stmt) {
        return Some(restoration);
    }

    let helper_id = simple_internal_call(stmt)?;
    if !seen.insert(helper_id) {
        return None;
    }
    let helper = hir.function(helper_id);
    let body = helper.modifiers.is_empty().then_some(helper.body?)?;
    let result = match body.stmts {
        [stmt] => state_lock_assignment(hir, stmt),
        _ => None,
    };
    seen.remove(&helper_id);
    result
}

fn simple_internal_call(stmt: &hir::Stmt<'_>) -> Option<FunctionId> {
    let StmtKind::Expr(expr) = stmt.kind else { return None };
    let ExprKind::Call(callee, args, opts) = &expr.peel_parens().kind else { return None };
    if opts.is_some() || args.exprs().next().is_some() {
        return None;
    }
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return None };
    unique(reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
        _ => None,
    }))
}

fn state_lock_assignment(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
) -> Option<(VariableId, LockValue)> {
    let StmtKind::Expr(expr) = stmt.kind else { return None };
    let ExprKind::Assign(lhs, None, rhs) = &expr.peel_parens().kind else { return None };
    let lock_var = direct_state_var(hir, lhs)?;
    Some((lock_var, constant_lock_value(hir, rhs)?))
}

fn direct_state_var(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return None };
    unique(reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Variable(var_id)) if hir.variable(*var_id).kind.is_state() => {
            Some(*var_id)
        }
        _ => None,
    }))
}

fn stmt_rejects_lock_value(
    hir: &hir::Hir<'_>,
    stmt: &hir::Stmt<'_>,
    lock_var: VariableId,
    entered: LockValue,
) -> bool {
    match stmt.kind {
        StmtKind::Expr(expr) => {
            let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
            is_require_or_assert(callee)
                && args.exprs().next().is_some_and(|condition| {
                    eval_lock_condition(hir, condition, lock_var, entered) == Some(false)
                })
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            let condition = eval_lock_condition(hir, condition, lock_var, entered);
            (condition == Some(true) && branch_always_exits(then_stmt))
                || (condition == Some(false) && else_stmt.is_some_and(branch_always_exits))
        }
        _ => false,
    }
}

fn constant_lock_value(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<LockValue> {
    eval_lock_value_inner(hir, expr, None, None, &mut BTreeSet::new())
}

fn eval_lock_condition(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    lock_var: VariableId,
    entered: LockValue,
) -> Option<bool> {
    match eval_lock_value_inner(hir, expr, Some(lock_var), Some(entered), &mut BTreeSet::new())? {
        LockValue::Bool(value) => Some(value),
        LockValue::Number(_) => None,
    }
}

fn eval_lock_value_inner(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    lock_var: Option<VariableId>,
    entered: Option<LockValue>,
    seen: &mut BTreeSet<VariableId>,
) -> Option<LockValue> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Bool(value) => Some(LockValue::Bool(value)),
            LitKind::Number(value) => Some(LockValue::Number(value)),
            _ => None,
        },
        ExprKind::Ident(reses) => {
            let var_id = unique(reses.iter().filter_map(|res| match res {
                Res::Item(ItemId::Variable(var_id)) => Some(*var_id),
                _ => None,
            }))?;
            if Some(var_id) == lock_var {
                return entered;
            }
            let var = hir.variable(var_id);
            if !var.is_constant() || !seen.insert(var_id) {
                return None;
            }
            let value = eval_lock_value_inner(hir, var.initializer?, lock_var, entered, seen);
            seen.remove(&var_id);
            value
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            let LockValue::Bool(value) =
                eval_lock_value_inner(hir, inner, lock_var, entered, seen)?
            else {
                return None;
            };
            Some(LockValue::Bool(!value))
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) => {
            let lhs = eval_lock_value_inner(hir, lhs, lock_var, entered, seen)?;
            let rhs = eval_lock_value_inner(hir, rhs, lock_var, entered, seen)?;
            Some(LockValue::Bool(if op.kind == BinOpKind::Eq { lhs == rhs } else { lhs != rhs }))
        }
        _ => None,
    }
}

fn call_uses_delegate_context(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    if let ExprKind::Member(_, member) = &callee.peel_parens().kind
        && matches!(member.name, kw::Callcode | kw::Delegatecall)
    {
        return true;
    }
    gcx.type_of_expr(callee.peel_parens().id).is_some_and(
        |ty| matches!(ty.kind, TyKind::Fn(function) if function.kind == TyFnKind::DelegateCall),
    )
}

fn self_address_paths(expr: &hir::Expr<'_>, state: &FlowState) -> PathAlternatives {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            if is_this(expr) {
                return [state.path_predicates.clone()].into_iter().collect();
            }
            let mut paths = PathAlternatives::new();
            for var_id in reses.iter().filter_map(|res| match res {
                Res::Item(ItemId::Variable(var_id)) => Some(var_id),
                _ => None,
            }) {
                if let Some(local_paths) = state.self_address_local_paths.get(var_id) {
                    paths.extend(constrain_paths(local_paths, &state.path_predicates));
                }
            }
            paths
        }
        ExprKind::Payable(inner) => self_address_paths(inner, state),
        ExprKind::Call(callee, args, opts)
            if opts.is_none() && is_address_type_expr(callee) && args.exprs().count() == 1 =>
        {
            args.exprs().next().map(|arg| self_address_paths(arg, state)).unwrap_or_default()
        }
        _ if is_this(expr) => [state.path_predicates.clone()].into_iter().collect(),
        _ => PathAlternatives::new(),
    }
}

fn is_uncapped_value_call(
    hir: &hir::Hir<'_>,
    callee: &hir::Expr<'_>,
    opts: Option<&hir::CallOptions<'_>>,
) -> bool {
    let Some(opts) = opts else { return false };
    let ExprKind::Member(_, member) = &callee.peel_parens().kind else { return false };
    if member.name != kw::Call {
        return false;
    }

    let mut value = None;
    let mut gas = None;
    for opt in opts.args {
        if opt.name.name == sym::value {
            value = Some(&opt.value);
        } else if opt.name.name == kw::Gas {
            gas = Some(&opt.value);
        }
    }

    value.is_some_and(|value| !is_zero_value(hir, value)) && gas.is_none_or(gas_option_forwards_all)
}

fn is_no_eth_reentrant_call<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    _args: &CallArgs<'hir>,
    opts: Option<&hir::CallOptions<'hir>>,
) -> bool {
    if call_sends_eth(hir, opts) {
        return false;
    }

    match &callee.peel_parens().kind {
        ExprKind::Member(receiver, _) if is_contract_receiver(gcx, receiver) => {
            external_call_can_reenter(gcx, callee)
        }
        ExprKind::Member(receiver, member)
            if is_address_like(gcx, hir, receiver)
                && matches!(
                    member.name,
                    kw::Call | kw::Callcode | kw::Delegatecall | kw::Staticcall
                ) =>
        {
            member.name != kw::Staticcall
        }
        ExprKind::Member(receiver, _) if is_super(receiver) => false,
        _ => external_call_can_reenter(gcx, callee),
    }
}

fn call_sends_eth(hir: &hir::Hir<'_>, opts: Option<&hir::CallOptions<'_>>) -> bool {
    opts.is_some_and(|opts| {
        opts.args.iter().any(|opt| opt.name.name == sym::value && !is_zero_value(hir, &opt.value))
    })
}

fn external_call_can_reenter<'hir>(gcx: Gcx<'hir>, callee: &'hir hir::Expr<'hir>) -> bool {
    let Some(ty) = gcx.type_of_expr(callee.peel_parens().id) else { return false };
    let TyKind::Fn(function) = ty.kind else { return false };
    is_externally_callable_fn_kind(function.kind)
        && !matches!(function.state_mutability, StateMutability::Pure | StateMutability::View)
}

const fn is_externally_callable_fn_kind(kind: TyFnKind) -> bool {
    matches!(kind, TyFnKind::External | TyFnKind::Declaration | TyFnKind::DelegateCall)
}

fn argument_for_parameter<'hir>(
    hir: &hir::Hir<'hir>,
    args: &CallArgs<'hir>,
    params: &[VariableId],
    index: usize,
) -> Option<&'hir hir::Expr<'hir>> {
    match args.kind {
        CallArgsKind::Unnamed(exprs) => exprs.get(index),
        CallArgsKind::Named(named_args) => {
            let name = hir.variable(*params.get(index)?).name?;
            named_args.iter().find(|arg| arg.name.name == name.name).map(|arg| &arg.value)
        }
    }
}

fn is_contract_receiver<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id)
        .is_some_and(|ty| matches!(ty.peel_refs().kind, TyKind::Contract(_)))
}

fn is_address_like<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) if is_address_type_expr(callee) => true,
        _ => expr_ty(gcx, hir, expr).is_some_and(type_is_address_like),
    }
}

fn is_address_type_expr(expr: &hir::Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: hir::TypeKind::Elementary(ElementaryType::Address(_)),
            ..
        })
    )
}

fn type_is_address_like(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}

fn expr_ty<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<Ty<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Array(_) | ExprKind::YulMember(..) => None,
        ExprKind::Call(callee, args, _) => {
            let callee_ty = expr_ty(gcx, hir, callee)?;
            match callee_ty.kind {
                TyKind::Fn(func) => fn_call_return_type(gcx, func.returns),
                TyKind::Type(to) => Some(explicit_cast_ty(gcx, to, args)),
                _ => None,
            }
        }
        ExprKind::Ident(reses) => {
            let res = unique(reses.iter().filter(|res| !matches!(res, Res::Err(_))).copied())?;
            match res {
                Res::Builtin(builtin) if matches!(builtin.name(), sym::this | sym::super_) => None,
                Res::Item(ItemId::Variable(var_id)) => Some(
                    gcx.type_of_res(res)
                        .with_loc_if_ref_opt(gcx, variable_data_location(hir, var_id)),
                ),
                _ => Some(gcx.type_of_res(res)),
            }
        }
        ExprKind::Index(lhs, index) => {
            let lhs_ty = expr_ty(gcx, hir, lhs)?;
            if let Some(index) = index
                && !expr_ty(gcx, hir, index)?.convert_implicit_to(gcx.types.uint(256), gcx)
            {
                return None;
            }
            index_ty(gcx, lhs_ty)
        }
        ExprKind::Lit(lit) => Some(match &lit.kind {
            LitKind::Str(StrKind::Hex, s, _) => {
                let size = TypeSize::try_new_fb_bytes(s.as_byte_str().len().min(32) as u8)?;
                gcx.types.fixed_bytes(size.bytes())
            }
            LitKind::Str(_, s, _) => gcx.mk_ty_string_literal(s.as_byte_str()),
            LitKind::Number(int) => gcx.mk_ty_int_literal(false, int.bit_len() as _)?,
            LitKind::Rational(_) | LitKind::Err(_) => return None,
            LitKind::Address(_) => gcx.types.address,
            LitKind::Bool(_) => gcx.types.bool,
        }),
        ExprKind::Member(base, member) => member_ty(gcx, hir, base, member.name),
        ExprKind::New(ty) => {
            let ty = gcx.type_of_hir_ty(ty);
            Some(gcx.mk_ty(TyKind::Type(ty)))
        }
        ExprKind::Payable(inner) => {
            let inner_ty = expr_ty(gcx, hir, inner)?;
            inner_ty
                .convert_explicit_to(gcx.types.address_payable, gcx)
                .then_some(gcx.types.address_payable)
        }
        ExprKind::Slice(lhs, ..) => {
            let lhs_ty = expr_ty(gcx, hir, lhs)?;
            lhs_ty.is_sliceable().then_some(gcx.mk_ty(TyKind::Slice(lhs_ty)))
        }
        ExprKind::Tuple(exprs) => {
            let tys = exprs
                .iter()
                .map(|expr| expr.and_then(|expr| expr_ty(gcx, hir, expr)))
                .collect::<Option<Vec<_>>>()?;
            Some(gcx.mk_ty_tuple(gcx.mk_tys(&tys)))
        }
        ExprKind::Ternary(_, true_expr, false_expr) => {
            let true_ty = expr_ty(gcx, hir, true_expr)?;
            let false_ty = expr_ty(gcx, hir, false_expr)?;
            common_ty(gcx, true_ty, false_ty)
        }
        ExprKind::Type(ty) | ExprKind::TypeCall(ty) => {
            let ty = gcx.type_of_hir_ty(ty);
            Some(gcx.mk_ty(TyKind::Type(ty)))
        }
        ExprKind::Unary(op, inner) => match op.kind {
            UnOpKind::Not => Some(gcx.types.bool),
            _ => expr_ty(gcx, hir, inner),
        },
        ExprKind::Binary(_, op, _) if binary_op_returns_bool(op.kind) => Some(gcx.types.bool),
        ExprKind::Assign(..) | ExprKind::Binary(..) | ExprKind::Delete(..) | ExprKind::Err(_) => {
            None
        }
    }
}

const fn binary_op_returns_bool(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
            | BinOpKind::And
            | BinOpKind::Or
    )
}

fn member_ty<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    base: &'hir hir::Expr<'hir>,
    member_name: solar::interface::Symbol,
) -> Option<Ty<'hir>> {
    if is_this(base) || is_super(base) {
        return None;
    }

    let base_ty = expr_ty(gcx, hir, base)?;
    unique(
        gcx.members_of(base_ty, base_item_source(hir, base), base_contract(hir, base))
            .filter(|member| member.name == member_name)
            .map(|member| member.ty),
    )
}

fn common_ty<'hir>(gcx: Gcx<'hir>, lhs: Ty<'hir>, rhs: Ty<'hir>) -> Option<Ty<'hir>> {
    if lhs.convert_implicit_to(rhs, gcx) {
        Some(rhs)
    } else {
        rhs.convert_implicit_to(lhs, gcx).then_some(lhs)
    }
}

fn fn_call_return_type<'hir>(gcx: Gcx<'hir>, returns: &'hir [Ty<'hir>]) -> Option<Ty<'hir>> {
    Some(match returns {
        [] => gcx.types.unit,
        [ret] => *ret,
        _ => gcx.mk_ty_tuple(returns),
    })
}

fn explicit_cast_ty<'hir>(gcx: Gcx<'hir>, to: Ty<'hir>, args: &'hir CallArgs<'hir>) -> Ty<'hir> {
    match args.exprs().next().and_then(|arg| expr_ty(gcx, &gcx.hir, arg)) {
        Some(from) => from.try_convert_explicit_to(to, gcx).unwrap_or(to),
        None => to,
    }
}

fn index_ty<'hir>(gcx: Gcx<'hir>, base_ty: Ty<'hir>) -> Option<Ty<'hir>> {
    let loc = indexed_base_data_location(base_ty);
    match base_ty.peel_refs().kind {
        TyKind::Mapping(_, value) => Some(value.with_loc_if_ref_opt(gcx, loc)),
        _ => base_ty.base_type(gcx),
    }
}

fn indexed_base_data_location(ty: Ty<'_>) -> Option<DataLocation> {
    ty.loc().or_else(|| matches!(ty.kind, TyKind::Mapping(..)).then_some(DataLocation::Storage))
}

fn base_item_source(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> hir::SourceId {
    referenced_item(expr)
        .map(|id| hir.item(id).source())
        .unwrap_or_else(|| hir.sources_enumerated().next().expect("HIR has a source").0)
}

fn base_contract(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    referenced_item(expr).and_then(|id| hir.item(id).contract())
}

fn referenced_item(expr: &hir::Expr<'_>) -> Option<ItemId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(id), ..]) => Some(*id),
        _ => None,
    }
}

fn variable_data_location(hir: &hir::Hir<'_>, var_id: VariableId) -> Option<DataLocation> {
    let var = hir.variable(var_id);
    var.data_location.or_else(|| var.kind.is_state().then_some(DataLocation::Storage))
}

fn is_this(expr: &hir::Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(res, Res::Builtin(builtin) if builtin.name() == sym::this)
            })
    )
}

fn is_super(expr: &hir::Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(res, Res::Builtin(builtin) if builtin.name() == sym::super_)
            })
    )
}

fn unique<T>(mut iter: impl Iterator<Item = T>) -> Option<T> {
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}

fn is_zero_value(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let mut seen = BTreeSet::new();
    is_zero_value_inner(hir, expr, &mut seen)
}

fn is_zero_value_inner(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    seen: &mut BTreeSet<VariableId>,
) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => matches!(lit.kind, LitKind::Number(value) if value.is_zero()),
        ExprKind::Ident(reses) => {
            let mut saw_variable = false;
            reses.iter().all(|res| match res {
                Res::Item(ItemId::Variable(var_id)) => {
                    saw_variable = true;
                    constant_var_is_zero(hir, *var_id, seen)
                }
                _ => false,
            }) && saw_variable
        }
        ExprKind::Call(callee, args, opts)
            if opts.is_none()
                && matches!(callee.peel_parens().kind, ExprKind::Type(_))
                && args.exprs().count() == 1 =>
        {
            args.exprs().next().is_some_and(|arg| is_zero_value_inner(hir, arg, seen))
        }
        _ => false,
    }
}

fn constant_var_is_zero(
    hir: &hir::Hir<'_>,
    var_id: VariableId,
    seen: &mut BTreeSet<VariableId>,
) -> bool {
    let var = hir.variable(var_id);
    if !var.is_constant() || !seen.insert(var_id) {
        return false;
    }
    var.initializer.is_some_and(|init| is_zero_value_inner(hir, init, seen))
}

fn gas_option_forwards_all(expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, opts) = &expr.peel_parens().kind else {
        return false;
    };
    if opts.is_some() || args.exprs().next().is_some() {
        return false;
    }
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|res| {
                matches!(res, Res::Builtin(builtin) if builtin.name() == sym::gasleft)
            })
    )
}

fn lhs_local_var(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> Option<VariableId> {
    if let ExprKind::Ident(reses) = &lhs.peel_parens().kind {
        for res in *reses {
            if let Res::Item(ItemId::Variable(var_id)) = res
                && !hir.variable(*var_id).kind.is_state()
            {
                return Some(*var_id);
            }
        }
    }
    None
}

fn local_write_lhs_vars(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    collect_local_write_lhs_vars(hir, expr, &mut vars);
    vars
}

fn collect_local_write_lhs_vars(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    vars: &mut Vec<VariableId>,
) {
    match &expr.kind {
        ExprKind::Ident(reses) => {
            for &res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && !hir.variable(var_id).kind.is_state()
                {
                    push_unique(vars, var_id);
                }
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_local_write_lhs_vars(hir, expr, vars);
            }
        }
        _ => {}
    }
}

fn forget_path_predicates(state: &mut FlowState, vars: impl IntoIterator<Item = VariableId>) {
    for var_id in vars {
        state.path_predicates.remove(&var_id);
        for paths in state.balance_local_paths.values_mut() {
            *paths = paths
                .iter()
                .map(|path| {
                    let mut path = path.clone();
                    path.remove(&var_id);
                    path
                })
                .collect();
        }
        for paths in state.self_address_local_paths.values_mut() {
            *paths = paths
                .iter()
                .map(|path| {
                    let mut path = path.clone();
                    path.remove(&var_id);
                    path
                })
                .collect();
        }
        for call in &mut state.pending_balance_calls {
            call.paths = call
                .paths
                .iter()
                .map(|path| {
                    let mut path = path.clone();
                    path.remove(&var_id);
                    path
                })
                .collect();
        }
    }
}

fn state_write_lhs_vars(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    collect_state_write_lhs_vars(hir, expr, &mut vars);
    vars
}

fn collect_state_write_lhs_vars(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    vars: &mut Vec<VariableId>,
) {
    match &expr.kind {
        ExprKind::Ident(reses) => {
            for &res in *reses {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && hir.variable(var_id).kind.is_state()
                {
                    push_unique(vars, var_id);
                }
            }
        }
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) => {
            collect_state_write_lhs_vars(hir, base, vars);
        }
        ExprKind::Member(base, _)
        | ExprKind::Payable(base)
        | ExprKind::Unary(_, base)
        | ExprKind::Delete(base) => collect_state_write_lhs_vars(hir, base, vars),
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().copied().flatten() {
                collect_state_write_lhs_vars(hir, expr, vars);
            }
        }
        _ => {}
    }
}

fn push_unique<T: Copy + Eq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn extend_unique<T: Copy + Eq>(items: &mut Vec<T>, values: impl IntoIterator<Item = T>) {
    for value in values {
        push_unique(items, value);
    }
}
