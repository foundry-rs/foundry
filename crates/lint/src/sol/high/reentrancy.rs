use super::ReentrancyEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::helper_cache::{DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT, HelperAnalysisCache},
    },
};
use solar::{
    ast::{
        BinOpKind, DataLocation, ElementaryType, LitKind, StateMutability, StrKind, TypeSize,
        UnOpKind, Visibility,
    },
    interface::{Span, kw, sym},
    sema::{
        Gcx, Ty,
        builtins::Builtin,
        hir::{
            self, CallArgs, CallArgsKind, ExprKind, FunctionId, ItemId, LoopSource, Res, StmtKind,
            VariableId,
        },
        ty::{TyFnKind, TyKind},
    },
};
use std::collections::{BTreeSet, HashMap, HashSet};

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

declare_forge_lint!(
    REENTRANCY_UNLIMITED_GAS,
    Severity::Info,
    "reentrancy-unlimited-gas",
    "state change or event emission follows `transfer`/`send`; gas repricing could enable reentrancy"
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

        let mut analyzer = Analyzer::new(ctx, gcx, hir);
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
    stored_send_results: Vec<StoredSendResult>,
    stored_call_results: Vec<StoredCallResult>,
    stored_return_results: Vec<StoredReturnResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PendingCall {
    span: Span,
    kind: ReentrantCallKind,
    state_reads: BTreeSet<VariableId>,
    result_correlatable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SendResultSource {
    call_span: Span,
    succeeds_when_true: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StoredSendResult {
    variable: VariableId,
    source: SendResultSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StoredCallResult {
    expression_span: Span,
    index: usize,
    source: SendResultSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StoredReturnResult {
    function: FunctionId,
    index: usize,
    source: SendResultSource,
}

#[derive(Default)]
struct CallReturns {
    function: Option<FunctionId>,
    state: Option<FlowState>,
}

#[derive(Default)]
struct LoopJumps<'hir> {
    breaks: Option<FlowState>,
    continues: Option<FlowState>,
    continue_epilogue: Option<ContinueEpilogue<'hir>>,
}

#[derive(Clone, Copy)]
enum ContinueEpilogue<'hir> {
    Expr(&'hir hir::Expr<'hir>),
    Block(hir::Block<'hir>),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ReentrantCallKind {
    Eth,
    NoEth,
    Stipend,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SendEvaluation {
    NotEvaluated,
    Failed,
    Succeeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BooleanOutcome {
    value: bool,
    send: SendEvaluation,
}

fn unconstrained_boolean_outcomes(contains_target: bool) -> Vec<BooleanOutcome> {
    let evaluations: &[_] = if contains_target {
        &[SendEvaluation::NotEvaluated, SendEvaluation::Failed, SendEvaluation::Succeeded]
    } else {
        &[SendEvaluation::NotEvaluated]
    };
    evaluations
        .iter()
        .flat_map(|&send| {
            [BooleanOutcome { value: true, send }, BooleanOutcome { value: false, send }]
        })
        .collect()
}

fn merge_send_evaluations(lhs: SendEvaluation, rhs: SendEvaluation) -> Option<SendEvaluation> {
    match (lhs, rhs) {
        (SendEvaluation::NotEvaluated, other) | (other, SendEvaluation::NotEvaluated) => {
            Some(other)
        }
        (lhs, rhs) if lhs == rhs => Some(lhs),
        _ => None,
    }
}

fn push_unique_boolean_outcome(outcomes: &mut Vec<BooleanOutcome>, outcome: BooleanOutcome) {
    if !outcomes.contains(&outcome) {
        outcomes.push(outcome);
    }
}

impl FlowState {
    fn push_read(&mut self, var_id: VariableId) {
        self.state_reads.insert(var_id);
    }

    fn push_call(&mut self, span: Span, kind: ReentrantCallKind) {
        if self.state_reads.is_empty() && kind != ReentrantCallKind::Stipend {
            return;
        }

        if let Some(existing) =
            self.pending_calls.iter_mut().find(|call| call.span == span && call.kind == kind)
        {
            existing.state_reads.extend(self.state_reads.iter().copied());
            existing.result_correlatable = false;
            self.stored_send_results.retain(|result| result.source.call_span != span);
            self.stored_call_results.retain(|result| result.source.call_span != span);
            self.stored_return_results.retain(|result| result.source.call_span != span);
        } else {
            self.pending_calls.push(PendingCall {
                span,
                kind,
                state_reads: self.state_reads.clone(),
                result_correlatable: true,
            });
        }
    }

    fn remove_call(&mut self, span: Span, kind: ReentrantCallKind) {
        self.pending_calls.retain(|call| call.span != span || call.kind != kind);
        self.stored_send_results.retain(|result| result.source.call_span != span);
        self.stored_call_results.retain(|result| result.source.call_span != span);
        self.stored_return_results.retain(|result| result.source.call_span != span);
    }

    fn set_stored_send_results(&mut self, variable: VariableId, sources: &[SendResultSource]) {
        self.stored_send_results.retain(|result| result.variable != variable);
        for &source in sources {
            if self.is_correlatable_source(source) {
                let result = StoredSendResult { variable, source };
                if !self.stored_send_results.contains(&result) {
                    self.stored_send_results.push(result);
                }
            }
        }
    }

    fn clear_stored_call_results(&mut self, expression_span: Span) {
        self.stored_call_results.retain(|result| result.expression_span != expression_span);
    }

    fn set_stored_call_results(
        &mut self,
        expression_span: Span,
        sources: &[Vec<SendResultSource>],
    ) {
        self.clear_stored_call_results(expression_span);
        for (index, sources) in sources.iter().enumerate() {
            for &source in sources {
                if self.is_correlatable_source(source) {
                    let result = StoredCallResult { expression_span, index, source };
                    if !self.stored_call_results.contains(&result) {
                        self.stored_call_results.push(result);
                    }
                }
            }
        }
    }

    fn clear_stored_return_results(&mut self, function: FunctionId) {
        self.stored_return_results.retain(|result| result.function != function);
    }

    fn set_stored_return_results(
        &mut self,
        function: FunctionId,
        sources: &[Vec<SendResultSource>],
    ) {
        self.clear_stored_return_results(function);
        for (index, sources) in sources.iter().enumerate() {
            for &source in sources {
                if self.is_correlatable_source(source) {
                    let result = StoredReturnResult { function, index, source };
                    if !self.stored_return_results.contains(&result) {
                        self.stored_return_results.push(result);
                    }
                }
            }
        }
    }

    fn is_correlatable_source(&self, source: SendResultSource) -> bool {
        self.pending_calls.iter().any(|call| {
            call.span == source.call_span
                && call.kind == ReentrantCallKind::Stipend
                && call.result_correlatable
        })
    }
}

fn merge_optional_state(target: &mut Option<FlowState>, state: &FlowState) {
    if let Some(target) = target {
        target.merge(state);
    } else {
        *target = Some(state.clone());
    }
}

/// Returns the Solidity update expression or Yul post block lowered after a `for` loop body.
///
/// A source-level `continue` skips this synthetic statement in HIR, even though the source
/// language executes it before the next condition check.
fn for_loop_continue_epilogue<'hir>(
    block: hir::Block<'hir>,
    source: LoopSource,
) -> Option<ContinueEpilogue<'hir>> {
    if source != LoopSource::For || block.stmts.len() != 1 {
        return None;
    }

    let stmt = block.stmts.first()?;
    let body = match stmt.kind {
        StmtKind::If(_, then_stmt, Some(else_stmt))
            if matches!(else_stmt.kind, StmtKind::Break) =>
        {
            then_stmt
        }
        _ => stmt,
    };
    let StmtKind::Block(body) = body.kind else { return None };
    if body.span != block.span || body.stmts.len() < 2 {
        return None;
    }

    match body.stmts.last()?.kind {
        StmtKind::Expr(epilogue) => Some(ContinueEpilogue::Expr(epilogue)),
        StmtKind::Block(epilogue) => Some(ContinueEpilogue::Block(epilogue)),
        _ => None,
    }
}

/// Returns the user statements and condition from Solar's lowered `do-while` loop.
fn do_while_parts<'hir>(
    block: hir::Block<'hir>,
    source: LoopSource,
) -> Option<(&'hir [hir::Stmt<'hir>], &'hir hir::Expr<'hir>)> {
    if source != LoopSource::DoWhile {
        return None;
    }

    let (check, body) = block.stmts.split_last()?;
    let StmtKind::If(condition, then_stmt, Some(else_stmt)) = check.kind else { return None };
    if !matches!(then_stmt.kind, StmtKind::Continue) || !matches!(else_stmt.kind, StmtKind::Break) {
        return None;
    }
    Some((body, condition))
}

fn is_state_mutating_array_call(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    matches!(
        gcx.builtin_callee(callee.peel_parens().id),
        Some(Builtin::ArrayPush0 | Builtin::ArrayPush | Builtin::ArrayPop)
    )
}

fn is_builtin_assertion(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    matches!(gcx.builtin_callee(callee.peel_parens().id), Some(Builtin::Require | Builtin::Assert))
}

fn is_non_returning_builtin_call(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    matches!(
        gcx.builtin_callee(callee.peel_parens().id),
        Some(
            Builtin::Revert
                | Builtin::RevertMsg
                | Builtin::Selfdestruct
                | Builtin::YulInvalid
                | Builtin::YulReturn
                | Builtin::YulRevert
                | Builtin::YulSelfdestruct
                | Builtin::YulStop
        )
    )
}

fn is_successful_halt_builtin_call(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    matches!(
        gcx.builtin_callee(callee.peel_parens().id),
        Some(
            Builtin::Selfdestruct
                | Builtin::YulReturn
                | Builtin::YulSelfdestruct
                | Builtin::YulStop
        )
    )
}

fn is_yul_state_effect(gcx: Gcx<'_>, callee: &hir::Expr<'_>) -> bool {
    matches!(
        gcx.builtin_callee(callee.peel_parens().id),
        Some(
            Builtin::YulSstore
                | Builtin::YulTstore
                | Builtin::YulLog0
                | Builtin::YulLog1
                | Builtin::YulLog2
                | Builtin::YulLog3
                | Builtin::YulLog4
        )
    )
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    emitted: HashSet<Span>,
    call_stack: Vec<FunctionId>,
    inline_cache: HelperAnalysisCache<InlineCallKey, InlineCallResult>,
    recursive_cut_frontiers: HashMap<RecursiveFrontierKey, Vec<FunctionId>>,
    direct_internal_calls: HashMap<FunctionId, Vec<FunctionId>>,
    loop_jumps: Vec<LoopJumps<'hir>>,
    call_returns: Vec<CallReturns>,
    state_effects: usize,
    completion_probe_depth: usize,
    completion_probe_successful_halt: bool,
    reentrancy_eth_enabled: bool,
    reentrancy_no_eth_enabled: bool,
    reentrancy_unlimited_gas_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    /// First active function that can cut recursion from this callee.
    recursive_cut: Option<FunctionId>,
    state: FlowState,
}

#[derive(Clone)]
struct InlineCallResult {
    state: FlowState,
    may_return: bool,
    has_state_effect: bool,
    return_sources: Vec<Vec<SendResultSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RecursiveFrontierKey {
    func_id: FunctionId,
    active_call_stack: Vec<FunctionId>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(ctx: &'ctx LintContext<'s, 'c>, gcx: Gcx<'hir>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self {
            ctx,
            gcx,
            hir,
            emitted: HashSet::new(),
            call_stack: Vec::new(),
            inline_cache: HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
            recursive_cut_frontiers: HashMap::new(),
            direct_internal_calls: HashMap::new(),
            loop_jumps: Vec::new(),
            call_returns: Vec::new(),
            state_effects: 0,
            completion_probe_depth: 0,
            completion_probe_successful_halt: false,
            reentrancy_eth_enabled: ctx.is_lint_enabled(REENTRANCY_ETH.id),
            reentrancy_no_eth_enabled: ctx.is_lint_enabled(REENTRANCY_NO_ETH.id),
            reentrancy_unlimited_gas_enabled: ctx.is_lint_enabled(REENTRANCY_UNLIMITED_GAS.id),
        }
    }

    const fn has_enabled_lints(&self) -> bool {
        self.reentrancy_eth_enabled
            || self.reentrancy_no_eth_enabled
            || self.reentrancy_unlimited_gas_enabled
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
            if !self.analyze_expr(arg, state) {
                return false;
            }
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

        self.call_stack.push(modifier_id);
        let falls_through =
            self.analyze_block(modifier_body, Some((modifiers, index + 1, body)), state);
        self.call_stack.pop();
        falls_through
    }

    fn analyze_block(
        &mut self,
        block: hir::Block<'hir>,
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
        state: &mut FlowState,
    ) -> bool {
        self.analyze_stmts(block.stmts, placeholder, state)
    }

    fn analyze_stmts(
        &mut self,
        stmts: &'hir [hir::Stmt<'hir>],
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
        state: &mut FlowState,
    ) -> bool {
        for stmt in stmts {
            if !self.analyze_stmt(stmt, placeholder, state) {
                return false;
            }
        }
        true
    }

    fn analyze_stmt(
        &mut self,
        stmt: &'hir hir::Stmt<'hir>,
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
        state: &mut FlowState,
    ) -> bool {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    let completes = self.analyze_expr(init, state);
                    if completes {
                        let sources = self.send_result_sources(init, state);
                        state.set_stored_send_results(var_id, &sources);
                    }
                    completes
                } else {
                    true
                }
            }
            StmtKind::DeclMulti(variables, expr) => {
                let completes = self.analyze_expr(expr, state);
                if completes {
                    let sources = self.send_result_sources_by_index(expr, variables.len(), state);
                    for (index, variable) in variables.iter().enumerate() {
                        if let Some(variable) = variable {
                            state.set_stored_send_results(*variable, &sources[index]);
                        }
                    }
                }
                completes
            }
            StmtKind::Expr(expr) => self.analyze_expr(expr, state),
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.analyze_block(block, placeholder, state)
            }
            StmtKind::Emit(expr) => {
                if !self.analyze_expr(expr, state) {
                    return false;
                }
                self.emit_pending_stipend_calls(state);
                self.record_state_effect();
                true
            }
            StmtKind::Revert(expr) => {
                self.analyze_expr(expr, state);
                false
            }
            StmtKind::Return(expr) => {
                let completes = expr.is_none_or(|expr| self.analyze_expr(expr, state));
                if completes {
                    if let Some(function) =
                        self.call_returns.last().and_then(|returns| returns.function)
                    {
                        let sources = self.return_result_sources(function, expr, state);
                        state.set_stored_return_results(function, &sources);
                    }
                    if let Some(returns) = self.call_returns.last_mut() {
                        merge_optional_state(&mut returns.state, state);
                    }
                }
                false
            }
            StmtKind::Break => {
                if let Some(jumps) = self.loop_jumps.last_mut() {
                    merge_optional_state(&mut jumps.breaks, state);
                }
                false
            }
            StmtKind::Continue => {
                let epilogue = self.loop_jumps.last().and_then(|jumps| jumps.continue_epilogue);
                let completes = match epilogue {
                    Some(ContinueEpilogue::Expr(epilogue)) => self.analyze_expr(epilogue, state),
                    Some(ContinueEpilogue::Block(epilogue)) => {
                        self.analyze_block(epilogue, placeholder, state)
                    }
                    None => true,
                };
                if completes && let Some(jumps) = self.loop_jumps.last_mut() {
                    merge_optional_state(&mut jumps.continues, state);
                }
                false
            }
            StmtKind::Loop(block, source) => {
                let before_loop = state.clone();
                let mut header_state = before_loop.clone();
                let mut exit_state = None;
                let continue_epilogue = for_loop_continue_epilogue(block, source);
                let do_while_parts = do_while_parts(block, source);

                loop {
                    self.loop_jumps.push(LoopJumps { continue_epilogue, ..Default::default() });
                    let mut body_state = header_state.clone();
                    let falls_through = if let Some((body, _)) = do_while_parts {
                        self.analyze_stmts(body, placeholder, &mut body_state)
                    } else {
                        self.analyze_block(block, placeholder, &mut body_state)
                    };
                    let jumps = self.loop_jumps.pop().expect("loop jump state exists");

                    if let Some(breaks) = jumps.breaks {
                        merge_optional_state(&mut exit_state, &breaks);
                    }

                    let mut next_header = before_loop.clone();
                    let mut backedges = None;
                    if falls_through {
                        merge_optional_state(&mut backedges, &body_state);
                    }
                    if let Some(continues) = jumps.continues {
                        merge_optional_state(&mut backedges, &continues);
                    }

                    if let Some((_, condition)) = do_while_parts {
                        if let Some(mut condition_state) = backedges
                            && self.analyze_expr(condition, &mut condition_state)
                        {
                            let mut continue_state = condition_state.clone();
                            self.remove_failed_send_calls(condition, true, &mut continue_state);
                            next_header.merge(&continue_state);

                            self.remove_failed_send_calls(condition, false, &mut condition_state);
                            merge_optional_state(&mut exit_state, &condition_state);
                        }
                    } else if let Some(backedges) = backedges {
                        next_header.merge(&backedges);
                    }

                    if next_header == header_state {
                        break;
                    }
                    header_state = next_header;
                }

                if let Some(exit_state) = exit_state {
                    *state = exit_state;
                    true
                } else {
                    state.clear();
                    false
                }
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                if !self.analyze_expr(cond, state) {
                    return false;
                }

                let mut then_state = state.clone();
                let mut else_state = state.clone();
                self.remove_failed_send_calls(cond, true, &mut then_state);
                self.remove_failed_send_calls(cond, false, &mut else_state);
                let then_falls_through = self.analyze_stmt(then_stmt, placeholder, &mut then_state);

                let else_falls_through = if let Some(else_stmt) = else_stmt {
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

                then_falls_through || else_falls_through
            }
            StmtKind::Try(try_stmt) => {
                if !self.analyze_expr(&try_stmt.expr, state) {
                    return false;
                }

                let mut merged = FlowState::default();
                let mut any_falls_through = false;
                for clause in try_stmt.clauses {
                    let mut clause_state = state.clone();
                    let falls_through =
                        self.analyze_block(clause.block, placeholder, &mut clause_state);
                    if falls_through {
                        merged.merge(&clause_state);
                        any_falls_through = true;
                    }
                }

                *state = merged;
                any_falls_through
            }
            StmtKind::Placeholder => {
                if let Some((modifiers, index, body)) = placeholder {
                    let function = self.call_returns.last().and_then(|returns| returns.function);
                    self.call_returns.push(CallReturns { function, state: None });
                    let falls_through = self.analyze_modifier_chain(modifiers, index, body, state);
                    let mut completions =
                        self.call_returns.pop().expect("modifier return state exists");
                    if falls_through {
                        if let Some(function) = function
                            && !state
                                .stored_return_results
                                .iter()
                                .any(|result| result.function == function)
                        {
                            let sources = self.named_return_result_sources(function, state);
                            state.set_stored_return_results(function, &sources);
                        }
                        merge_optional_state(&mut completions.state, state);
                    }
                    if let Some(completions) = completions.state {
                        *state = completions;
                        true
                    } else {
                        state.clear();
                        false
                    }
                } else {
                    true
                }
            }
            StmtKind::AssemblyBlock(block) => self.analyze_block(block, placeholder, state),
            StmtKind::Switch(switch) => {
                if !self.analyze_expr(switch.selector, state) {
                    return false;
                }

                let before_switch = state.clone();
                let mut completions = None;
                for case in switch.cases {
                    let mut case_state = before_switch.clone();
                    if self.analyze_block(case.body, placeholder, &mut case_state) {
                        merge_optional_state(&mut completions, &case_state);
                    }
                }
                if !switch.cases.iter().any(|case| case.constant.is_none()) {
                    merge_optional_state(&mut completions, &before_switch);
                }

                if let Some(completions) = completions {
                    *state = completions;
                    true
                } else {
                    state.clear();
                    false
                }
            }
            StmtKind::Err(_) => true,
        }
    }

    fn analyze_expr(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut FlowState) -> bool {
        match &expr.kind {
            ExprKind::Assign(lhs, op, rhs) => {
                let mut completes = true;
                if op.is_some() {
                    completes &= self.analyze_expr(lhs, state);
                }
                completes &= self.analyze_expr(rhs, state);
                completes &= self.analyze_lhs_indices(lhs, state);
                if completes {
                    let assignments = if op.is_none() {
                        self.assignment_send_result_sources(lhs, rhs, state)
                    } else {
                        assigned_variables(lhs)
                            .into_iter()
                            .map(|variable| (variable, Vec::new()))
                            .collect()
                    };
                    for (variable, sources) in assignments {
                        state.set_stored_send_results(variable, &sources);
                    }
                    let written_vars = state_write_lhs_vars(self.hir, lhs);
                    if !written_vars.is_empty()
                        || is_storage_write_lhs(self.gcx, self.hir, lhs, false)
                    {
                        self.emit_pending_calls(state, &written_vars);
                        self.record_state_effect();
                    }
                }
                completes
            }
            ExprKind::Delete(inner) => {
                let completes = self.analyze_lhs_indices(inner, state);
                if completes {
                    for variable in assigned_variables(inner) {
                        state.set_stored_send_results(variable, &[]);
                    }
                    let written_vars = state_write_lhs_vars(self.hir, inner);
                    if !written_vars.is_empty()
                        || is_storage_write_lhs(self.gcx, self.hir, inner, true)
                    {
                        self.emit_pending_calls(state, &written_vars);
                        self.record_state_effect();
                    }
                }
                completes
            }
            ExprKind::Unary(op, inner)
                if matches!(
                    op.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) =>
            {
                let completes = self.analyze_expr(inner, state);
                if completes {
                    let written_vars = state_write_lhs_vars(self.hir, inner);
                    if !written_vars.is_empty()
                        || is_storage_write_lhs(self.gcx, self.hir, inner, true)
                    {
                        self.emit_pending_calls(state, &written_vars);
                        self.record_state_effect();
                    }
                }
                completes
            }
            ExprKind::Unary(_, inner) => self.analyze_expr(inner, state),
            ExprKind::Call(callee, args, opts) => {
                state.clear_stored_call_results(expr.span);
                let mut children =
                    Vec::with_capacity(1 + opts.map_or(0, |opts| opts.args.len()) + args.len());
                children.push(*callee);
                if let Some(opts) = opts {
                    for opt in opts.args {
                        children.push(&opt.value);
                    }
                }
                for arg in args.exprs() {
                    children.push(arg);
                }
                let assertion_condition =
                    is_builtin_assertion(self.gcx, callee).then(|| args.exprs().next()).flatten();
                let mut completes =
                    self.analyze_unordered_exprs(&children, assertion_condition, state);

                if completes {
                    let func_ids =
                        resolved_function_ids(self.gcx, self.hir, callee).collect::<Vec<_>>();
                    if !func_ids.is_empty() {
                        let before_call = state.clone();
                        let mut returned_state = FlowState::default();
                        let mut any_returns = false;
                        for func_id in func_ids {
                            let mut candidate_state = before_call.clone();
                            if let Some(return_sources) =
                                self.analyze_internal_call(func_id, &mut candidate_state)
                            {
                                candidate_state.set_stored_call_results(expr.span, &return_sources);
                                returned_state.merge(&candidate_state);
                                any_returns = true;
                            }
                        }
                        *state = returned_state;
                        completes = any_returns;
                    }
                }

                if completes {
                    if is_state_mutating_array_call(self.gcx, callee)
                        || is_yul_state_effect(self.gcx, callee)
                    {
                        self.emit_pending_stipend_calls(state);
                        self.record_state_effect();
                    }
                    if let Some(kind) = self.reentrant_call_kind(callee, args, *opts) {
                        state.push_call(expr.span, kind);
                    }

                    if is_builtin_assertion(self.gcx, callee)
                        && let Some(condition) = args.exprs().next()
                    {
                        self.remove_failed_send_calls(condition, true, state);
                    }
                    if is_non_returning_builtin_call(self.gcx, callee) {
                        if self.completion_probe_depth > 0
                            && is_successful_halt_builtin_call(self.gcx, callee)
                        {
                            self.completion_probe_successful_halt = true;
                        }
                        state.clear();
                        completes = false;
                    }
                }
                completes
            }
            ExprKind::Binary(lhs, op, rhs) => {
                if matches!(op.kind, BinOpKind::And | BinOpKind::Or) {
                    if !self.analyze_expr(lhs, state) {
                        return false;
                    }

                    let rhs_outcome = op.kind == BinOpKind::And;
                    let mut short_circuit_state = state.clone();
                    self.remove_failed_send_calls(lhs, !rhs_outcome, &mut short_circuit_state);

                    let mut rhs_state = state.clone();
                    self.remove_failed_send_calls(lhs, rhs_outcome, &mut rhs_state);
                    let rhs_completes = self.analyze_expr(rhs, &mut rhs_state);

                    *state = short_circuit_state;
                    if rhs_completes {
                        state.merge(&rhs_state);
                    }
                    true
                } else {
                    self.analyze_unordered_exprs(&[lhs, rhs], None, state)
                }
            }
            ExprKind::Index(base, index) => {
                if let Some(index) = index {
                    self.analyze_unordered_exprs(&[base, index], None, state)
                } else {
                    self.analyze_expr(base, state)
                }
            }
            ExprKind::Slice(base, start, end) => {
                let mut children = Vec::with_capacity(3);
                children.push(*base);
                if let Some(start) = start {
                    children.push(*start);
                }
                if let Some(end) = end {
                    children.push(*end);
                }
                self.analyze_unordered_exprs(&children, None, state)
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                if !self.analyze_expr(cond, state) {
                    return false;
                }

                let mut true_state = state.clone();
                self.remove_failed_send_calls(cond, true, &mut true_state);
                let true_completes = self.analyze_expr(true_expr, &mut true_state);

                let mut false_state = state.clone();
                self.remove_failed_send_calls(cond, false, &mut false_state);
                let false_completes = self.analyze_expr(false_expr, &mut false_state);

                state.clear();
                if true_completes {
                    state.merge(&true_state);
                }
                if false_completes {
                    state.merge(&false_state);
                }
                true_completes || false_completes
            }
            ExprKind::Array(exprs) => {
                let children = exprs.iter().collect::<Vec<_>>();
                self.analyze_unordered_exprs(&children, None, state)
            }
            ExprKind::Tuple(exprs) => {
                let children = exprs.iter().copied().flatten().collect::<Vec<_>>();
                self.analyze_unordered_exprs(&children, None, state)
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => self.analyze_expr(base, state),
            ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => true,
            ExprKind::Ident(reses) => {
                for &res in *reses {
                    if let Res::Item(ItemId::Variable(var_id)) = res
                        && self.hir.variable(var_id).kind.is_state()
                    {
                        state.push_read(var_id);
                    }
                }
                true
            }
            ExprKind::Lit(_) | ExprKind::YulMember(..) | ExprKind::Err(_) => true,
        }
    }

    fn analyze_unordered_exprs(
        &mut self,
        exprs: &[&'hir hir::Expr<'hir>],
        assertion_condition: Option<&'hir hir::Expr<'hir>>,
        state: &mut FlowState,
    ) -> bool {
        if !self.reentrancy_unlimited_gas_enabled {
            let mut all_complete = true;
            for &expr in exprs {
                all_complete &= self.analyze_expr(expr, state);
            }
            return all_complete;
        }

        if self.completion_probe_depth == 0 && !self.unordered_exprs_can_persist(exprs, state) {
            state.clear();
            return false;
        }

        let mut summaries = Vec::with_capacity(exprs.len());
        let mut all_complete = true;
        for &expr in exprs {
            let prior_calls = state
                .pending_calls
                .iter()
                .filter(|call| call.kind == ReentrantCallKind::Stipend)
                .map(|call| call.span)
                .collect::<HashSet<_>>();
            let prior_effects = self.state_effects;
            all_complete &= self.analyze_expr(expr, state);
            let calls = state
                .pending_calls
                .iter()
                .filter(|call| {
                    call.kind == ReentrantCallKind::Stipend && !prior_calls.contains(&call.span)
                })
                .map(|call| call.span)
                .collect::<Vec<_>>();
            summaries.push((calls, self.state_effects != prior_effects));
        }

        for (index, (calls, _)) in summaries.iter().enumerate() {
            if summaries
                .iter()
                .enumerate()
                .any(|(other_index, (_, has_effect))| other_index != index && *has_effect)
            {
                for &span in calls {
                    let call_can_succeed = assertion_condition.is_none_or(|condition| {
                        !condition.span.contains(span)
                            || self.send_can_succeed_for_outcome(condition, span, true, state)
                    });
                    if call_can_succeed {
                        self.emit_stipend_call(span);
                    }
                }
            }
        }
        all_complete
    }

    fn unordered_exprs_can_persist(
        &mut self,
        exprs: &[&'hir hir::Expr<'hir>],
        state: &FlowState,
    ) -> bool {
        let inline_cache = std::mem::replace(
            &mut self.inline_cache,
            HelperAnalysisCache::new(DEFAULT_HELPER_ANALYSIS_CACHE_LIMIT),
        );
        let prior_effects = self.state_effects;
        let prior_successful_halt = self.completion_probe_successful_halt;
        self.completion_probe_depth += 1;
        self.completion_probe_successful_halt = false;

        let mut probe_state = state.clone();
        let mut all_complete = true;
        for &expr in exprs {
            all_complete &= self.analyze_expr(expr, &mut probe_state);
        }

        let successful_halt = self.completion_probe_successful_halt;
        self.completion_probe_successful_halt = prior_successful_halt;
        self.completion_probe_depth -= 1;
        self.state_effects = prior_effects;
        self.inline_cache = inline_cache;
        all_complete || successful_halt
    }

    const fn record_state_effect(&mut self) {
        if self.reentrancy_unlimited_gas_enabled {
            self.state_effects = self.state_effects.saturating_add(1);
        }
    }

    fn remove_failed_send_calls(
        &self,
        condition: &'hir hir::Expr<'hir>,
        outcome: bool,
        state: &mut FlowState,
    ) {
        if !self.reentrancy_unlimited_gas_enabled {
            return;
        }

        let calls = state
            .pending_calls
            .iter()
            .filter(|call| {
                call.kind == ReentrantCallKind::Stipend
                    && self.expr_contains_send_target(condition, call.span, state)
            })
            .map(|call| call.span)
            .collect::<Vec<_>>();
        for span in calls {
            if !self.send_can_succeed_for_outcome(condition, span, outcome, state) {
                state.remove_call(span, ReentrantCallKind::Stipend);
            }
        }
    }

    fn send_can_succeed_for_outcome(
        &self,
        condition: &'hir hir::Expr<'hir>,
        span: Span,
        outcome: bool,
        state: &FlowState,
    ) -> bool {
        self.boolean_outcomes_for_send(condition, span, state).iter().any(|candidate| {
            candidate.value == outcome && candidate.send == SendEvaluation::Succeeded
        })
    }

    fn boolean_outcomes_for_send(
        &self,
        expr: &'hir hir::Expr<'hir>,
        target: Span,
        state: &FlowState,
    ) -> Vec<BooleanOutcome> {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Call(callee, _, _)
                if expr.span == target
                    && self.gcx.builtin_callee(callee.peel_parens().id)
                        == Some(Builtin::AddressPayableSend) =>
            {
                vec![
                    BooleanOutcome { value: true, send: SendEvaluation::Succeeded },
                    BooleanOutcome { value: false, send: SendEvaluation::Failed },
                ]
            }
            ExprKind::Call(..) => {
                let mut outcomes = Vec::new();
                for result in state.stored_call_results.iter().filter(|result| {
                    result.expression_span == expr.span
                        && result.index == 0
                        && result.source.call_span == target
                }) {
                    let (true_send, false_send) = if result.source.succeeds_when_true {
                        (SendEvaluation::Succeeded, SendEvaluation::Failed)
                    } else {
                        (SendEvaluation::Failed, SendEvaluation::Succeeded)
                    };
                    push_unique_boolean_outcome(
                        &mut outcomes,
                        BooleanOutcome { value: true, send: true_send },
                    );
                    push_unique_boolean_outcome(
                        &mut outcomes,
                        BooleanOutcome { value: false, send: false_send },
                    );
                }
                if outcomes.is_empty() {
                    unconstrained_boolean_outcomes(
                        self.expr_contains_send_target(expr, target, state),
                    )
                } else {
                    outcomes
                }
            }
            ExprKind::Lit(lit) => match lit.kind {
                LitKind::Bool(value) => {
                    vec![BooleanOutcome { value, send: SendEvaluation::NotEvaluated }]
                }
                _ => unconstrained_boolean_outcomes(expr.span.contains(target)),
            },
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => self
                .boolean_outcomes_for_send(inner, target, state)
                .into_iter()
                .map(|outcome| BooleanOutcome { value: !outcome.value, ..outcome })
                .collect(),
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                let lhs_outcomes = self.boolean_outcomes_for_send(lhs, target, state);
                let rhs_outcomes = self.boolean_outcomes_for_send(rhs, target, state);
                let mut outcomes = Vec::new();
                for lhs_outcome in lhs_outcomes {
                    let short_circuits = if op.kind == BinOpKind::And {
                        !lhs_outcome.value
                    } else {
                        lhs_outcome.value
                    };
                    if short_circuits {
                        push_unique_boolean_outcome(&mut outcomes, lhs_outcome);
                    } else {
                        for rhs_outcome in &rhs_outcomes {
                            let Some(send) =
                                merge_send_evaluations(lhs_outcome.send, rhs_outcome.send)
                            else {
                                continue;
                            };
                            push_unique_boolean_outcome(
                                &mut outcomes,
                                BooleanOutcome { value: rhs_outcome.value, send },
                            );
                        }
                    }
                }
                outcomes
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) => {
                let lhs_outcomes = self.boolean_outcomes_for_send(lhs, target, state);
                let rhs_outcomes = self.boolean_outcomes_for_send(rhs, target, state);
                let mut outcomes = Vec::new();
                for lhs_outcome in lhs_outcomes {
                    for rhs_outcome in &rhs_outcomes {
                        let equal = lhs_outcome.value == rhs_outcome.value;
                        let Some(send) = merge_send_evaluations(lhs_outcome.send, rhs_outcome.send)
                        else {
                            continue;
                        };
                        push_unique_boolean_outcome(
                            &mut outcomes,
                            BooleanOutcome {
                                value: if op.kind == BinOpKind::Eq { equal } else { !equal },
                                send,
                            },
                        );
                    }
                }
                outcomes
            }
            ExprKind::Ternary(cond, true_expr, false_expr) => {
                let condition_outcomes = self.boolean_outcomes_for_send(cond, target, state);
                let true_outcomes = self.boolean_outcomes_for_send(true_expr, target, state);
                let false_outcomes = self.boolean_outcomes_for_send(false_expr, target, state);
                let mut outcomes = Vec::new();
                for condition_outcome in condition_outcomes {
                    let branch_outcomes =
                        if condition_outcome.value { &true_outcomes } else { &false_outcomes };
                    for branch_outcome in branch_outcomes {
                        let Some(send) =
                            merge_send_evaluations(condition_outcome.send, branch_outcome.send)
                        else {
                            continue;
                        };
                        push_unique_boolean_outcome(
                            &mut outcomes,
                            BooleanOutcome { value: branch_outcome.value, send },
                        );
                    }
                }
                outcomes
            }
            ExprKind::Ident(reses) => {
                let mut outcomes = Vec::new();
                for variable in reses.iter().filter_map(|res| res.as_variable()) {
                    for result in state.stored_send_results.iter().filter(|result| {
                        result.variable == variable && result.source.call_span == target
                    }) {
                        let (true_send, false_send) = if result.source.succeeds_when_true {
                            (SendEvaluation::Succeeded, SendEvaluation::Failed)
                        } else {
                            (SendEvaluation::Failed, SendEvaluation::Succeeded)
                        };
                        push_unique_boolean_outcome(
                            &mut outcomes,
                            BooleanOutcome { value: true, send: true_send },
                        );
                        push_unique_boolean_outcome(
                            &mut outcomes,
                            BooleanOutcome { value: false, send: false_send },
                        );
                    }
                }
                if outcomes.is_empty() { unconstrained_boolean_outcomes(false) } else { outcomes }
            }
            _ => {
                unconstrained_boolean_outcomes(self.expr_contains_send_target(expr, target, state))
            }
        }
    }

    fn send_result_sources_by_index(
        &self,
        expr: &'hir hir::Expr<'hir>,
        result_count: usize,
        state: &FlowState,
    ) -> Vec<Vec<SendResultSource>> {
        if result_count == 1 {
            return vec![self.send_result_sources(expr, state)];
        }

        let mut sources = vec![Vec::new(); result_count];
        match &expr.peel_parens().kind {
            ExprKind::Tuple(exprs) => {
                for (index, expr) in exprs.iter().take(result_count).enumerate() {
                    if let Some(expr) = expr {
                        sources[index] = self.send_result_sources(expr, state);
                    }
                }
            }
            ExprKind::Call(..) => {
                for result in state.stored_call_results.iter().filter(|result| {
                    result.expression_span == expr.span && result.index < result_count
                }) {
                    if !sources[result.index].contains(&result.source) {
                        sources[result.index].push(result.source);
                    }
                }
            }
            _ => {}
        }
        sources
    }

    fn assignment_send_result_sources(
        &self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        state: &FlowState,
    ) -> Vec<(VariableId, Vec<SendResultSource>)> {
        let mut assignments = Vec::new();
        self.collect_assignment_send_result_sources(lhs, rhs, state, &mut assignments);
        assignments
    }

    fn collect_assignment_send_result_sources(
        &self,
        lhs: &'hir hir::Expr<'hir>,
        rhs: &'hir hir::Expr<'hir>,
        state: &FlowState,
        assignments: &mut Vec<(VariableId, Vec<SendResultSource>)>,
    ) {
        let ExprKind::Tuple(lhs_exprs) = &lhs.peel_parens().kind else {
            for variable in assigned_variables(lhs) {
                let sources = if assigned_variable(lhs) == Some(variable) {
                    self.send_result_sources(rhs, state)
                } else {
                    Vec::new()
                };
                assignments.push((variable, sources));
            }
            return;
        };

        if let ExprKind::Tuple(rhs_exprs) = &rhs.peel_parens().kind {
            for index in (0..lhs_exprs.len()).rev() {
                if let Some(lhs) = lhs_exprs[index]
                    && let Some(rhs) = rhs_exprs.get(index).copied().flatten()
                {
                    self.collect_assignment_send_result_sources(lhs, rhs, state, assignments);
                } else if let Some(lhs) = lhs_exprs[index] {
                    assignments.extend(
                        assigned_variables(lhs).into_iter().map(|variable| (variable, Vec::new())),
                    );
                }
            }
            return;
        }

        let sources = self.send_result_sources_by_index(rhs, lhs_exprs.len(), state);
        for (index, lhs) in lhs_exprs.iter().enumerate().rev() {
            if let Some(lhs) = lhs {
                if let Some(variable) = assigned_variable(lhs) {
                    assignments.push((variable, sources[index].clone()));
                } else {
                    assignments.extend(
                        assigned_variables(lhs).into_iter().map(|variable| (variable, Vec::new())),
                    );
                }
            }
        }
    }

    fn named_return_result_sources(
        &self,
        function: FunctionId,
        state: &FlowState,
    ) -> Vec<Vec<SendResultSource>> {
        self.hir
            .function(function)
            .returns
            .iter()
            .map(|variable| {
                state
                    .stored_send_results
                    .iter()
                    .filter(|result| result.variable == *variable)
                    .map(|result| result.source)
                    .collect()
            })
            .collect()
    }

    fn return_result_sources(
        &self,
        function: FunctionId,
        expr: Option<&'hir hir::Expr<'hir>>,
        state: &FlowState,
    ) -> Vec<Vec<SendResultSource>> {
        if let Some(expr) = expr {
            self.send_result_sources_by_index(
                expr,
                self.hir.function(function).returns.len(),
                state,
            )
        } else {
            self.named_return_result_sources(function, state)
        }
    }

    fn send_result_sources(
        &self,
        expr: &'hir hir::Expr<'hir>,
        state: &FlowState,
    ) -> Vec<SendResultSource> {
        let expr = expr.peel_parens();
        match &expr.kind {
            ExprKind::Call(callee, _, _)
                if self.gcx.builtin_callee(callee.peel_parens().id)
                    == Some(Builtin::AddressPayableSend) =>
            {
                vec![SendResultSource { call_span: expr.span, succeeds_when_true: true }]
            }
            ExprKind::Call(..) => state
                .stored_call_results
                .iter()
                .filter(|result| result.expression_span == expr.span && result.index == 0)
                .map(|result| result.source)
                .collect(),
            ExprKind::Ident(reses) => {
                let mut sources = Vec::new();
                for variable in reses.iter().filter_map(|res| res.as_variable()) {
                    for result in state
                        .stored_send_results
                        .iter()
                        .filter(|result| result.variable == variable)
                    {
                        if !sources.contains(&result.source) {
                            sources.push(result.source);
                        }
                    }
                }
                sources
            }
            ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => self
                .send_result_sources(inner, state)
                .into_iter()
                .map(|source| SendResultSource {
                    succeeds_when_true: !source.succeeds_when_true,
                    ..source
                })
                .collect(),
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
                self.path_sensitive_send_result_sources(expr, [lhs, rhs], state)
            }
            ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) => {
                let (source_expr, literal) = if let Some(literal) = bool_literal(rhs) {
                    (lhs, literal)
                } else if let Some(literal) = bool_literal(lhs) {
                    (rhs, literal)
                } else {
                    return Vec::new();
                };
                let invert = (op.kind == BinOpKind::Eq) != literal;
                self.send_result_sources(source_expr, state)
                    .into_iter()
                    .map(|source| SendResultSource {
                        succeeds_when_true: source.succeeds_when_true != invert,
                        ..source
                    })
                    .collect()
            }
            ExprKind::Ternary(condition, true_expr, false_expr) => self
                .path_sensitive_send_result_sources(
                    expr,
                    [condition, true_expr, false_expr],
                    state,
                ),
            _ => Vec::new(),
        }
    }

    fn path_sensitive_send_result_sources<const N: usize>(
        &self,
        expr: &'hir hir::Expr<'hir>,
        children: [&'hir hir::Expr<'hir>; N],
        state: &FlowState,
    ) -> Vec<SendResultSource> {
        let mut candidates = Vec::new();
        for child in children {
            for source in self.send_result_sources(child, state) {
                if !candidates.contains(&source) {
                    candidates.push(source);
                }
            }
        }

        candidates
            .into_iter()
            .filter_map(|source| {
                let mut succeeds_when = None;
                for outcome in self.boolean_outcomes_for_send(expr, source.call_span, state) {
                    if outcome.send != SendEvaluation::Succeeded {
                        continue;
                    }
                    if succeeds_when.is_some_and(|value| value != outcome.value) {
                        return None;
                    }
                    succeeds_when = Some(outcome.value);
                }
                succeeds_when
                    .map(|succeeds_when_true| SendResultSource { succeeds_when_true, ..source })
            })
            .collect()
    }

    fn expr_contains_send_target(
        &self,
        expr: &'hir hir::Expr<'hir>,
        target: Span,
        state: &FlowState,
    ) -> bool {
        expr.span.contains(target)
            || state.stored_send_results.iter().any(|result| {
                result.source.call_span == target && expr_references_variable(expr, result.variable)
            })
            || state.stored_call_results.iter().any(|result| {
                result.source.call_span == target && expr.span.contains(result.expression_span)
            })
    }

    fn analyze_internal_call(
        &mut self,
        func_id: FunctionId,
        state: &mut FlowState,
    ) -> Option<Vec<Vec<SendResultSource>>> {
        if self.call_stack.contains(&func_id) {
            return Some(vec![Vec::new(); self.hir.function(func_id).returns.len()]);
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return Some(vec![Vec::new(); func.returns.len()]) };

        let key = InlineCallKey {
            func_id,
            recursive_cut: self.first_recursive_cut(func_id),
            state: state.clone(),
        };
        if self.inline_cache.is_in_progress(&key) {
            return Some(vec![Vec::new(); func.returns.len()]);
        }
        if let Some(cached) = self.inline_cache.get(&key) {
            *state = cached.state.clone();
            let may_return = cached.may_return;
            let return_sources = cached.return_sources.clone();
            if cached.has_state_effect {
                self.record_state_effect();
            }
            return may_return.then_some(return_sources);
        }

        let mut after = state.clone();
        after.clear_stored_return_results(func_id);
        let prior_effects = self.state_effects;
        self.inline_cache.start(key.clone());
        self.call_stack.push(func_id);
        self.call_returns.push(CallReturns { function: Some(func_id), state: None });
        let falls_through = self.analyze_callable(func, body, &mut after);
        let mut returned_state = self.call_returns.pop().expect("call return state exists").state;
        self.call_stack.pop();

        if falls_through {
            if !after.stored_return_results.iter().any(|result| result.function == func_id) {
                let sources = self.named_return_result_sources(func_id, &after);
                after.set_stored_return_results(func_id, &sources);
            }
            merge_optional_state(&mut returned_state, &after);
        }
        let may_return = returned_state.is_some();
        after = returned_state.unwrap_or_default();
        let return_sources = (0..func.returns.len())
            .map(|index| {
                after
                    .stored_return_results
                    .iter()
                    .filter(|result| result.function == func_id && result.index == index)
                    .map(|result| result.source)
                    .collect()
            })
            .collect::<Vec<_>>();
        after.clear_stored_return_results(func_id);

        self.inline_cache.finish(
            key,
            InlineCallResult {
                state: after.clone(),
                may_return,
                has_state_effect: self.state_effects != prior_effects,
                return_sources: return_sources.clone(),
            },
        );
        *state = after;
        may_return.then_some(return_sources)
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
            StmtKind::Block(block)
            | StmtKind::UncheckedBlock(block)
            | StmtKind::AssemblyBlock(block)
            | StmtKind::Loop(block, _) => {
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
            StmtKind::Switch(switch) => {
                self.collect_direct_internal_calls_expr(switch.selector, calls);
                for case in switch.cases {
                    self.collect_direct_internal_calls_block(case.body, calls);
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => {}
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
                for func_id in resolved_function_ids(self.gcx, self.hir, callee) {
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

    fn analyze_lhs_indices(&mut self, expr: &'hir hir::Expr<'hir>, state: &mut FlowState) -> bool {
        match &expr.kind {
            ExprKind::Index(base, index) => {
                let mut completes = self.analyze_lhs_indices(base, state);
                if let Some(index) = index {
                    completes &= self.analyze_expr(index, state);
                }
                completes
            }
            ExprKind::Slice(base, start, end) => {
                let mut completes = self.analyze_lhs_indices(base, state);
                if let Some(start) = start {
                    completes &= self.analyze_expr(start, state);
                }
                if let Some(end) = end {
                    completes &= self.analyze_expr(end, state);
                }
                completes
            }
            ExprKind::Member(base, _) | ExprKind::Payable(base) => {
                self.analyze_lhs_indices(base, state)
            }
            ExprKind::Tuple(exprs) => {
                let mut completes = true;
                for expr in exprs.iter().copied().flatten() {
                    completes &= self.analyze_lhs_indices(expr, state);
                }
                completes
            }
            ExprKind::Call(..) => self.analyze_expr(expr, state),
            _ => true,
        }
    }

    fn emit_pending_calls(&mut self, state: &FlowState, written_vars: &[VariableId]) {
        if self.completion_probe_depth > 0 {
            return;
        }
        self.emit_pending_stipend_calls(state);

        for call in &state.pending_calls {
            let (lint, msg_prefix) = match call.kind {
                ReentrantCallKind::Eth => {
                    (&REENTRANCY_ETH, "uncapped ETH transfer can be reentered before")
                }
                ReentrantCallKind::NoEth => {
                    (&REENTRANCY_NO_ETH, "external call can be reentered before")
                }
                ReentrantCallKind::Stipend => continue,
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

    fn emit_pending_stipend_calls(&mut self, state: &FlowState) {
        if !self.reentrancy_unlimited_gas_enabled {
            return;
        }

        for call in &state.pending_calls {
            if call.kind == ReentrantCallKind::Stipend {
                self.emit_stipend_call(call.span);
            }
        }
    }

    fn emit_stipend_call(&mut self, span: Span) {
        if self.completion_probe_depth == 0
            && self.reentrancy_unlimited_gas_enabled
            && self.emitted.insert(span)
        {
            self.ctx.emit(&REENTRANCY_UNLIMITED_GAS, span);
        }
    }

    fn reentrant_call_kind(
        &self,
        callee: &'hir hir::Expr<'hir>,
        args: &CallArgs<'hir>,
        opts: Option<&hir::CallOptions<'hir>>,
    ) -> Option<ReentrantCallKind> {
        if self.reentrancy_unlimited_gas_enabled && is_stipend_value_call(self.gcx, callee) {
            return Some(ReentrantCallKind::Stipend);
        }
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
}

fn assigned_variable(expr: &hir::Expr<'_>) -> Option<VariableId> {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return None };
    unique(reses.iter().filter_map(|res| res.as_variable()))
}

fn assigned_variables(expr: &hir::Expr<'_>) -> Vec<VariableId> {
    fn collect(expr: &hir::Expr<'_>, variables: &mut Vec<VariableId>) {
        match &expr.peel_parens().kind {
            ExprKind::Ident(reses) => {
                if let Some(variable) = unique(reses.iter().filter_map(|res| res.as_variable()))
                    && !variables.contains(&variable)
                {
                    variables.push(variable);
                }
            }
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().flatten() {
                    collect(expr, variables);
                }
            }
            _ => {}
        }
    }

    let mut variables = Vec::new();
    collect(expr, &mut variables);
    variables
}

fn bool_literal(expr: &hir::Expr<'_>) -> Option<bool> {
    let ExprKind::Lit(lit) = &expr.peel_parens().kind else { return None };
    let LitKind::Bool(value) = lit.kind else { return None };
    Some(value)
}

fn expr_references_variable(expr: &hir::Expr<'_>, variable: VariableId) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_references_variable(lhs, variable) || expr_references_variable(rhs, variable)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => expr_references_variable(inner, variable),
        ExprKind::Call(callee, args, opts) => {
            expr_references_variable(callee, variable)
                || opts.is_some_and(|opts| {
                    opts.args.iter().any(|opt| expr_references_variable(&opt.value, variable))
                })
                || args.exprs().any(|arg| expr_references_variable(arg, variable))
        }
        ExprKind::Index(base, index) => {
            expr_references_variable(base, variable)
                || index.is_some_and(|index| expr_references_variable(index, variable))
        }
        ExprKind::Slice(base, start, end) => {
            expr_references_variable(base, variable)
                || start.is_some_and(|start| expr_references_variable(start, variable))
                || end.is_some_and(|end| expr_references_variable(end, variable))
        }
        ExprKind::Ternary(condition, true_expr, false_expr) => {
            expr_references_variable(condition, variable)
                || expr_references_variable(true_expr, variable)
                || expr_references_variable(false_expr, variable)
        }
        ExprKind::Array(exprs) => exprs.iter().any(|expr| expr_references_variable(expr, variable)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_references_variable(expr, variable))
        }
        ExprKind::Ident(reses) => reses.iter().any(|res| res.as_variable() == Some(variable)),
        ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::YulMember(..)
        | ExprKind::Err(_) => false,
    }
}

fn is_stipend_value_call<'hir>(gcx: Gcx<'hir>, callee: &'hir hir::Expr<'hir>) -> bool {
    matches!(
        gcx.builtin_callee(callee.peel_parens().id),
        Some(Builtin::AddressPayableTransfer | Builtin::AddressPayableSend)
    )
}

impl FlowState {
    fn clear(&mut self) {
        self.state_reads.clear();
        self.pending_calls.clear();
        self.stored_send_results.clear();
        self.stored_call_results.clear();
        self.stored_return_results.clear();
    }

    fn merge(&mut self, other: &Self) {
        self.state_reads.extend(other.state_reads.iter().copied());
        let self_stipend_calls = self
            .pending_calls
            .iter()
            .filter(|call| call.kind == ReentrantCallKind::Stipend)
            .map(|call| call.span)
            .collect::<HashSet<_>>();
        let other_stipend_calls = other
            .pending_calls
            .iter()
            .filter(|call| call.kind == ReentrantCallKind::Stipend)
            .map(|call| call.span)
            .collect::<HashSet<_>>();
        let stored_send_results = merge_send_correlations(
            &self.stored_send_results,
            &other.stored_send_results,
            &self_stipend_calls,
            &other_stipend_calls,
            |result| result.source,
        );
        let stored_call_results = merge_send_correlations(
            &self.stored_call_results,
            &other.stored_call_results,
            &self_stipend_calls,
            &other_stipend_calls,
            |result| result.source,
        );
        let stored_return_results = merge_send_correlations(
            &self.stored_return_results,
            &other.stored_return_results,
            &self_stipend_calls,
            &other_stipend_calls,
            |result| result.source,
        );
        for call in &other.pending_calls {
            if let Some(existing) = self
                .pending_calls
                .iter_mut()
                .find(|existing| existing.span == call.span && existing.kind == call.kind)
            {
                existing.state_reads.extend(call.state_reads.iter().copied());
                existing.result_correlatable &= call.result_correlatable;
            } else {
                self.pending_calls.push(call.clone());
            }
        }
        self.stored_send_results = stored_send_results;
        self.stored_call_results = stored_call_results;
        self.stored_return_results = stored_return_results;
    }
}

fn merge_send_correlations<T: Copy + PartialEq>(
    lhs: &[T],
    rhs: &[T],
    lhs_calls: &HashSet<Span>,
    rhs_calls: &HashSet<Span>,
    source: impl Fn(&T) -> SendResultSource,
) -> Vec<T> {
    let mut merged = Vec::new();
    for &result in lhs.iter().chain(rhs) {
        let source = source(&result);
        let valid_on_lhs = !lhs_calls.contains(&source.call_span) || lhs.contains(&result);
        let valid_on_rhs = !rhs_calls.contains(&source.call_span) || rhs.contains(&result);
        if valid_on_lhs && valid_on_rhs && !merged.contains(&result) {
            merged.push(result);
        }
    }
    merged
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
    args: &CallArgs<'hir>,
    opts: Option<&hir::CallOptions<'hir>>,
) -> bool {
    if call_sends_eth(hir, opts) {
        return false;
    }

    match &callee.peel_parens().kind {
        ExprKind::Member(receiver, member) => match member.name {
            kw::Call | kw::Callcode | kw::Delegatecall => is_address_like(gcx, hir, receiver),
            kw::Staticcall => false,
            _ => external_member_call_can_reenter(gcx, hir, receiver, member.name, args),
        },
        _ => external_function_pointer_can_reenter(gcx, hir, callee, args),
    }
}

fn call_sends_eth(hir: &hir::Hir<'_>, opts: Option<&hir::CallOptions<'_>>) -> bool {
    opts.is_some_and(|opts| {
        opts.args.iter().any(|opt| opt.name.name == sym::value && !is_zero_value(hir, &opt.value))
    })
}

fn external_member_call_can_reenter<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    receiver: &'hir hir::Expr<'hir>,
    member: solar::interface::Symbol,
    args: &CallArgs<'hir>,
) -> bool {
    if is_super(receiver) {
        return false;
    }

    let Some(receiver_ty) = expr_ty(gcx, hir, receiver) else { return false };
    gcx.members_of(receiver_ty, base_item_source(hir, receiver), base_contract(hir, receiver))
        .filter(|candidate| candidate.name == member)
        .any(|candidate| match (candidate.res, candidate.ty.kind) {
            (Some(Res::Item(ItemId::Function(function_id))), _) => {
                let function = hir.function(function_id);
                is_externally_callable(function)
                    && args_match_function(gcx, hir, args, function.parameters)
                    && function.mutates_state()
            }
            (_, TyKind::Fn(function)) => {
                is_externally_callable_fn_kind(function.kind)
                    && args_match_types(gcx, hir, args, function.parameters)
                    && !matches!(
                        function.state_mutability,
                        StateMutability::Pure | StateMutability::View
                    )
            }
            _ => false,
        })
}

fn external_function_pointer_can_reenter<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
    args: &CallArgs<'hir>,
) -> bool {
    let Some(ty) = expr_ty(gcx, hir, callee) else { return false };
    let TyKind::Fn(function) = ty.kind else { return false };
    function.kind == TyFnKind::External
        && args_match_types(gcx, hir, args, function.parameters)
        && !matches!(function.state_mutability, StateMutability::Pure | StateMutability::View)
}

const fn is_externally_callable(func: &hir::Function<'_>) -> bool {
    matches!(func.visibility, Visibility::Public | Visibility::External)
}

const fn is_externally_callable_fn_kind(kind: TyFnKind) -> bool {
    matches!(kind, TyFnKind::External | TyFnKind::Declaration | TyFnKind::DelegateCall)
}

fn args_match_function<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    args: &CallArgs<'hir>,
    params: &'hir [VariableId],
) -> bool {
    if args.len() != params.len() {
        return false;
    }

    match args.kind {
        CallArgsKind::Unnamed(exprs) => exprs.iter().zip(params).all(|(arg, &param)| {
            let param = hir.variable(param);
            let param_ty =
                gcx.type_of_hir_ty(&param.ty).with_loc_if_ref_opt(gcx, param.data_location);
            arg_matches_type(gcx, hir, arg, param_ty)
        }),
        CallArgsKind::Named(named_args) => named_args.iter().all(|arg| {
            params
                .iter()
                .copied()
                .find(|&param| {
                    hir.variable(param).name.is_some_and(|name| name.name == arg.name.name)
                })
                .is_some_and(|param| {
                    let param = hir.variable(param);
                    let param_ty =
                        gcx.type_of_hir_ty(&param.ty).with_loc_if_ref_opt(gcx, param.data_location);
                    arg_matches_type(gcx, hir, &arg.value, param_ty)
                })
        }),
    }
}

fn args_match_types<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    args: &CallArgs<'hir>,
    params: &'hir [Ty<'hir>],
) -> bool {
    if args.len() != params.len() {
        return false;
    }

    match args.kind {
        CallArgsKind::Unnamed(exprs) => {
            exprs.iter().zip(params).all(|(arg, &param)| arg_matches_type(gcx, hir, arg, param))
        }
        CallArgsKind::Named(_) => false,
    }
}

fn arg_matches_type<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    arg: &'hir hir::Expr<'hir>,
    param_ty: Ty<'hir>,
) -> bool {
    expr_ty(gcx, hir, arg).is_some_and(|arg_ty| arg_ty.convert_implicit_to(param_ty, gcx))
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

fn resolved_function_ids<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
) -> impl Iterator<Item = FunctionId> + use<'hir> {
    let resolved =
        gcx.resolved_callee(callee.peel_parens().id).and_then(|resolved| match resolved.res {
            Res::Item(ItemId::Function(func_id))
                if is_internal_function_dispatch(hir, callee, func_id) =>
            {
                Some(func_id)
            }
            _ => None,
        });
    let reses = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => *reses,
        _ => &[],
    };
    resolved.into_iter().chain(reses.iter().filter_map(move |res| match res {
        Res::Item(ItemId::Function(func_id)) if resolved.is_none() => Some(*func_id),
        _ => None,
    }))
}

fn is_internal_function_dispatch(
    hir: &hir::Hir<'_>,
    callee: &hir::Expr<'_>,
    func_id: FunctionId,
) -> bool {
    match &callee.peel_parens().kind {
        ExprKind::Ident(_) => true,
        ExprKind::Member(receiver, _) => {
            is_super(receiver)
                || matches!(
                    hir.function(func_id).visibility,
                    Visibility::Internal | Visibility::Private
                )
        }
        _ => false,
    }
}

fn state_write_lhs_vars(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Vec<VariableId> {
    let mut vars = Vec::new();
    collect_state_write_lhs_vars(hir, expr, &mut vars);
    vars
}

fn is_storage_write_lhs(
    gcx: Gcx<'_>,
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
    storage_ref_is_dereferenced: bool,
) -> bool {
    match &expr.kind {
        ExprKind::Ident(reses) => reses.iter().any(|res| {
            matches!(
                res,
                Res::Item(ItemId::Variable(var_id))
                    if hir.variable(*var_id).kind.is_state()
                        || (storage_ref_is_dereferenced
                            && hir.variable(*var_id).data_location
                                == Some(DataLocation::Storage))
            )
        }),
        ExprKind::Index(base, _) | ExprKind::Slice(base, ..) | ExprKind::Member(base, _) => {
            is_storage_write_lhs(gcx, hir, base, true)
        }
        ExprKind::Payable(base) | ExprKind::Unary(_, base) | ExprKind::Delete(base) => {
            is_storage_write_lhs(gcx, hir, base, storage_ref_is_dereferenced)
        }
        ExprKind::Call(callee, _, _) => {
            is_state_mutating_array_call(gcx, callee)
                || (storage_ref_is_dereferenced
                    && gcx
                        .type_of_expr(expr.peel_parens().id)
                        .is_some_and(|ty| ty.loc() == Some(DataLocation::Storage)))
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().copied().flatten().any(|expr| is_storage_write_lhs(gcx, hir, expr, false))
        }
        _ => false,
    }
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
