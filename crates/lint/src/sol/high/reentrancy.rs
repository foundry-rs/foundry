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
        hir::{
            self, CallArgs, CallArgsKind, ExprKind, FunctionId, ItemId, Res, StmtKind, VariableId,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PendingCall {
    span: Span,
    kind: ReentrantCallKind,
    state_reads: BTreeSet<VariableId>,
}

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
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    emitted: HashSet<Span>,
    call_stack: Vec<FunctionId>,
    inline_cache: HelperAnalysisCache<InlineCallKey, FlowState>,
    recursive_cut_frontiers: HashMap<RecursiveFrontierKey, Vec<FunctionId>>,
    direct_internal_calls: HashMap<FunctionId, Vec<FunctionId>>,
    reentrancy_eth_enabled: bool,
    reentrancy_no_eth_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InlineCallKey {
    func_id: FunctionId,
    /// First active function that can cut recursion from this callee.
    recursive_cut: Option<FunctionId>,
    state: FlowState,
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
            reentrancy_eth_enabled: ctx.is_lint_enabled(REENTRANCY_ETH.id),
            reentrancy_no_eth_enabled: ctx.is_lint_enabled(REENTRANCY_NO_ETH.id),
        }
    }

    const fn has_enabled_lints(&self) -> bool {
        self.reentrancy_eth_enabled || self.reentrancy_no_eth_enabled
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
        placeholder: Option<(&'hir [hir::Modifier<'hir>], usize, hir::Block<'hir>)>,
        state: &mut FlowState,
    ) -> bool {
        match stmt.kind {
            StmtKind::DeclSingle(var_id) => {
                if let Some(init) = self.hir.variable(var_id).initializer {
                    self.analyze_expr(init, state);
                }
                true
            }
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) => {
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
                false
            }
            StmtKind::Break | StmtKind::Continue => false,
            StmtKind::Loop(block, _) => {
                let before_loop = state.clone();
                let mut body_state = state.clone();
                self.analyze_block(block, placeholder, &mut body_state);
                state.clear();
                state.merge(&before_loop);
                state.merge(&body_state);
                true
            }
            StmtKind::If(cond, then_stmt, else_stmt) => {
                self.analyze_expr(cond, state);

                let mut then_state = state.clone();
                let then_falls_through = self.analyze_stmt(then_stmt, placeholder, &mut then_state);

                let mut else_state = state.clone();
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
                self.analyze_expr(&try_stmt.expr, state);

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
                    self.analyze_modifier_chain(modifiers, index, body, state)
                } else {
                    true
                }
            }
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => true,
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
                }
            }
            ExprKind::Delete(inner) => {
                self.analyze_lhs_indices(inner, state);
                let written_vars = state_write_lhs_vars(self.hir, inner);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
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
                }
            }
            ExprKind::Unary(_, inner) => {
                self.analyze_expr(inner, state);
            }
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

                for func_id in resolved_function_ids(callee) {
                    self.analyze_internal_call(func_id, state);
                }
                if !state.state_reads.is_empty()
                    && let Some(kind) = self.reentrant_call_kind(callee, args, *opts)
                {
                    state.push_call(expr.span, kind);
                }
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
                self.analyze_expr(true_expr, &mut true_state);

                let mut false_state = state.clone();
                self.analyze_expr(false_expr, &mut false_state);

                state.clear();
                state.merge(&true_state);
                state.merge(&false_state);
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

    fn analyze_internal_call(&mut self, func_id: FunctionId, state: &mut FlowState) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        let key = InlineCallKey {
            func_id,
            recursive_cut: self.first_recursive_cut(func_id),
            state: state.clone(),
        };
        if self.inline_cache.is_in_progress(&key) {
            return;
        }
        if let Some(cached) = self.inline_cache.get(&key) {
            *state = cached.clone();
            return;
        }

        let mut after = state.clone();
        self.inline_cache.start(key.clone());
        self.call_stack.push(func_id);
        self.analyze_callable(func, body, &mut after);
        self.call_stack.pop();

        self.inline_cache.finish(key, after.clone());
        *state = after;
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
                for func_id in resolved_function_ids(callee) {
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
}

impl FlowState {
    fn clear(&mut self) {
        self.state_reads.clear();
        self.pending_calls.clear();
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
    callee: &'hir hir::Expr<'hir>,
) -> impl Iterator<Item = FunctionId> + 'hir {
    let reses = match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => *reses,
        _ => &[],
    };
    reses.iter().filter_map(|res| match res {
        Res::Item(ItemId::Function(func_id)) => Some(*func_id),
        _ => None,
    })
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
