use super::CostlyLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Hir,
    hir::{Block, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind},
};

declare_forge_lint!(COSTLY_LOOP, Severity::Gas, "costly-loop", "storage write inside a loop");

impl<'hir> LateLintPass<'hir> for CostlyLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            check_block(ctx, hir, body, 0);
        }
    }
}

fn check_block<'hir>(ctx: &LintContext, hir: &'hir Hir<'hir>, block: Block<'hir>, loop_depth: u32) {
    for stmt in block.stmts {
        check_stmt(ctx, hir, stmt, loop_depth);
    }
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    loop_depth: u32,
) {
    match &stmt.kind {
        StmtKind::Loop(block, _) => check_block(ctx, hir, *block, loop_depth + 1),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            check_block(ctx, hir, *block, loop_depth);
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            check_stmt(ctx, hir, then_stmt, loop_depth);
            if let Some(else_stmt) = else_stmt {
                check_stmt(ctx, hir, else_stmt, loop_depth);
            }
        }
        StmtKind::Try(stmt_try) => {
            for clause in stmt_try.clauses {
                check_block(ctx, hir, clause.block, loop_depth);
            }
        }
        StmtKind::Expr(expr) if loop_depth > 0 => {
            check_expr_for_writes(ctx, hir, expr);
        }
        StmtKind::DeclSingle(var_id) if loop_depth > 0 => {
            if let Some(init) = hir.variable(*var_id).initializer {
                check_expr_for_writes(ctx, hir, init);
            }
        }
        StmtKind::DeclMulti(_, expr) if loop_depth > 0 => {
            check_expr_for_writes(ctx, hir, expr);
        }
        StmtKind::Return(Some(expr)) if loop_depth > 0 => {
            check_expr_for_writes(ctx, hir, expr);
        }
        StmtKind::Emit(expr) | StmtKind::Revert(expr) if loop_depth > 0 => {
            check_expr_for_writes(ctx, hir, expr);
        }
        _ => {}
    }
}

fn check_expr_for_writes<'hir>(ctx: &LintContext, hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) {
    match &expr.kind {
        ExprKind::Assign(lhs, _, rhs) => {
            if lvalue_is_state_var(hir, lhs) {
                ctx.emit(&COSTLY_LOOP, expr.span);
            }
            check_expr_for_writes(ctx, hir, lhs);
            check_expr_for_writes(ctx, hir, rhs);
        }
        ExprKind::Unary(op, inner) => {
            if op.kind.has_side_effects() && lvalue_is_state_var(hir, inner) {
                ctx.emit(&COSTLY_LOOP, expr.span);
            }
            check_expr_for_writes(ctx, hir, inner);
        }
        ExprKind::Delete(inner) => {
            if lvalue_is_state_var(hir, inner) {
                ctx.emit(&COSTLY_LOOP, expr.span);
            }
            check_expr_for_writes(ctx, hir, inner);
        }
        ExprKind::Binary(lhs, _, rhs) => {
            check_expr_for_writes(ctx, hir, lhs);
            check_expr_for_writes(ctx, hir, rhs);
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr_for_writes(ctx, hir, cond);
            check_expr_for_writes(ctx, hir, then_expr);
            check_expr_for_writes(ctx, hir, else_expr);
        }
        ExprKind::Call(callee, args, named_args) => {
            check_expr_for_writes(ctx, hir, callee);
            for arg in args.exprs() {
                check_expr_for_writes(ctx, hir, arg);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    check_expr_for_writes(ctx, hir, &arg.value);
                }
            }
        }
        ExprKind::Index(base, index) => {
            check_expr_for_writes(ctx, hir, base);
            if let Some(index) = index {
                check_expr_for_writes(ctx, hir, index);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr_for_writes(ctx, hir, base);
            if let Some(start) = start {
                check_expr_for_writes(ctx, hir, start);
            }
            if let Some(end) = end {
                check_expr_for_writes(ctx, hir, end);
            }
        }
        ExprKind::Member(base, _) | ExprKind::Payable(base) => {
            check_expr_for_writes(ctx, hir, base);
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs.iter().flatten() {
                check_expr_for_writes(ctx, hir, e);
            }
        }
        ExprKind::Array(exprs) => {
            for e in *exprs {
                check_expr_for_writes(ctx, hir, e);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

/// Returns `true` if the lvalue expression ultimately writes to a storage variable.
///
/// Peels through index accesses, member accesses, and slices to find the root identifier.
fn lvalue_is_state_var(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            hir.variable(*id).is_state_variable()
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base) => lvalue_is_state_var(hir, base),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().any(|e| lvalue_is_state_var(hir, e)),
        _ => false,
    }
}
