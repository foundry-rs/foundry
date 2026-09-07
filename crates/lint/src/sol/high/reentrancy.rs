use super::ReentrancyEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache, arg_for_param,
            branch_always_exits, cast_type, count_placeholders, expr_ty, for_each_child,
            for_each_lhs_var, function_ids, is_address_cast, is_address_like, is_builtin,
            is_inc_dec, is_require_or_assert, lhs_local_var, loop_update, state_lhs_vars,
            stmts_before_placeholder, tuple_elems, unique,
        },
    },
};
use alloy_primitives::U256;
use solar::{
    ast::{BinOpKind, FunctionKind, LitKind, StateMutability, UnOpKind, Visibility},
    interface::{Span, Symbol, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, CallArgs, CallOptions, ElementaryType, Expr, ExprKind, FunctionId, ItemId,
            LoopSource, Res, Stmt, StmtKind, VariableId, Visit,
        },
        ty::{TyFnKind, TyKind},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ops::ControlFlow,
};

/// Gas stipend forwarded by `transfer`/`send`; a call capped at or below it cannot reenter.
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

impl<'gcx> LateLintPass<'gcx> for ReentrancyEth {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        let Some(body) = func.body.filter(|_| is_entry_point(func)) else { return };
        let mut analyzer = Analyzer::new(ctx, gcx, func);
        if analyzer.has_enabled_lints() {
            analyzer.analyze_callable(func, body, &mut FlowState::default());
        }
    }
}

/// Non-view functions an external caller can invoke: public/external functions, `fallback` and
/// `receive`.
fn is_entry_point(func: &hir::Function<'_>) -> bool {
    !is_view_or_pure(func.state_mutability)
        && !func.is_constructor()
        && (func.is_special()
            || (func.kind.is_function()
                && matches!(func.visibility, Visibility::Public | Visibility::External)))
}

const fn is_view_or_pure(mutability: StateMutability) -> bool {
    matches!(mutability, StateMutability::Pure | StateMutability::View)
}

type PathPredicates = BTreeMap<PathPredicate, bool>;
type PathAlternatives = BTreeSet<PathPredicates>;

/// Facts that hold along one execution path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct FlowState {
    /// State variables read so far.
    state_reads: BTreeSet<VariableId>,
    /// Reentrant calls made after a state read, with the state read before each, awaiting a
    /// later write to that state.
    pending_calls: BTreeMap<(Span, ReentrantCallKind), BTreeSet<VariableId>>,
    /// Internal functions a function-typed local may point to.
    internal_function_targets: BTreeMap<VariableId, BTreeSet<FunctionId>>,
    /// Locals holding `address(this)`, with the path predicates under which they do.
    self_address_local_paths: BTreeMap<VariableId, PathAlternatives>,
    /// Locals derived from `address(this).balance`, with the predicates under which they are.
    balance_local_paths: BTreeMap<VariableId, PathAlternatives>,
    /// Supported arithmetic forms of local values; an empty set means unknown.
    balance_local_forms: BTreeMap<VariableId, BTreeSet<BalanceForm>>,
    /// Balance occurrences retained through arbitrary arithmetic on local values.
    balance_local_dependencies: BTreeMap<VariableId, BTreeSet<BalanceForm>>,
    /// Locals holding a comparison against a balance made stale by the given calls.
    balance_comparison_locals: BTreeMap<VariableId, BTreeSet<Span>>,
    /// External calls after which cached balance locals are stale.
    pending_balance_calls: BTreeMap<Span, PathAlternatives>,
    /// Reentrancy-guard locks written or bypassed while active.
    invalidated_balance_guards: BTreeSet<VariableId>,
    /// Boolean facts known to hold on this path.
    path_predicates: PathPredicates,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum PathPredicate {
    Boolean(VariableId),
    Equality(Operand, Operand),
}

impl PathPredicate {
    fn mentions(self, f: impl Fn(VariableId) -> bool) -> bool {
        match self {
            Self::Boolean(var_id) => f(var_id),
            Self::Equality(lhs, rhs) => {
                [lhs, rhs].into_iter().any(|op| matches!(op, Operand::Variable(v) if f(v)))
            }
        }
    }
}

/// A local variable or constant appearing in a predicate or lock expression.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum Operand {
    Variable(VariableId),
    Number(U256),
    Boolean(bool),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ReentrantCallKind {
    Eth,
    NoEth,
}

/// One signed balance read, retaining the calls across which it was cached.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BalanceTerm {
    negative: bool,
    stale_calls: BTreeSet<Span>,
}

/// A supported additive source pattern on one path. An empty term list is balance-independent.
/// At most two balance terms are retained: a stale/current comparison needs one of each. More
/// terms are rejected rather than cancelled, since Solidity arithmetic may wrap or revert.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BalanceForm {
    terms: Vec<BalanceTerm>,
    path: PathPredicates,
}

impl BalanceForm {
    fn combine(&self, rhs: &Self, subtract: bool) -> Option<Self> {
        if self.terms.len() + rhs.terms.len() > 2 || !paths_compatible(&self.path, &rhs.path) {
            return None;
        }
        let mut terms = self.terms.clone();
        terms.extend(rhs.terms.iter().cloned().map(|mut term| {
            term.negative ^= subtract;
            term
        }));
        Some(Self {
            terms,
            path: self.path.iter().chain(&rhs.path).map(|(p, v)| (*p, *v)).collect(),
        })
    }

    fn constrained(&self, active: &PathPredicates) -> Option<Self> {
        paths_compatible(&self.path, active).then(|| Self {
            terms: self.terms.clone(),
            path: self.path.iter().chain(active).map(|(p, v)| (*p, *v)).collect(),
        })
    }
}

/// Balance-related facts about a value flowing into a local, parameter or return slot.
#[derive(Clone, Debug, Default)]
struct BalanceValue {
    /// Supported arithmetic forms, separate from occurrence-based dependencies.
    forms: BTreeSet<BalanceForm>,
    /// Individual balance occurrences, without arithmetic signs or identities.
    dependencies: BTreeSet<BalanceForm>,
    /// Predicates under which the value derives from `address(this).balance`.
    balance_paths: PathAlternatives,
    /// Predicates under which the value is `address(this)`.
    self_address_paths: PathAlternatives,
    /// Calls whose stale balance the value compares against.
    stale_comparisons: BTreeSet<Span>,
}

impl BalanceValue {
    fn merge(&mut self, other: Self) {
        self.forms.extend(other.forms);
        self.dependencies.extend(other.dependencies);
        self.balance_paths.extend(other.balance_paths);
        self.self_address_paths.extend(other.self_address_paths);
        self.stale_comparisons.extend(other.stale_comparisons);
    }

    /// The same value seen from a path on which `predicates` hold.
    fn constrained(&self, predicates: &PathPredicates) -> Self {
        Self {
            forms: self.forms.iter().filter_map(|form| form.constrained(predicates)).collect(),
            dependencies: self
                .dependencies
                .iter()
                .filter_map(|form| form.constrained(predicates))
                .collect(),
            balance_paths: constrain_paths(&self.balance_paths, predicates),
            self_address_paths: constrain_paths(&self.self_address_paths, predicates),
            ..self.clone()
        }
    }
}

impl FlowState {
    fn push_call(&mut self, span: Span, kind: ReentrantCallKind) {
        if !self.state_reads.is_empty() {
            self.pending_calls.entry((span, kind)).or_default().extend(&self.state_reads);
        }
    }

    fn push_balance_call(&mut self, span: Span) {
        if self
            .balance_local_paths
            .values()
            .any(|paths| paths.iter().any(|path| paths_compatible(path, &self.path_predicates)))
        {
            self.pending_balance_calls
                .entry(span)
                .or_default()
                .insert(self.path_predicates.clone());
            for forms in self
                .balance_local_forms
                .values_mut()
                .chain(self.balance_local_dependencies.values_mut())
            {
                *forms = std::mem::take(forms)
                    .into_iter()
                    .map(|mut form| {
                        if paths_compatible(&form.path, &self.path_predicates) {
                            for term in &mut form.terms {
                                term.stale_calls.insert(span);
                            }
                        }
                        form
                    })
                    .collect();
            }
        }
    }

    fn merge(&mut self, other: &Self) {
        self.state_reads.extend(&other.state_reads);
        merge_maps(&mut self.pending_calls, &other.pending_calls);
        self.merge_balance(other);
    }

    fn merge_balance(&mut self, other: &Self) {
        merge_maps(&mut self.internal_function_targets, &other.internal_function_targets);
        merge_maps(&mut self.self_address_local_paths, &other.self_address_local_paths);
        merge_maps(&mut self.balance_local_paths, &other.balance_local_paths);
        merge_maps(&mut self.balance_local_forms, &other.balance_local_forms);
        merge_maps(&mut self.balance_local_dependencies, &other.balance_local_dependencies);
        merge_maps(&mut self.balance_comparison_locals, &other.balance_comparison_locals);
        self.invalidated_balance_guards.extend(&other.invalidated_balance_guards);
        merge_maps(&mut self.pending_balance_calls, &other.pending_balance_calls);
    }

    fn balance_only(&self) -> Self {
        Self { state_reads: BTreeSet::new(), pending_calls: BTreeMap::new(), ..self.clone() }
    }

    /// Records that `predicate` holds; returns `false` if that contradicts a known fact.
    fn constrain_path(&mut self, (predicate, value): (PathPredicate, bool)) -> bool {
        match self.path_predicates.get(&predicate) {
            Some(existing) => *existing == value,
            None => {
                self.path_predicates.insert(predicate, value);
                for paths in self.self_address_local_paths.values_mut() {
                    *paths = constrain_paths(paths, &self.path_predicates);
                }
                true
            }
        }
    }
}

fn merge_maps<K: Copy + Ord, V: Clone + Ord>(
    into: &mut BTreeMap<K, BTreeSet<V>>,
    from: &BTreeMap<K, BTreeSet<V>>,
) {
    for (key, values) in from {
        into.entry(*key).or_default().extend(values.iter().cloned());
    }
}

/// Replaces `state` with the union of the reachable `branches`, keeping only the path predicates
/// all of them agree on. Returns whether any branch was reachable.
fn join_branches(
    state: &mut FlowState,
    branches: impl IntoIterator<Item = Option<FlowState>>,
) -> bool {
    *state = FlowState::default();
    let mut predicates = None;
    for branch in branches.into_iter().flatten() {
        state.merge(&branch);
        predicates = Some(match predicates {
            Some(common) => common_path_predicates(&common, &branch.path_predicates),
            None => branch.path_predicates,
        });
    }
    let reachable = predicates.is_some();
    state.path_predicates = predicates.unwrap_or_default();
    reachable
}

struct Analyzer<'ctx, 's, 'c, 'gcx> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'gcx>,
    emitted: HashSet<Span>,
    emitted_balance: HashSet<Span>,
    call_stack: Vec<FunctionId>,
    inline_cache: HelperAnalysisCache<InlineCallKey, (FlowState, Vec<BalanceValue>)>,
    /// First function on the given call stack reachable again from a callee, per callee.
    recursive_cuts: HashMap<(FunctionId, BTreeSet<FunctionId>), Option<FunctionId>>,
    direct_internal_calls: HashMap<FunctionId, Vec<FunctionId>>,
    reentrancy_eth_enabled: bool,
    reentrancy_no_eth_enabled: bool,
    reentrancy_balance_enabled: bool,
    /// Set while re-running code only to refine the balance analysis.
    balance_only_analysis: bool,
    /// Balance facts about the return values of each analysed internal call site.
    call_balance_values: HashMap<Span, Vec<BalanceValue>>,
    /// Return-value accumulators of the internal calls currently being inlined.
    return_collectors: Vec<(FunctionId, Vec<BalanceValue>)>,
    /// Reentrancy-guard locks held by the modifiers wrapping the code being analysed.
    active_balance_guards: Vec<VariableId>,
    /// The lock guarding every entry point of each deployable contract, if any.
    balance_reentry_lock: Option<VariableId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    /// First active function that can cut recursion from this callee.
    recursive_cut: Option<FunctionId>,
    balance_only: bool,
    active_balance_guards: Vec<VariableId>,
    parameter_predicates: Vec<Option<(PathPredicate, bool)>>,
    state: FlowState,
}

/// What to analyse when a modifier's `_` is reached: the remaining modifiers, the function body
/// and the reentrancy-guard lock the current modifier holds.
type ModifierContinuation<'gcx> =
    (&'gcx [hir::Modifier<'gcx>], usize, hir::Block<'gcx>, Option<VariableId>);

impl<'ctx, 's, 'c, 'gcx> Analyzer<'ctx, 's, 'c, 'gcx> {
    fn new(
        ctx: &'ctx LintContext<'s, 'c>,
        gcx: Gcx<'gcx>,
        entry: &'gcx hir::Function<'gcx>,
    ) -> Self {
        let reentrancy_balance_enabled = ctx.is_lint_enabled(REENTRANCY_BALANCE.id);
        Self {
            ctx,
            gcx,
            emitted: HashSet::new(),
            emitted_balance: HashSet::new(),
            call_stack: Vec::new(),
            inline_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            recursive_cuts: HashMap::new(),
            direct_internal_calls: HashMap::new(),
            reentrancy_eth_enabled: ctx.is_lint_enabled(REENTRANCY_ETH.id),
            reentrancy_no_eth_enabled: ctx.is_lint_enabled(REENTRANCY_NO_ETH.id),
            reentrancy_balance_enabled,
            balance_only_analysis: false,
            call_balance_values: HashMap::new(),
            return_collectors: Vec::new(),
            active_balance_guards: Vec::new(),
            balance_reentry_lock: reentrancy_balance_enabled
                .then(|| balance_reentry_lock(gcx, entry))
                .flatten(),
        }
    }

    const fn has_enabled_lints(&self) -> bool {
        self.reentrancy_eth_enabled
            || self.reentrancy_no_eth_enabled
            || self.reentrancy_balance_enabled
    }

    /// Analyses `func` (modifiers first, then `body`); returns whether it can fall through.
    fn analyze_callable(
        &mut self,
        func: &'gcx hir::Function<'gcx>,
        body: hir::Block<'gcx>,
        state: &mut FlowState,
    ) -> bool {
        self.analyze_modifier_chain(func.modifiers, 0, body, state)
    }

    fn analyze_modifier_chain(
        &mut self,
        modifiers: &'gcx [hir::Modifier<'gcx>],
        index: usize,
        body: hir::Block<'gcx>,
        state: &mut FlowState,
    ) -> bool {
        let Some(modifier) = modifiers.get(index) else {
            return self.analyze_block(body, None, state);
        };
        for arg in modifier.args.exprs() {
            self.analyze_expr(arg, state);
        }
        let Some((modifier_id, modifier_func, modifier_body)) = modifier
            .id
            .as_function()
            .filter(|id| !self.call_stack.contains(id))
            .map(|id| (id, self.gcx.hir.function(id)))
            .and_then(|(id, func)| Some((id, func, func.body?)))
        else {
            return self.analyze_modifier_chain(modifiers, index + 1, body, state);
        };

        self.seed_balance_parameters(modifier_func, &modifier.args, state);
        self.call_stack.push(modifier_id);
        let balance_guard = self
            .reentrancy_balance_enabled
            .then(|| standard_reentrancy_guard_lock(&self.gcx.hir, modifier_func))
            .flatten();
        let continuation = Some((modifiers, index + 1, body, balance_guard));
        let falls_through = self.analyze_block(modifier_body, continuation, state);
        self.call_stack.pop();
        self.clear_function_locals(modifier_id, state);
        falls_through
    }

    /// Analyzes one loop iteration: the body, then the `for` update if the body completes.
    fn analyze_iteration(
        &mut self,
        block: hir::Block<'gcx>,
        source: LoopSource<'gcx>,
        placeholder: Option<ModifierContinuation<'gcx>>,
        state: &mut FlowState,
    ) -> bool {
        self.analyze_block(block, placeholder, state)
            && loop_update(source)
                .is_none_or(|update| self.analyze_stmt(update, placeholder, state))
    }

    fn analyze_block(
        &mut self,
        block: hir::Block<'gcx>,
        placeholder: Option<ModifierContinuation<'gcx>>,
        state: &mut FlowState,
    ) -> bool {
        block.stmts.iter().all(|stmt| self.analyze_stmt(stmt, placeholder, state))
    }

    /// Analyses `stmt`; returns whether control can continue past it.
    fn analyze_stmt(
        &mut self,
        stmt: &'gcx Stmt<'gcx>,
        placeholder: Option<ModifierContinuation<'gcx>>,
        state: &mut FlowState,
    ) -> bool {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.gcx.hir.variable(var_id).initializer {
                    self.analyze_expr(init, state);
                    self.update_internal_function_target(state, var_id, init);
                    if self.reentrancy_balance_enabled {
                        self.bind_locals(state, &[Some(var_id)], init, None);
                    }
                } else if self.reentrancy_balance_enabled {
                    self.set_self_address_paths(state, var_id, PathAlternatives::new());
                }
                true
            }
            StmtKind::DeclMulti(vars, expr) => {
                self.analyze_expr(expr, state);
                if self.reentrancy_balance_enabled {
                    self.bind_locals(state, vars, expr, None);
                }
                true
            }
            StmtKind::Expr(expr) | StmtKind::Emit(expr) => {
                self.analyze_expr(expr, state);
                true
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, state);
                false
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, placeholder, state)
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
            StmtKind::Loop(block, source) => {
                let before_loop = state.clone();
                let mut body_state = state.clone();
                self.analyze_iteration(block, source, placeholder, &mut body_state);
                // One bounded second iteration exposes loop-carried balance checks while leaving
                // the established ETH and no-ETH analysis unchanged.
                let second_iteration = self.reentrancy_balance_enabled.then(|| {
                    let mut second = body_state.balance_only();
                    self.analyze_with_only_balance(|this| {
                        this.analyze_iteration(block, source, placeholder, &mut second)
                    });
                    second
                });
                join_branches(state, [Some(before_loop), Some(body_state)]);
                if let Some(second) = second_iteration {
                    state.path_predicates =
                        common_path_predicates(&state.path_predicates, &second.path_predicates);
                    state.merge_balance(&second);
                }
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
                let (mut then_state, mut else_state) = (state.clone(), state.clone());
                let (then_reachable, else_reachable) =
                    self.split_on(cond, &mut then_state, &mut else_state);
                let then_falls_through =
                    then_reachable && self.analyze_stmt(then_stmt, placeholder, &mut then_state);
                let else_falls_through = else_reachable
                    && else_stmt.is_none_or(|e| self.analyze_stmt(e, placeholder, &mut else_state));
                join_branches(
                    state,
                    [
                        then_falls_through.then_some(then_state),
                        else_falls_through.then_some(else_state),
                    ],
                )
            }
            StmtKind::Try(try_stmt) => {
                self.analyze_expr(&try_stmt.expr, state);
                let clauses = try_stmt
                    .clauses
                    .iter()
                    .map(|clause| {
                        let mut clause_state = state.clone();
                        self.analyze_block(clause.block, placeholder, &mut clause_state)
                            .then_some(clause_state)
                    })
                    .collect::<Vec<_>>();
                join_branches(state, clauses)
            }
            StmtKind::Placeholder => {
                let Some((modifiers, index, body, balance_guard)) = placeholder else {
                    return true;
                };
                if let Some(lock_var) = balance_guard {
                    state.invalidated_balance_guards.remove(&lock_var);
                    self.active_balance_guards.push(lock_var);
                }
                let falls_through = self.analyze_modifier_chain(modifiers, index, body, state);
                if balance_guard.is_some() {
                    self.active_balance_guards.pop();
                }
                falls_through
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) => {
                state.invalidated_balance_guards.extend(&self.active_balance_guards);
                state.internal_function_targets.clear();
                state.self_address_local_paths.clear();
                true
            }
            StmtKind::Err(_) => true,
        }
    }

    /// Constrains the branch states of a conditional on `cond`; returns whether each is reachable.
    fn split_on(
        &self,
        cond: &'gcx Expr<'gcx>,
        then_state: &mut FlowState,
        else_state: &mut FlowState,
    ) -> (bool, bool) {
        let predicate =
            self.reentrancy_balance_enabled.then(|| path_predicate(&self.gcx.hir, cond)).flatten();
        let Some((predicate, value)) = predicate else { return (true, true) };
        (
            then_state.constrain_path((predicate, value)),
            else_state.constrain_path((predicate, !value)),
        )
    }

    fn analyze_expr(&mut self, expr: &'gcx Expr<'gcx>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                if op.is_some() {
                    self.analyze_expr(lhs, state);
                }
                self.analyze_expr(rhs, state);
                self.analyze_lhs_indices(lhs, state);
                self.record_write(lhs, state);
                if let Some(var_id) = lhs_local_var(&self.gcx.hir, lhs) {
                    if op.is_none() {
                        self.update_internal_function_target(state, var_id, rhs);
                    } else {
                        state.internal_function_targets.remove(&var_id);
                    }
                }
                if self.reentrancy_balance_enabled {
                    let targets = match tuple_elems(lhs) {
                        Some(elems) => elems
                            .iter()
                            .map(|e| e.and_then(|e| lhs_local_var(&self.gcx.hir, e)))
                            .collect(),
                        None => vec![lhs_local_var(&self.gcx.hir, lhs)],
                    };
                    self.bind_locals(state, &targets, rhs, op.map(|op| op.kind));
                }
            }
            ExprKind::Delete(inner) => {
                self.analyze_lhs_indices(inner, state);
                self.record_write(inner, state);
                if let Some(var_id) = lhs_local_var(&self.gcx.hir, inner) {
                    state.internal_function_targets.remove(&var_id);
                    if self.reentrancy_balance_enabled {
                        self.clear_local(state, var_id);
                    }
                }
            }
            ExprKind::Unary(op, inner) => {
                self.analyze_expr(inner, state);
                if is_inc_dec(op.kind) {
                    self.record_write(inner, state);
                    if self.reentrancy_balance_enabled
                        && let Some(var_id) = lhs_local_var(&self.gcx.hir, inner)
                    {
                        self.set_self_address_paths(state, var_id, PathAlternatives::new());
                    }
                }
            }
            ExprKind::Call(callee, args, opts) => {
                let mut operands = vec![*callee];
                operands.extend(opts.iter().flat_map(|opts| opts.args).map(|opt| &opt.value));
                operands.extend(args.exprs());

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

                for func_id in self.internal_callees(callee, state) {
                    let returns = self.analyze_internal_call(func_id, args, state);
                    self.merge_call_balance_values(expr.span, returns);
                }
                if !state.state_reads.is_empty()
                    && let Some(kind) = self.reentrant_call_kind(callee, *opts)
                {
                    state.push_call(expr.span, kind);
                }
                if self.reentrancy_balance_enabled
                    && call_options_allow_reentrancy(&self.gcx.hir, *opts)
                    && callee_can_reenter(self.gcx, callee)
                    && !self.balance_guard_blocks_call(state, callee)
                {
                    state.push_balance_call(expr.span);
                }
                if call_uses_delegate_context(self.gcx, callee) {
                    state.invalidated_balance_guards.extend(&self.active_balance_guards);
                }
            }
            ExprKind::Binary(lhs, op, rhs)
                if self.reentrancy_balance_enabled
                    && matches!(op.kind, BinOpKind::And | BinOpKind::Or) =>
            {
                self.analyze_expr(lhs, state);
                let rhs_outcome = op.kind == BinOpKind::And;
                let (mut short_state, mut rhs_state) = (state.clone(), state.clone());
                let short_reachable =
                    constrain_boolean_outcome(&self.gcx.hir, lhs, !rhs_outcome, &mut short_state);
                let rhs_reachable =
                    constrain_boolean_outcome(&self.gcx.hir, lhs, rhs_outcome, &mut rhs_state);
                if rhs_reachable {
                    self.analyze_expr(rhs, &mut rhs_state);
                }
                join_branches(
                    state,
                    [short_reachable.then_some(short_state), rhs_reachable.then_some(rhs_state)],
                );
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                self.analyze_expr(cond, state);
                let (mut true_state, mut false_state) = (state.clone(), state.clone());
                let (true_reachable, false_reachable) =
                    self.split_on(cond, &mut true_state, &mut false_state);
                if true_reachable {
                    self.analyze_expr(true_expr, &mut true_state);
                }
                if false_reachable {
                    self.analyze_expr(false_expr, &mut false_state);
                }
                join_branches(
                    state,
                    [true_reachable.then_some(true_state), false_reachable.then_some(false_state)],
                );
            }
            ExprKind::Ident(reses) => state.state_reads.extend(
                reses
                    .iter()
                    .filter_map(Res::as_variable)
                    .filter(|v| self.gcx.hir.variable(*v).kind.is_state()),
            ),
            _ => for_each_child(expr, &mut |child| self.analyze_expr(child, state)),
        }
    }

    /// Evaluates the index and slice operands of an lvalue; the written root itself is no read.
    fn analyze_lhs_indices(&mut self, expr: &'gcx Expr<'gcx>, state: &mut FlowState) {
        match &expr.kind {
            ExprKind::Index(base, index) => {
                self.analyze_lhs_indices(base, state);
                if let Some(index) = index {
                    self.analyze_expr(index, state);
                }
            }
            ExprKind::Slice(base, start, end) => {
                self.analyze_lhs_indices(base, state);
                for bound in [start, end].into_iter().flatten() {
                    self.analyze_expr(bound, state);
                }
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs_indices(base, state);
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().flatten() {
                    self.analyze_lhs_indices(expr, state);
                }
            }
            _ => {}
        }
    }

    /// Handles a write to `lhs`: reports pending reentrant calls that read the written state,
    /// invalidates written guard locks and forgets path facts about written locals.
    fn record_write(&mut self, lhs: &'gcx Expr<'gcx>, state: &mut FlowState) {
        let written = state_lhs_vars(&self.gcx.hir, lhs);
        self.emit_pending_calls(state, &written);
        state
            .invalidated_balance_guards
            .extend(written.iter().filter(|v| self.active_balance_guards.contains(v)));
        for_each_lhs_var(lhs, &mut |var_id| {
            if !self.gcx.hir.variable(var_id).kind.is_state() {
                forget_path_predicates(state, var_id);
            }
        });
    }

    fn analyze_internal_call(
        &mut self,
        func_id: FunctionId,
        args: &CallArgs<'gcx>,
        state: &mut FlowState,
    ) -> Vec<BalanceValue> {
        let func = self.gcx.hir.function(func_id);
        let Some(body) = func.body.filter(|_| !self.call_stack.contains(&func_id)) else {
            return Vec::new();
        };

        self.seed_balance_parameters(func, args, state);
        let parameter_predicates = if self.reentrancy_balance_enabled {
            func.parameters
                .iter()
                .map(|&param| {
                    arg_for_param(&self.gcx.hir, func, param, args)
                        .and_then(|arg| path_predicate(&self.gcx.hir, arg))
                })
                .collect()
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
            self.clear_function_locals(func_id, state);
            return Vec::new();
        }
        if let Some((cached, returns)) = self.inline_cache.get(&key).cloned() {
            *state = if self.balance_only_analysis { cached.balance_only() } else { cached };
            return returns;
        }

        self.inline_cache.start(key.clone());
        if self.reentrancy_balance_enabled {
            let slots = vec![BalanceValue::default(); func.returns.len()];
            self.return_collectors.push((func_id, slots));
        }
        // Call-site results belong to this invocation; only its evaluation alternatives merge.
        let caller_balance_values = std::mem::take(&mut self.call_balance_values);
        self.call_stack.push(func_id);
        let mut after = state.clone();
        let falls_through = self.analyze_callable(func, body, &mut after);
        self.call_stack.pop();

        let mut returns = Vec::new();
        if self.reentrancy_balance_enabled {
            if falls_through {
                self.record_return(None, &after);
            }
            returns = self.return_collectors.pop().expect("return collector is active").1;
            remap_return_paths(
                &self.gcx.hir,
                func_id,
                func.parameters,
                &parameter_predicates,
                &mut returns,
            );
        }
        self.call_balance_values = caller_balance_values;
        self.clear_function_locals(func_id, &mut after);
        if self.balance_only_analysis {
            after = after.balance_only();
        }

        self.inline_cache.finish(key, (after.clone(), returns.clone()));
        *state = after;
        returns
    }

    /// Internal functions a call through `callee` may reach, following function-typed locals.
    fn internal_callees(
        &self,
        callee: &'gcx Expr<'gcx>,
        state: &FlowState,
    ) -> BTreeSet<FunctionId> {
        if let Some(targets) = lhs_local_var(&self.gcx.hir, callee)
            .and_then(|v| state.internal_function_targets.get(&v))
        {
            return targets.clone();
        }
        static_internal_callee(self.gcx, callee).into_iter().collect()
    }

    fn update_internal_function_target(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        value: &'gcx Expr<'gcx>,
    ) {
        let targets = self.internal_callees(value, state);
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
            stored.merge(value);
        }
    }

    fn analyze_with_only_balance<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = (
            self.reentrancy_eth_enabled,
            self.reentrancy_no_eth_enabled,
            self.balance_only_analysis,
        );
        (self.reentrancy_eth_enabled, self.reentrancy_no_eth_enabled, self.balance_only_analysis) =
            (false, false, true);
        let result = f(self);
        (self.reentrancy_eth_enabled, self.reentrancy_no_eth_enabled, self.balance_only_analysis) =
            saved;
        result
    }

    fn first_recursive_cut(&mut self, func_id: FunctionId) -> Option<FunctionId> {
        if self.call_stack.is_empty() {
            return None;
        }
        let key = (func_id, self.call_stack.iter().copied().collect::<BTreeSet<_>>());
        if let Some(cut) = self.recursive_cuts.get(&key) {
            return *cut;
        }
        let cut = self.first_recursive_cut_from(func_id, &key.1, &mut HashSet::new());
        self.recursive_cuts.insert(key, cut);
        cut
    }

    /// Depth-first search from `func_id` for the first callee that is currently `active`.
    fn first_recursive_cut_from(
        &mut self,
        func_id: FunctionId,
        active: &BTreeSet<FunctionId>,
        seen: &mut HashSet<FunctionId>,
    ) -> Option<FunctionId> {
        if !seen.insert(func_id) {
            return None;
        }
        self.direct_internal_calls(func_id).into_iter().find_map(|callee| {
            if active.contains(&callee) {
                Some(callee)
            } else {
                self.first_recursive_cut_from(callee, active, seen)
            }
        })
    }

    /// Internal functions and modifiers `func_id` invokes directly.
    fn direct_internal_calls(&mut self, func_id: FunctionId) -> Vec<FunctionId> {
        if let Some(calls) = self.direct_internal_calls.get(&func_id) {
            return calls.clone();
        }
        let mut collector = CallCollector { gcx: self.gcx, calls: BTreeSet::new() };
        let _ = collector.visit_function(self.gcx.hir.function(func_id));
        let calls = collector.calls.into_iter().collect::<Vec<_>>();
        self.direct_internal_calls.insert(func_id, calls.clone());
        calls
    }

    fn emit_pending_calls(&mut self, state: &FlowState, written_vars: &[VariableId]) {
        for (&(span, kind), reads) in &state.pending_calls {
            let Some(var_id) = written_vars.iter().find(|v| reads.contains(v)) else { continue };
            if !self.emitted.insert(span) {
                continue;
            }
            let (lint, what) = match kind {
                ReentrantCallKind::Eth => (&REENTRANCY_ETH, "uncapped ETH transfer"),
                ReentrantCallKind::NoEth => (&REENTRANCY_NO_ETH, "external call"),
            };
            let name = self
                .gcx
                .hir
                .variable(*var_id)
                .name
                .map_or_else(|| "state".to_string(), |name| name.to_string());
            let msg = format!("{what} can be reentered before `{name}` is updated");
            self.ctx.emit_with_msg(lint, span, msg);
        }
    }

    /// Reports pending balance calls whose stale balance `guard` compares against.
    fn emit_balance_calls(&mut self, guard: &'gcx Expr<'gcx>, state: &FlowState) {
        for (&span, call) in &state.pending_balance_calls {
            if !self.emitted_balance.contains(&span)
                && self.guard_has_stale_balance_comparison(guard, span, call, state)
            {
                self.ctx.emit(&REENTRANCY_BALANCE, span);
                self.emitted_balance.insert(span);
            }
        }
    }

    /// True if `expr` compares a balance read before the pending `call` against one read after.
    fn guard_has_stale_balance_comparison(
        &self,
        expr: &'gcx Expr<'gcx>,
        span: Span,
        call: &PathAlternatives,
        state: &FlowState,
    ) -> bool {
        let expr = expr.peel_parens();
        let recurse = |e, s| self.guard_has_stale_balance_comparison(e, span, call, s);
        match &expr.kind {
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                if recurse(lhs, state) {
                    return true;
                }
                let mut rhs_state = state.clone();
                constrain_boolean_outcome(
                    &self.gcx.hir,
                    lhs,
                    op.kind == BinOpKind::And,
                    &mut rhs_state,
                ) && recurse(rhs, &rhs_state)
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
                (is_comparison && {
                    let lhs_dependencies = self.balance_operand_dependencies(lhs, state);
                    let rhs_dependencies = self.balance_operand_dependencies(rhs, state);
                    let direct = lhs_dependencies.iter().any(|lhs| {
                        rhs_dependencies.iter().any(|rhs| {
                            paths_compatible(&lhs.path, &rhs.path)
                                && call.iter().any(|path| {
                                    paths_compatible(path, &lhs.path)
                                        && paths_compatible(path, &rhs.path)
                                })
                                && lhs.terms.iter().any(|a| {
                                    rhs.terms.iter().any(|b| {
                                        a.stale_calls.contains(&span)
                                            != b.stale_calls.contains(&span)
                                    })
                                })
                        })
                    });
                    let lhs = self.balance_forms(lhs, state);
                    let rhs = self.balance_forms(rhs, state);
                    direct
                        || lhs.iter().any(|lhs| {
                            rhs.iter().filter_map(|rhs| lhs.combine(rhs, true)).any(|form| {
                                let [a, b] = form.terms.as_slice() else { return false };
                                a.negative != b.negative
                                    && a.stale_calls.contains(&span)
                                        != b.stale_calls.contains(&span)
                                    && call.iter().any(|path| paths_compatible(path, &form.path))
                            })
                        })
                }) || recurse(lhs, state)
                    || recurse(rhs, state)
            }
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => recurse(inner, state),
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                [*cond, *true_expr, *false_expr].into_iter().any(|e| recurse(e, state))
            }
            ExprKind::Call(..) => match cast_args(expr) {
                Some(args) => args.exprs().any(|arg| recurse(arg, state)),
                None => self.call_balance_values.get(&expr.span).is_some_and(|values| {
                    values.iter().any(|value| value.stale_comparisons.contains(&span))
                }),
            },
            ExprKind::Ident(reses) => reses.iter().filter_map(Res::as_variable).any(|var_id| {
                state
                    .balance_comparison_locals
                    .get(&var_id)
                    .is_some_and(|calls| calls.contains(&span))
            }),
            _ => false,
        }
    }

    /// Binds the components of `rhs` to the local `targets` they are assigned to.
    fn bind_locals(
        &mut self,
        state: &mut FlowState,
        targets: &[Option<VariableId>],
        rhs: &'gcx Expr<'gcx>,
        op: Option<BinOpKind>,
    ) {
        if targets.iter().all(Option::is_none) {
            return;
        }
        let values = self.balance_values(rhs, state);
        for (var_id, value) in targets.iter().zip(values) {
            let Some(var_id) = *var_id else { continue };
            self.set_balance_local(state, var_id, &value, op);
            let paths =
                if op.is_some() { PathAlternatives::new() } else { value.self_address_paths };
            self.set_self_address_paths(state, var_id, paths);
        }
    }

    fn set_self_address_paths(
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

    /// Makes `var_id` hold `value`; a compound assignment also keeps what the local held before.
    fn set_balance_local(
        &self,
        state: &mut FlowState,
        var_id: VariableId,
        value: &BalanceValue,
        op: Option<BinOpKind>,
    ) {
        let forms = match op {
            None => value.forms.clone(),
            Some(op @ (BinOpKind::Add | BinOpKind::Sub)) => state
                .balance_local_forms
                .get(&var_id)
                .into_iter()
                .flatten()
                .flat_map(|lhs| {
                    value.forms.iter().filter_map(move |rhs| lhs.combine(rhs, op == BinOpKind::Sub))
                })
                .collect(),
            Some(_) => BTreeSet::new(),
        };
        state.balance_local_forms.insert(var_id, forms);
        let mut dependencies = value.dependencies.clone();
        let mut balance_paths = value.balance_paths.clone();
        let mut stale_comparisons = value.stale_comparisons.clone();
        if op.is_some() {
            dependencies.extend(
                state
                    .balance_local_dependencies
                    .get(&var_id)
                    .into_iter()
                    .flatten()
                    .filter_map(|form| form.constrained(&state.path_predicates)),
            );
            balance_paths
                .extend(state.balance_local_paths.get(&var_id).into_iter().flatten().cloned());
            stale_comparisons
                .extend(state.balance_comparison_locals.get(&var_id).into_iter().flatten());
        }
        state.balance_local_dependencies.insert(var_id, dependencies);
        state.balance_local_paths.remove(&var_id);
        state.balance_comparison_locals.remove(&var_id);
        if !balance_paths.is_empty() {
            state.balance_local_paths.insert(var_id, balance_paths);
        }
        if !stale_comparisons.is_empty() {
            state.balance_comparison_locals.insert(var_id, stale_comparisons);
        }
    }

    /// Balance facts of each component of `rhs`, one per destructuring target.
    fn balance_values(&self, rhs: &'gcx Expr<'gcx>, state: &FlowState) -> Vec<BalanceValue> {
        if let Some(elems) = tuple_elems(rhs) {
            return elems
                .iter()
                .map(|e| e.map(|e| self.balance_dependency(e, state)).unwrap_or_default())
                .collect();
        }
        match self.call_balance_values.get(&rhs.peel_parens().span) {
            Some(values) if !values.is_empty() => {
                values.iter().map(|v| v.constrained(&state.path_predicates)).collect()
            }
            _ => vec![self.balance_dependency(rhs, state)],
        }
    }

    fn balance_dependency(&self, expr: &'gcx Expr<'gcx>, state: &FlowState) -> BalanceValue {
        let pending = &state.pending_balance_calls;
        BalanceValue {
            forms: self.balance_forms(expr, state),
            dependencies: self.balance_operand_dependencies(expr, state),
            balance_paths: self.expr_balance_paths(expr, state),
            self_address_paths: self.self_address_path(expr, state),
            stale_comparisons: pending
                .iter()
                .filter(|(span, call)| {
                    self.guard_has_stale_balance_comparison(expr, **span, call, state)
                })
                .map(|(span, _)| *span)
                .collect(),
        }
    }

    /// Predicates under which `expr` evaluates to `address(this)`.
    fn self_address_path(&self, expr: &Expr<'_>, state: &FlowState) -> PathAlternatives {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Payable(inner) => self.self_address_path(inner, state),
            ExprKind::Call(callee, args, None) if is_address_cast(callee) && args.len() == 1 => {
                self.self_address_path(args.exprs().next().expect("one argument"), state)
            }
            ExprKind::Call(..) => self
                .call_balance_values
                .get(&expr.span)
                .and_then(|values| values.first())
                .map(|value| constrain_paths(&value.self_address_paths, &state.path_predicates))
                .unwrap_or_default(),
            ExprKind::Ident(_) if is_builtin(expr, sym::this) => {
                BTreeSet::from([state.path_predicates.clone()])
            }
            ExprKind::Ident(reses) => reses
                .iter()
                .filter_map(Res::as_variable)
                .filter_map(|v| state.self_address_local_paths.get(&v))
                .flat_map(|paths| constrain_paths(paths, &state.path_predicates))
                .collect(),
            _ => PathAlternatives::new(),
        }
    }

    /// Predicates under which `expr` is `<self address>.balance`.
    fn self_balance_paths(&self, expr: &Expr<'_>, state: &FlowState) -> PathAlternatives {
        match &expr.peel_parens().kind {
            ExprKind::Member(base, member) if member.name == kw::Balance => {
                self.self_address_path(base, state)
            }
            _ => PathAlternatives::new(),
        }
    }

    /// Predicates under which `expr` derives from the contract's own balance.
    fn expr_balance_paths(&self, expr: &'gcx Expr<'gcx>, state: &FlowState) -> PathAlternatives {
        let expr = expr.peel_parens();
        let self_balance = self.self_balance_paths(expr, state);
        if !self_balance.is_empty() {
            return self_balance;
        }
        let recurse = |e| self.expr_balance_paths(e, state);
        match &expr.kind {
            ExprKind::Ident(reses) => reses
                .iter()
                .filter_map(Res::as_variable)
                .filter_map(|v| state.balance_local_paths.get(&v))
                .flat_map(|paths| constrain_paths(paths, &state.path_predicates))
                .collect(),
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => recurse(inner),
            ExprKind::Binary(lhs, _, rhs) => [*lhs, *rhs].into_iter().flat_map(recurse).collect(),
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                [*cond, *true_expr, *false_expr].into_iter().flat_map(recurse).collect()
            }
            ExprKind::Call(..) => match cast_args(expr) {
                Some(args) => args.exprs().flat_map(recurse).collect(),
                None => self
                    .call_balance_values
                    .get(&expr.span)
                    .into_iter()
                    .flatten()
                    .flat_map(|value| constrain_paths(&value.balance_paths, &state.path_predicates))
                    .collect(),
            },
            _ => PathAlternatives::new(),
        }
    }

    /// Retains balance occurrences across comparison operands without assuming arithmetic
    /// identities.
    fn balance_operand_dependencies(
        &self,
        expr: &'gcx Expr<'gcx>,
        state: &FlowState,
    ) -> BTreeSet<BalanceForm> {
        let recurse = |expr| self.balance_operand_dependencies(expr, state);
        match &expr.peel_parens().kind {
            ExprKind::Binary(lhs, _, rhs) => [*lhs, *rhs].into_iter().flat_map(recurse).collect(),
            ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => recurse(inner),
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                let mut then_state = state.clone();
                let mut else_state = state.clone();
                let (then_reachable, else_reachable) =
                    self.split_on(cond, &mut then_state, &mut else_state);
                [(true_expr, then_state, then_reachable), (false_expr, else_state, else_reachable)]
                    .into_iter()
                    .filter(|(_, _, reachable)| *reachable)
                    .flat_map(|(expr, state, _)| self.balance_operand_dependencies(expr, &state))
                    .collect()
            }
            ExprKind::Call(..) if let Some(args) = cast_args(expr) => {
                args.exprs().flat_map(recurse).collect()
            }
            ExprKind::Ident(reses) => reses
                .iter()
                .filter_map(Res::as_variable)
                .filter_map(|var| state.balance_local_dependencies.get(&var))
                .flatten()
                .filter_map(|form| form.constrained(&state.path_predicates))
                .collect(),
            ExprKind::Call(..) => self
                .call_balance_values
                .get(&expr.peel_parens().span)
                .into_iter()
                .flatten()
                .flat_map(|value| &value.dependencies)
                .filter_map(|form| form.constrained(&state.path_predicates))
                .collect(),
            _ => self.balance_forms(expr, state),
        }
    }

    /// Preserves balance terms through addition/subtraction and value-preserving integer casts.
    /// Other operators may form independent offsets, but cannot transform balance terms.
    /// Unsupported expressions return no forms, rather than an independent offset.
    fn balance_forms(&self, expr: &'gcx Expr<'gcx>, state: &FlowState) -> BTreeSet<BalanceForm> {
        let expr = expr.peel_parens();
        let paths = self.self_balance_paths(expr, state);
        if !paths.is_empty() {
            return paths
                .into_iter()
                .map(|path| BalanceForm {
                    terms: vec![BalanceTerm { negative: false, stale_calls: BTreeSet::new() }],
                    path,
                })
                .collect();
        }
        let independent = || {
            BTreeSet::from([BalanceForm {
                path: state.path_predicates.clone(),
                ..BalanceForm::default()
            }])
        };
        match &expr.kind {
            ExprKind::Lit(_) => independent(),
            ExprKind::Ident(reses) => {
                let Some(var) = unique(reses.iter().filter_map(Res::as_variable)) else {
                    return BTreeSet::new();
                };
                match state.balance_local_forms.get(&var) {
                    Some(forms) => forms
                        .iter()
                        .filter_map(|form| form.constrained(&state.path_predicates))
                        .collect(),
                    None => independent(),
                }
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                let mut then_state = state.clone();
                let mut else_state = state.clone();
                let (then_reachable, else_reachable) =
                    self.split_on(cond, &mut then_state, &mut else_state);
                [(true_expr, then_state, then_reachable), (false_expr, else_state, else_reachable)]
                    .into_iter()
                    .filter(|(_, _, reachable)| *reachable)
                    .flat_map(|(expr, state, _)| self.balance_forms(expr, &state))
                    .collect()
            }
            ExprKind::Binary(lhs, op, rhs) => {
                let lhs = self.balance_forms(lhs, state);
                let rhs = self.balance_forms(rhs, state);
                lhs.iter()
                    .flat_map(|lhs| {
                        rhs.iter().filter_map(|rhs| {
                            if matches!(op.kind, BinOpKind::Add | BinOpKind::Sub)
                                || (lhs.terms.is_empty() && rhs.terms.is_empty())
                            {
                                lhs.combine(rhs, op.kind == BinOpKind::Sub)
                            } else {
                                None
                            }
                        })
                    })
                    .collect()
            }
            ExprKind::Call(callee, args, _) if cast_type(callee).is_some() && args.len() == 1 => {
                let inner = args.exprs().next().expect("one argument");
                let preserving = match (expr_ty(self.gcx, inner), expr_ty(self.gcx, expr)) {
                    (Some(from), Some(to)) => match (from.kind, to.kind) {
                        (
                            TyKind::Elementary(ElementaryType::UInt(from)),
                            TyKind::Elementary(ElementaryType::UInt(to)),
                        )
                        | (
                            TyKind::Elementary(ElementaryType::Int(from)),
                            TyKind::Elementary(ElementaryType::Int(to)),
                        ) => from.bits() <= to.bits(),
                        _ => false,
                    },
                    _ => false,
                };
                self.balance_forms(inner, state)
                    .into_iter()
                    .filter(|form| preserving || form.terms.is_empty())
                    .collect()
            }
            ExprKind::Call(..) => self
                .call_balance_values
                .get(&expr.span)
                .into_iter()
                .flatten()
                .flat_map(|value| {
                    value.forms.iter().filter_map(|form| form.constrained(&state.path_predicates))
                })
                .collect(),
            _ => BTreeSet::new(),
        }
    }

    /// Binds the balance facts of the call arguments to the callee's parameters.
    fn seed_balance_parameters(
        &mut self,
        func: &'gcx hir::Function<'gcx>,
        args: &CallArgs<'gcx>,
        state: &mut FlowState,
    ) {
        if !self.reentrancy_balance_enabled {
            return;
        }
        for &param in func.parameters {
            match arg_for_param(&self.gcx.hir, func, param, args) {
                Some(arg) => self.bind_locals(state, &[Some(param)], arg, None),
                None => self.clear_local(state, param),
            }
        }
    }

    fn clear_local(&self, state: &mut FlowState, var_id: VariableId) {
        self.set_balance_local(state, var_id, &BalanceValue::default(), None);
        self.set_self_address_paths(state, var_id, PathAlternatives::new());
    }

    /// Accumulates the values returned by `return expr;` (or the named return variables when
    /// the function falls through) into the innermost return collector.
    fn record_return(&mut self, expr: Option<&'gcx Expr<'gcx>>, state: &FlowState) {
        let Some(&(func_id, _)) = self.return_collectors.last() else { return };
        let values = match expr {
            Some(expr) => self.balance_values(expr, state),
            None => self
                .gcx
                .hir
                .function(func_id)
                .returns
                .iter()
                .map(|&var_id| self.local_value(var_id, state))
                .collect(),
        };
        let (_, stored) = self.return_collectors.last_mut().expect("return collector is active");
        for (stored, value) in stored.iter_mut().zip(values) {
            stored.merge(value);
        }
    }

    /// The balance facts currently recorded for the local `var_id`.
    fn local_value(&self, var_id: VariableId, state: &FlowState) -> BalanceValue {
        BalanceValue {
            forms: state.balance_local_forms.get(&var_id).cloned().unwrap_or_default(),
            dependencies: state
                .balance_local_dependencies
                .get(&var_id)
                .cloned()
                .unwrap_or_default(),
            balance_paths: state.balance_local_paths.get(&var_id).cloned().unwrap_or_default(),
            self_address_paths: state
                .self_address_local_paths
                .get(&var_id)
                .cloned()
                .unwrap_or_default(),
            stale_comparisons: state
                .balance_comparison_locals
                .get(&var_id)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Drops every fact about locals of `func_id` once its inlined body is left.
    fn clear_function_locals(&self, func_id: FunctionId, state: &mut FlowState) {
        let owned = owned_by(&self.gcx.hir, func_id);
        state.internal_function_targets.retain(|v, _| !owned(*v));
        state.self_address_local_paths.retain(|v, _| !owned(*v));
        state.balance_local_paths.retain(|v, _| !owned(*v));
        state.balance_local_forms.retain(|v, _| !owned(*v));
        state.balance_local_dependencies.retain(|v, _| !owned(*v));
        state.balance_comparison_locals.retain(|v, _| !owned(*v));
        state.path_predicates.retain(|predicate, _| !predicate.mentions(&owned));
    }

    fn reentrant_call_kind(
        &self,
        callee: &'gcx Expr<'gcx>,
        opts: Option<&CallOptions<'gcx>>,
    ) -> Option<ReentrantCallKind> {
        if self.reentrancy_eth_enabled && is_uncapped_value_call(&self.gcx.hir, callee, opts) {
            Some(ReentrantCallKind::Eth)
        } else if self.reentrancy_no_eth_enabled
            && !call_sends_eth(&self.gcx.hir, opts)
            && callee_can_reenter(self.gcx, callee)
        {
            Some(ReentrantCallKind::NoEth)
        } else {
            None
        }
    }

    /// True if the contract-wide reentrancy lock is held and intact around the call.
    fn balance_guard_blocks_call(&self, state: &FlowState, callee: &'gcx Expr<'gcx>) -> bool {
        !call_uses_delegate_context(self.gcx, callee)
            && self.balance_reentry_lock.is_some_and(|lock| {
                self.active_balance_guards.contains(&lock)
                    && !state.invalidated_balance_guards.contains(&lock)
            })
    }
}

/// Collects the internal functions and modifiers a function invokes directly.
struct CallCollector<'gcx> {
    gcx: Gcx<'gcx>,
    calls: BTreeSet<FunctionId>,
}

impl<'gcx> Visit<'gcx> for CallCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_modifier(
        &mut self,
        modifier: &'gcx hir::Modifier<'gcx>,
    ) -> ControlFlow<Self::BreakValue> {
        self.calls.extend(modifier.id.as_function());
        self.visit_call_args(&modifier.args)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        collect_internal_calls(self.gcx, expr, &mut self.calls);
        ControlFlow::Continue(())
    }
}

fn collect_internal_calls(gcx: Gcx<'_>, expr: &Expr<'_>, calls: &mut BTreeSet<FunctionId>) {
    if let ExprKind::Call(callee, ..) = &expr.kind {
        calls.extend(static_internal_callee(gcx, callee));
    }
    for_each_child(expr, &mut |child| collect_internal_calls(gcx, child, calls));
}

/// Internal function statically named by `callee`: a bare identifier or `super.f`.
fn static_internal_callee(gcx: Gcx<'_>, callee: &Expr<'_>) -> Option<FunctionId> {
    let callee = callee.peel_parens();
    let direct = match &callee.kind {
        ExprKind::Ident(_) => true,
        ExprKind::Member(base, _) => is_builtin(base, sym::super_),
        _ => false,
    };
    let TyKind::Fn(function) = gcx.type_of_expr(callee.id).filter(|_| direct)?.kind else {
        return None;
    };
    function.is_internal().then_some(function.function_id).flatten()
}

/// The variables of `func_id`.
fn owned_by(hir: &hir::Hir<'_>, func_id: FunctionId) -> impl Fn(VariableId) -> bool {
    move |var_id| hir.variable(var_id).parent == Some(ItemId::Function(func_id))
}

/// The boolean fact `expr` establishes when it evaluates to `true`: a local flag, its negation,
/// or an `==`/`!=` between locals and literals.
fn path_predicate(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> Option<(PathPredicate, bool)> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => Some((PathPredicate::Boolean(lhs_local_var(hir, expr)?), true)),
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            path_predicate(hir, inner).map(|(predicate, value)| (predicate, !value))
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) => {
            let (lhs, rhs) = (predicate_operand(hir, lhs)?, predicate_operand(hir, rhs)?);
            let predicate = PathPredicate::Equality(lhs.min(rhs), lhs.max(rhs));
            Some((predicate, op.kind == BinOpKind::Eq))
        }
        _ => None,
    }
}

fn predicate_operand(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> Option<Operand> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(_) => Some(Operand::Variable(lhs_local_var(hir, expr)?)),
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Number(value) => Some(Operand::Number(value)),
            LitKind::Bool(value) => Some(Operand::Boolean(value)),
            _ => None,
        },
        _ => None,
    }
}

/// Records in `state` that `expr` evaluated to `outcome`; returns `false` if that is impossible.
fn constrain_boolean_outcome(
    hir: &hir::Hir<'_>,
    expr: &Expr<'_>,
    outcome: bool,
    state: &mut FlowState,
) -> bool {
    if let Some((predicate, value)) = path_predicate(hir, expr) {
        return state.constrain_path((predicate, value == outcome));
    }
    match &expr.peel_parens().kind {
        // `a && b` being true (or `a || b` being false) fixes both operands.
        ExprKind::Binary(lhs, op, rhs)
            if op.kind == if outcome { BinOpKind::And } else { BinOpKind::Or } =>
        {
            constrain_boolean_outcome(hir, lhs, outcome, state)
                && constrain_boolean_outcome(hir, rhs, outcome, state)
        }
        _ => true,
    }
}

fn common_path_predicates(lhs: &PathPredicates, rhs: &PathPredicates) -> PathPredicates {
    lhs.iter().filter(|(p, v)| rhs.get(p) == Some(v)).map(|(p, v)| (*p, *v)).collect()
}

fn paths_compatible(lhs: &PathPredicates, rhs: &PathPredicates) -> bool {
    lhs.iter().all(|(p, v)| rhs.get(p).is_none_or(|other| other == v))
}

/// Keeps the `paths` compatible with `active`, extended by the active predicates.
fn constrain_paths(paths: &PathAlternatives, active: &PathPredicates) -> PathAlternatives {
    paths
        .iter()
        .filter(|path| paths_compatible(path, active))
        .map(|path| path.iter().chain(active).map(|(p, v)| (*p, *v)).collect())
        .collect()
}

/// Rewrites the callee-local predicates in the returned `values` into the caller's terms.
fn remap_return_paths(
    hir: &hir::Hir<'_>,
    func_id: FunctionId,
    parameters: &[VariableId],
    parameter_predicates: &[Option<(PathPredicate, bool)>],
    values: &mut [BalanceValue],
) {
    let owned = owned_by(hir, func_id);
    let remap = |mut path: PathPredicates| {
        for (&parameter, &argument) in parameters.iter().zip(parameter_predicates) {
            let Some(parameter_value) = path.remove(&PathPredicate::Boolean(parameter)) else {
                continue;
            };
            if let Some((predicate, argument_value)) = argument {
                let mapped = parameter_value == argument_value;
                if path.get(&predicate).is_some_and(|existing| *existing != mapped) {
                    return None;
                }
                path.insert(predicate, mapped);
            }
        }
        path.retain(|predicate, _| !predicate.mentions(&owned));
        Some(path)
    };
    for value in values {
        value.balance_paths =
            std::mem::take(&mut value.balance_paths).into_iter().filter_map(remap).collect();
        value.self_address_paths =
            std::mem::take(&mut value.self_address_paths).into_iter().filter_map(remap).collect();
        for forms in [&mut value.forms, &mut value.dependencies] {
            *forms = std::mem::take(forms)
                .into_iter()
                .filter_map(|form| Some(BalanceForm { path: remap(form.path)?, ..form }))
                .collect();
        }
    }
}

/// Drops every path fact that mentions the (re)assigned local `var_id`.
fn forget_path_predicates(state: &mut FlowState, var_id: VariableId) {
    let mentions = |predicate: &PathPredicate| predicate.mentions(|v| v == var_id);
    state.path_predicates.retain(|p, _| !mentions(p));
    let strip = |paths: &mut PathAlternatives| {
        *paths = paths
            .iter()
            .map(|path| path.iter().filter(|(p, _)| !mentions(p)).map(|(p, v)| (*p, *v)).collect())
            .collect();
    };
    for forms in
        state.balance_local_forms.values_mut().chain(state.balance_local_dependencies.values_mut())
    {
        *forms = std::mem::take(forms)
            .into_iter()
            .map(|mut form| {
                form.path.retain(|p, _| !mentions(p));
                form
            })
            .collect();
    }
    state.balance_local_paths.values_mut().for_each(strip);
    state.self_address_local_paths.values_mut().for_each(strip);
    state.pending_balance_calls.values_mut().for_each(strip);
}

/// Arguments of a plain type conversion such as `uint256(x)` or `address(x)`.
fn cast_args<'a>(expr: &'a Expr<'a>) -> Option<&'a CallArgs<'a>> {
    match &expr.peel_parens().kind {
        ExprKind::Call(callee, args, None)
            if matches!(callee.peel_parens().kind, ExprKind::Type(_) | ExprKind::TypeCall(_)) =>
        {
            Some(args)
        }
        _ => None,
    }
}

fn call_option<'a>(opts: Option<&'a CallOptions<'a>>, name: Symbol) -> Option<&'a Expr<'a>> {
    opts?.args.iter().find(|opt| opt.name.name == name).map(|opt| &opt.value)
}

fn call_sends_eth(hir: &hir::Hir<'_>, opts: Option<&CallOptions<'_>>) -> bool {
    call_option(opts, sym::value).is_some_and(|value| !is_zero_value(hir, value))
}

/// `.call{value: v}(...)` with a non-zero value and no gas cap other than `gasleft()`.
fn is_uncapped_value_call(
    hir: &hir::Hir<'_>,
    callee: &Expr<'_>,
    opts: Option<&CallOptions<'_>>,
) -> bool {
    matches!(&callee.peel_parens().kind, ExprKind::Member(_, member) if member.name == kw::Call)
        && call_sends_eth(hir, opts)
        && call_option(opts, kw::Gas).is_none_or(|gas| {
            matches!(&gas.peel_parens().kind, ExprKind::Call(callee, args, None)
                if args.is_empty() && is_builtin(callee, sym::gasleft))
        })
}

/// True unless a `gas:` option provably leaves the callee too little gas to reenter.
fn call_options_allow_reentrancy(hir: &hir::Hir<'_>, opts: Option<&CallOptions<'_>>) -> bool {
    let Some(gas) = call_option(opts, kw::Gas) else { return true };
    let sends_eth = call_sends_eth(hir, opts);
    match const_value(hir, gas, None, &mut BTreeSet::new()) {
        Some(Operand::Number(gas)) => {
            gas > U256::from(REENTRANCY_GAS_STIPEND) || (sends_eth && !gas.is_zero())
        }
        _ => true,
    }
}

fn is_zero_value(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> bool {
    matches!(const_value(hir, expr, None, &mut BTreeSet::new()), Some(Operand::Number(n)) if n.is_zero())
}

/// `break`/`continue` or anything that exits the function, so the current path stops here.
fn branch_stops_current_path(stmt: &Stmt<'_>) -> bool {
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

/// The lock state variable of a standard reentrancy guard modifier: it rejects re-entry, sets
/// the lock, runs `_` exactly once and restores the lock right after.
fn standard_reentrancy_guard_lock(
    hir: &hir::Hir<'_>,
    modifier: &hir::Function<'_>,
) -> Option<VariableId> {
    if !matches!(modifier.kind, FunctionKind::Modifier) || !modifier.modifiers.is_empty() {
        return None;
    }
    let stmts = modifier.body?.stmts;
    if count_placeholders(stmts) != 1 {
        return None;
    }
    let mut activation = Vec::new();
    stmts_before_placeholder(stmts, &mut activation)?;
    let (lock_var, entered) = guard_activation(hir, &activation, &mut BTreeSet::new())?;
    let index = stmts.iter().position(|s| count_placeholders(std::slice::from_ref(s)) == 1)?;
    let (restored_var, restored) = guard_restoration(hir, stmts.get(index + 1)?)?;
    (lock_var == restored_var && entered != restored).then_some(lock_var)
}

/// The lock and value set by the last of `stmts` (directly or via an argument-less helper), if
/// an earlier statement rejects that value.
fn guard_activation(
    hir: &hir::Hir<'_>,
    stmts: &[&Stmt<'_>],
    seen: &mut BTreeSet<FunctionId>,
) -> Option<(VariableId, Operand)> {
    let (activation, prefix) = stmts.split_last()?;
    if let Some((lock_var, entered)) = state_lock_assignment(hir, activation) {
        return prefix
            .iter()
            .any(|stmt| stmt_rejects_lock_value(hir, stmt, lock_var, entered))
            .then_some((lock_var, entered));
    }
    let helper_id = simple_internal_call(activation)?;
    let helper = hir.function(helper_id);
    if !helper.modifiers.is_empty() || !seen.insert(helper_id) {
        return None;
    }
    let body = helper.body?.stmts.iter().collect::<Vec<_>>();
    let result = guard_activation(hir, &body, seen);
    seen.remove(&helper_id);
    result
}

/// The lock and value restored by `stmt` (directly or via a single-statement helper).
fn guard_restoration(hir: &hir::Hir<'_>, stmt: &Stmt<'_>) -> Option<(VariableId, Operand)> {
    state_lock_assignment(hir, stmt).or_else(|| {
        let helper = hir.function(simple_internal_call(stmt)?);
        let [stmt] = helper.modifiers.is_empty().then_some(helper.body?.stmts)? else {
            return None;
        };
        state_lock_assignment(hir, stmt)
    })
}

/// `f();` naming exactly one function.
fn simple_internal_call(stmt: &Stmt<'_>) -> Option<FunctionId> {
    let StmtKind::Expr(expr) = stmt.kind else { return None };
    let ExprKind::Call(callee, args, None) = &expr.peel_parens().kind else { return None };
    args.is_empty().then(|| unique(function_ids(callee))).flatten()
}

/// `lock = <constant>;` on a state variable.
fn state_lock_assignment(hir: &hir::Hir<'_>, stmt: &Stmt<'_>) -> Option<(VariableId, Operand)> {
    let StmtKind::Expr(expr) = stmt.kind else { return None };
    let ExprKind::Assign(lhs, None, rhs) = &expr.peel_parens().kind else { return None };
    let ExprKind::Ident(reses) = &lhs.peel_parens().kind else { return None };
    let lock_var = unique(
        reses.iter().filter_map(Res::as_variable).filter(|v| hir.variable(*v).kind.is_state()),
    )?;
    Some((lock_var, const_value(hir, rhs, None, &mut BTreeSet::new())?))
}

/// True if `stmt` reverts whenever `lock_var` holds `entered`.
fn stmt_rejects_lock_value(
    hir: &hir::Hir<'_>,
    stmt: &Stmt<'_>,
    lock_var: VariableId,
    entered: Operand,
) -> bool {
    let eval = |cond| match const_value(hir, cond, Some((lock_var, entered)), &mut BTreeSet::new())
    {
        Some(Operand::Boolean(value)) => Some(value),
        _ => None,
    };
    match stmt.kind {
        StmtKind::Expr(expr) => {
            let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
            is_require_or_assert(callee)
                && args.exprs().next().is_some_and(|cond| eval(cond) == Some(false))
        }
        StmtKind::If(cond, then_stmt, else_stmt) => match eval(cond) {
            Some(true) => branch_always_exits(then_stmt),
            Some(false) => else_stmt.is_some_and(branch_always_exits),
            None => false,
        },
        _ => false,
    }
}

/// Constant-folds `expr` over literals, `constant` variables, casts, `!` and `==`/`!=`; `lock`
/// supplies the value of the lock variable.
fn const_value(
    hir: &hir::Hir<'_>,
    expr: &Expr<'_>,
    lock: Option<(VariableId, Operand)>,
    seen: &mut BTreeSet<VariableId>,
) -> Option<Operand> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Bool(value) => Some(Operand::Boolean(value)),
            LitKind::Number(value) => Some(Operand::Number(value)),
            _ => None,
        },
        ExprKind::Ident(reses) => {
            let var_id = unique(reses.iter().filter_map(Res::as_variable))?;
            if let Some((lock_var, entered)) = lock
                && lock_var == var_id
            {
                return Some(entered);
            }
            let var = hir.variable(var_id);
            if !var.is_constant() || !seen.insert(var_id) {
                return None;
            }
            let value = const_value(hir, var.initializer?, lock, seen);
            seen.remove(&var_id);
            value
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            match const_value(hir, inner, lock, seen)? {
                Operand::Boolean(value) => Some(Operand::Boolean(!value)),
                _ => None,
            }
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) => {
            let lhs = const_value(hir, lhs, lock, seen)?;
            let rhs = const_value(hir, rhs, lock, seen)?;
            Some(Operand::Boolean((lhs == rhs) == (op.kind == BinOpKind::Eq)))
        }
        ExprKind::Call(..) => {
            let args = cast_args(expr).filter(|args| args.len() == 1)?;
            const_value(hir, args.exprs().next()?, lock, seen)
        }
        _ => None,
    }
}

/// The reentrancy-guard lock, if any, that protects every entry point of every deployable
/// contract exposing `entry`.
fn balance_reentry_lock<'gcx>(
    gcx: Gcx<'gcx>,
    entry: &'gcx hir::Function<'gcx>,
) -> Option<VariableId> {
    let entry_id = gcx.hir.function_ids().find(|&id| std::ptr::eq(gcx.hir.function(id), entry))?;
    let defining_contract = entry.contract?;
    guard_locks(&gcx.hir, entry).into_iter().find(|&lock_var| {
        let mut deployed = false;
        for contract_id in gcx.hir.contract_ids() {
            let contract = gcx.hir.contract(contract_id);
            if !contract.can_be_deployed()
                || contract.is_abstract()
                || !contract.linearized_bases.contains(&defining_contract)
            {
                continue;
            }
            let interface = gcx.interface_functions(contract_id);
            let special = || [contract.fallback, contract.receive].into_iter().flatten();
            if !interface.iter().any(|f| f.id == entry_id) && !special().any(|id| id == entry_id) {
                continue;
            }
            deployed = true;
            let guarded = interface
                .iter()
                .map(|f| gcx.hir.function(f.id))
                .filter(|f| !is_view_or_pure(f.state_mutability))
                .chain(special().map(|id| gcx.hir.function(id)))
                .all(|f| guard_locks(&gcx.hir, f).contains(&lock_var));
            if !guarded {
                return false;
            }
        }
        deployed
    })
}

/// Locks of the standard reentrancy guards among `function`'s modifiers.
fn guard_locks(hir: &hir::Hir<'_>, function: &hir::Function<'_>) -> Vec<VariableId> {
    function
        .modifiers
        .iter()
        .filter(|modifier| modifier.args.is_empty())
        .filter_map(|modifier| modifier.id.as_function())
        .filter_map(|id| standard_reentrancy_guard_lock(hir, hir.function(id)))
        .collect()
}

/// `delegatecall`/`callcode`, which run the callee in the caller's storage context.
fn call_uses_delegate_context(gcx: Gcx<'_>, callee: &Expr<'_>) -> bool {
    let callee = callee.peel_parens();
    matches!(&callee.kind, ExprKind::Member(_, member)
        if matches!(member.name, kw::Callcode | kw::Delegatecall))
        || gcx.type_of_expr(callee.id).is_some_and(
            |ty| matches!(ty.kind, TyKind::Fn(function) if function.kind == TyFnKind::DelegateCall),
        )
}

/// An external call that hands control to another contract: a low-level `call`/`callcode`/
/// `delegatecall` on an address, or a state-changing external function call.
fn callee_can_reenter<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    let callee = callee.peel_parens();
    match &callee.kind {
        ExprKind::Member(receiver, member)
            if is_address_like(gcx, receiver)
                && matches!(
                    member.name,
                    kw::Call | kw::Callcode | kw::Delegatecall | kw::Staticcall
                ) =>
        {
            member.name != kw::Staticcall
        }
        ExprKind::Member(receiver, _) if is_builtin(receiver, sym::super_) => false,
        _ => {
            let Some(TyKind::Fn(function)) = gcx.type_of_expr(callee.id).map(|ty| ty.kind) else {
                return false;
            };
            matches!(
                function.kind,
                TyFnKind::External | TyFnKind::Declaration | TyFnKind::DelegateCall
            ) && !is_view_or_pure(function.state_mutability)
        }
    }
}
