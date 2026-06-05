use super::UnusedReturn;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::interface::receiver_contract_id},
};
use solar::{
    interface::Span,
    sema::{
        Gcx, Hir,
        hir::{Block, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, TypeKind, VariableId},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    UNUSED_RETURN,
    Severity::Med,
    "unused-return",
    "Return value of an external call is not used"
);

impl<'hir> LateLintPass<'hir> for UnusedReturn {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            let mut state = ReturnUseState::default();
            check_block(ctx, hir, body, &mut state);
            state.finish(ctx);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PendingReturn {
    var_id: VariableId,
    span: Span,
}

#[derive(Clone, Default)]
struct ReturnUseState {
    pending: Vec<PendingReturn>,
    emitted: HashSet<Span>,
}

impl ReturnUseState {
    fn emit(&mut self, ctx: &LintContext, span: Span) {
        if self.emitted.insert(span) {
            ctx.emit(&UNUSED_RETURN, span);
        }
    }

    fn add_pending(&mut self, ctx: &LintContext, var_id: VariableId, span: Span) {
        if let Some(pending) = self.pending.iter_mut().find(|p| p.var_id == var_id) {
            let old_span = pending.span;
            pending.span = span;
            self.emit(ctx, old_span);
        } else {
            self.pending.push(PendingReturn { var_id, span });
        }
    }

    fn mark_read(&mut self, var_id: VariableId) {
        if let Some(idx) = self.pending.iter().position(|p| p.var_id == var_id) {
            self.pending.remove(idx);
        }
    }

    fn mark_overwritten(&mut self, ctx: &LintContext, var_id: VariableId) {
        if let Some(idx) = self.pending.iter().position(|p| p.var_id == var_id) {
            let pending = self.pending.remove(idx);
            self.emit(ctx, pending.span);
        }
    }

    fn merge_from(&mut self, other: Self) {
        self.emitted.extend(other.emitted);
        for pending in other.pending {
            if !self.pending.contains(&pending) {
                self.pending.push(pending);
            }
        }
    }

    fn finish(&mut self, ctx: &LintContext) {
        for pending in std::mem::take(&mut self.pending) {
            self.emit(ctx, pending.span);
        }
    }
}

fn check_block<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    state: &mut ReturnUseState,
) {
    for stmt in block.stmts {
        check_stmt(ctx, hir, stmt, state);
    }
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    state: &mut ReturnUseState,
) {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                check_expr(ctx, hir, init, state);
                if is_unused_return_call(hir, init) {
                    add_pending_var(ctx, hir, *var_id, init.span, state);
                }
            }
        }
        StmtKind::DeclMulti(vars, expr) => {
            check_expr(ctx, hir, expr, state);
            update_multi_capture(ctx, hir, vars, expr, state);
        }
        StmtKind::Expr(expr) => {
            if is_unused_return_call(hir, expr) {
                state.emit(ctx, expr.span);
            }
            check_expr(ctx, hir, expr, state);
        }
        StmtKind::Emit(expr) | StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            check_expr(ctx, hir, expr, state);
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            check_expr(ctx, hir, cond, state);

            let baseline = state.clone();
            let mut then_state = baseline.clone();
            check_stmt(ctx, hir, then_stmt, &mut then_state);

            let mut merged = ReturnUseState::default();
            merged.merge_from(then_state);

            if let Some(else_stmt) = else_stmt {
                let mut else_state = baseline;
                check_stmt(ctx, hir, else_stmt, &mut else_state);
                merged.merge_from(else_state);
            } else {
                merged.merge_from(baseline);
            }

            *state = merged;
        }
        StmtKind::Loop(block, _) => {
            let baseline = state.clone();
            let mut loop_state = baseline.clone();
            check_block(ctx, hir, *block, &mut loop_state);

            let mut merged = baseline;
            merged.merge_from(loop_state);
            *state = merged;
        }
        StmtKind::Try(try_stmt) => {
            check_expr(ctx, hir, &try_stmt.expr, state);

            let baseline = state.clone();
            let mut merged = baseline.clone();
            for clause in try_stmt.clauses {
                let mut clause_state = baseline.clone();
                check_block(ctx, hir, clause.block, &mut clause_state);
                merged.merge_from(clause_state);
            }
            *state = merged;
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            check_block(ctx, hir, *block, state);
        }
        StmtKind::AssemblyBlock(block) => check_block(ctx, hir, *block, state),
        StmtKind::Switch(switch) => {
            check_expr(ctx, hir, switch.selector, state);

            let baseline = state.clone();
            let mut merged = baseline.clone();
            for case in switch.cases {
                let mut case_state = baseline.clone();
                check_block(ctx, hir, case.body, &mut case_state);
                merged.merge_from(case_state);
            }
            *state = merged;
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::Err(_) => {}
    }
}

fn check_expr<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    state: &mut ReturnUseState,
) {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, op, rhs) => {
            check_expr(ctx, hir, rhs, state);
            if op.is_some() {
                check_expr(ctx, hir, lhs, state);
                overwrite_lhs(ctx, hir, lhs, state);
            } else {
                check_lhs_reads(ctx, hir, lhs, state);
                update_assignment_capture(ctx, hir, lhs, rhs, state);
            }
        }
        ExprKind::Call(callee, args, options) => {
            check_expr(ctx, hir, callee, state);
            if let Some(options) = options {
                for arg in options.args {
                    check_expr(ctx, hir, &arg.value, state);
                }
            }
            for arg in args.exprs() {
                check_expr(ctx, hir, arg, state);
            }
        }
        ExprKind::Binary(lhs, _, rhs) => {
            check_expr(ctx, hir, lhs, state);
            check_expr(ctx, hir, rhs, state);
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner)
        | ExprKind::YulMember(inner, _) => check_expr(ctx, hir, inner, state),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr(ctx, hir, cond, state);

            let baseline = state.clone();
            let mut then_state = baseline.clone();
            check_expr(ctx, hir, then_expr, &mut then_state);
            let mut else_state = baseline;
            check_expr(ctx, hir, else_expr, &mut else_state);

            let mut merged = ReturnUseState::default();
            merged.merge_from(then_state);
            merged.merge_from(else_state);
            *state = merged;
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                check_expr(ctx, hir, expr, state);
            }
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                check_expr(ctx, hir, expr, state);
            }
        }
        ExprKind::Index(base, index) => {
            check_expr(ctx, hir, base, state);
            if let Some(index) = index {
                check_expr(ctx, hir, index, state);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, hir, base, state);
            if let Some(start) = start {
                check_expr(ctx, hir, start, state);
            }
            if let Some(end) = end {
                check_expr(ctx, hir, end, state);
            }
        }
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    state.mark_read(*var_id);
                }
            }
        }
        ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

fn update_multi_capture(
    ctx: &LintContext,
    hir: &Hir<'_>,
    vars: &[Option<VariableId>],
    expr: &Expr<'_>,
    state: &mut ReturnUseState,
) {
    if let ExprKind::Tuple(exprs) = &expr.peel_parens().kind
        && exprs.len() == vars.len()
    {
        for (var_id, rhs) in vars.iter().zip(*exprs) {
            if let Some(var_id) = var_id {
                if let Some(rhs) = rhs
                    && is_unused_return_call(hir, rhs)
                {
                    add_pending_var(ctx, hir, *var_id, rhs.span, state);
                } else {
                    state.mark_overwritten(ctx, *var_id);
                }
            }
        }
        return;
    }

    if is_unused_return_call(hir, expr) {
        for var_id in vars.iter().flatten() {
            add_pending_var(ctx, hir, *var_id, expr.span, state);
        }
    } else {
        for var_id in vars.iter().flatten() {
            state.mark_overwritten(ctx, *var_id);
        }
    }
}

fn update_assignment_capture(
    ctx: &LintContext,
    hir: &Hir<'_>,
    lhs: &Expr<'_>,
    rhs: &Expr<'_>,
    state: &mut ReturnUseState,
) {
    if let (ExprKind::Tuple(lhs_exprs), ExprKind::Tuple(rhs_exprs)) =
        (&lhs.peel_parens().kind, &rhs.peel_parens().kind)
        && lhs_exprs.len() == rhs_exprs.len()
    {
        for (lhs, rhs) in lhs_exprs.iter().zip(*rhs_exprs) {
            if let Some(lhs) = lhs {
                if let Some(rhs) = rhs
                    && is_unused_return_call(hir, rhs)
                {
                    add_pending_lhs(ctx, hir, lhs, rhs.span, state);
                } else {
                    overwrite_lhs(ctx, hir, lhs, state);
                }
            }
        }
        return;
    }

    if is_unused_return_call(hir, rhs) {
        add_pending_lhs(ctx, hir, lhs, rhs.span, state);
    } else {
        overwrite_lhs(ctx, hir, lhs, state);
    }
}

fn add_pending_lhs(
    ctx: &LintContext,
    hir: &Hir<'_>,
    lhs: &Expr<'_>,
    span: Span,
    state: &mut ReturnUseState,
) {
    let mut locals = Vec::new();
    collect_lhs_locals(hir, lhs, &mut locals);
    for var_id in locals {
        add_pending_var(ctx, hir, var_id, span, state);
    }
}

fn add_pending_var(
    ctx: &LintContext,
    hir: &Hir<'_>,
    var_id: VariableId,
    span: Span,
    state: &mut ReturnUseState,
) {
    if hir.variable(var_id).is_local_or_return() {
        state.add_pending(ctx, var_id, span);
    }
}

fn overwrite_lhs(ctx: &LintContext, hir: &Hir<'_>, lhs: &Expr<'_>, state: &mut ReturnUseState) {
    let mut locals = Vec::new();
    collect_lhs_locals(hir, lhs, &mut locals);
    for var_id in locals {
        state.mark_overwritten(ctx, var_id);
    }
}

fn collect_lhs_locals(hir: &Hir<'_>, lhs: &Expr<'_>, out: &mut Vec<VariableId>) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res
                    && hir.variable(*var_id).is_local_or_return()
                {
                    out.push(*var_id);
                }
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_lhs_locals(hir, expr, out);
            }
        }
        _ => {}
    }
}

fn check_lhs_reads<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    lhs: &'hir Expr<'hir>,
    state: &mut ReturnUseState,
) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(_) => {}
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                check_lhs_reads(ctx, hir, expr, state);
            }
        }
        ExprKind::Member(base, _) | ExprKind::YulMember(base, _) => {
            check_expr(ctx, hir, base, state);
        }
        ExprKind::Index(base, index) => {
            check_expr(ctx, hir, base, state);
            if let Some(index) = index {
                check_expr(ctx, hir, index, state);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, hir, base, state);
            if let Some(start) = start {
                check_expr(ctx, hir, start, state);
            }
            if let Some(end) = end {
                check_expr(ctx, hir, end, state);
            }
        }
        _ => check_expr(ctx, hir, lhs, state),
    }
}

/// Returns true if `expr` is a member call on a contract whose resolved function has return
/// values, excluding ERC20 `transfer`/`transferFrom` (covered by `erc20-unchecked-transfer`).
fn is_unused_return_call(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let is_type = |var_id: VariableId, type_str: &str| {
        matches!(
            &hir.variable(var_id).ty.kind,
            TypeKind::Elementary(ty) if ty.to_abi_str() == type_str
        )
    };

    let ExprKind::Call(callee, call_args, ..) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(contract_expr, func_ident) = &callee.peel_parens().kind else {
        return false;
    };

    // Arity from either positional or named args.
    let arity = call_args.kind.len();

    let Some(cid) = receiver_contract_id(hir, contract_expr) else { return false };

    // Collect all functions in the contract matching this name and arity.
    let candidates: Vec<&Function<'_>> = hir
        .contract_item_ids(cid)
        .filter_map(|item| {
            let fid = item.as_function()?;
            let func = hir.function(fid);
            (func.name.is_some_and(|n| n.as_str() == func_ident.as_str())
                && func.kind.is_function()
                && func.parameters.len() == arity)
                .then_some(func)
        })
        .collect();

    // No matching candidate found, nothing to lint.
    if candidates.is_empty() {
        return false;
    }

    // If any candidate returns nothing, we can't tell which overload is being called,
    // skip to avoid a false positive.
    if candidates.iter().any(|f| f.returns.is_empty()) {
        return false;
    }

    // If any candidate is an ERC20 transfer/transferFrom, defer to erc20-unchecked-transfer.
    if candidates.iter().any(|f| is_erc20_transfer_sig(f, func_ident.as_str(), &is_type)) {
        return false;
    }

    true
}

/// Returns true if `func` matches the ERC20 `transfer` or `transferFrom` signature exactly.
/// These are handled by `erc20-unchecked-transfer` and must not be double-reported.
fn is_erc20_transfer_sig(
    func: &Function<'_>,
    name: &str,
    is_type: &impl Fn(VariableId, &str) -> bool,
) -> bool {
    match name {
        "transfer" if func.parameters.len() == 2 && func.returns.len() == 1 => {
            is_type(func.parameters[0], "address")
                && is_type(func.parameters[1], "uint256")
                && is_type(func.returns[0], "bool")
        }
        "transferFrom" if func.parameters.len() == 3 && func.returns.len() == 1 => {
            is_type(func.parameters[0], "address")
                && is_type(func.parameters[1], "address")
                && is_type(func.parameters[2], "uint256")
                && is_type(func.returns[0], "bool")
        }
        _ => false,
    }
}
