use super::ReentrancyUnlimitedGas;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{LitKind, StateMutability, UnOpKind, Visibility},
    interface::{Span, kw, sym},
    sema::hir::{self, ExprKind, FunctionId, ItemId, Res, StmtKind, VariableId},
};
use std::collections::{BTreeSet, HashSet};

declare_forge_lint!(
    REENTRANCY_UNLIMITED_GAS,
    Severity::High,
    "reentrancy-unlimited-gas",
    "state read before uncapped ETH transfer is written after the transfer"
);

impl<'hir> LateLintPass<'hir> for ReentrancyUnlimitedGas {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !is_entry_point(func) {
            return;
        }

        let Some(body) = func.body else { return };

        let mut analyzer = Analyzer::new(ctx, hir);
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

#[derive(Clone, Debug, Default)]
struct FlowState {
    state_reads: BTreeSet<VariableId>,
    pending_value_calls: Vec<PendingValueCall>,
}

#[derive(Clone, Debug)]
struct PendingValueCall {
    span: Span,
    state_reads: BTreeSet<VariableId>,
}

impl FlowState {
    fn push_read(&mut self, var_id: VariableId) {
        self.state_reads.insert(var_id);
    }

    fn push_call(&mut self, span: Span) {
        if self.state_reads.is_empty() {
            return;
        }

        if let Some(existing) = self.pending_value_calls.iter_mut().find(|call| call.span == span) {
            existing.state_reads.extend(self.state_reads.iter().copied());
        } else {
            self.pending_value_calls
                .push(PendingValueCall { span, state_reads: self.state_reads.clone() });
        }
    }
}

struct Analyzer<'ctx, 's, 'c, 'hir> {
    ctx: &'ctx LintContext<'s, 'c>,
    hir: &'hir hir::Hir<'hir>,
    emitted: HashSet<Span>,
    call_stack: Vec<FunctionId>,
}

impl<'ctx, 's, 'c, 'hir> Analyzer<'ctx, 's, 'c, 'hir> {
    fn new(ctx: &'ctx LintContext<'s, 'c>, hir: &'hir hir::Hir<'hir>) -> Self {
        Self { ctx, hir, emitted: HashSet::new(), call_stack: Vec::new() }
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
                let written_vars = state_write_lhs_vars(self.hir, lhs);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
                }
                self.analyze_lhs_indices(lhs, state);
            }
            ExprKind::Delete(inner) => {
                let written_vars = state_write_lhs_vars(self.hir, inner);
                if !written_vars.is_empty() {
                    self.emit_pending_calls(state, &written_vars);
                }
                self.analyze_lhs_indices(inner, state);
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
                    for opt in *opts {
                        self.analyze_expr(&opt.value, state);
                    }
                }
                for arg in args.exprs() {
                    self.analyze_expr(arg, state);
                }

                for func_id in resolved_function_ids(callee) {
                    self.analyze_internal_call(func_id, state);
                }
                if is_uncapped_value_call(callee, *opts) {
                    state.push_call(expr.span);
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
            ExprKind::Lit(_) | ExprKind::Err(_) => {}
        }
    }

    fn analyze_internal_call(&mut self, func_id: FunctionId, state: &mut FlowState) {
        if self.call_stack.contains(&func_id) {
            return;
        }

        let func = self.hir.function(func_id);
        let Some(body) = func.body else { return };

        self.call_stack.push(func_id);
        self.analyze_callable(func, body, state);
        self.call_stack.pop();
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
        for call in &state.pending_value_calls {
            if self.emitted.contains(&call.span) {
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
                    &REENTRANCY_UNLIMITED_GAS,
                    call.span,
                    format!("uncapped ETH transfer can be reentered before `{name}` is updated"),
                );
                self.emitted.insert(call.span);
            }
        }
    }
}

impl FlowState {
    fn clear(&mut self) {
        self.state_reads.clear();
        self.pending_value_calls.clear();
    }

    fn merge(&mut self, other: &Self) {
        self.state_reads.extend(other.state_reads.iter().copied());
        for call in &other.pending_value_calls {
            if let Some(existing) =
                self.pending_value_calls.iter_mut().find(|existing| existing.span == call.span)
            {
                existing.state_reads.extend(call.state_reads.iter().copied());
            } else {
                self.pending_value_calls.push(call.clone());
            }
        }
    }
}

fn is_uncapped_value_call(callee: &hir::Expr<'_>, opts: Option<&[hir::NamedArg<'_>]>) -> bool {
    let Some(opts) = opts else { return false };
    let ExprKind::Member(_, member) = &callee.kind else { return false };
    if member.name != kw::Call {
        return false;
    }

    let mut value = None;
    let mut gas = None;
    for opt in opts {
        if opt.name.name == sym::value {
            value = Some(&opt.value);
        } else if opt.name.name == kw::Gas {
            gas = Some(&opt.value);
        }
    }

    value.is_some_and(|value| !is_zero_literal(value)) && gas.is_none_or(gas_option_forwards_all)
}

fn is_zero_literal(expr: &hir::Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Number(value) if value.is_zero())
    )
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
