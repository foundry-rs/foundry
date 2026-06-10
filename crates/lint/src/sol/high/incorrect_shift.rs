use super::IncorrectShift;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Stmt, StmtKind, yul},
    interface::kw,
};

declare_forge_lint!(
    INCORRECT_SHIFT,
    Severity::High,
    "incorrect-shift",
    "the order of args in a shift operation is incorrect"
);

impl<'ast> EarlyLintPass<'ast> for IncorrectShift {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        if let StmtKind::Assembly(assembly) = &stmt.kind {
            check_yul_block(ctx, &assembly.block);
        }
    }
}

fn check_yul_block(ctx: &LintContext, block: &yul::Block<'_>) {
    for stmt in block.stmts.iter() {
        check_yul_stmt(ctx, stmt);
    }
}

fn check_yul_stmt(ctx: &LintContext, stmt: &yul::Stmt<'_>) {
    match &stmt.kind {
        yul::StmtKind::Block(block) => check_yul_block(ctx, block),
        yul::StmtKind::AssignSingle(_, expr)
        | yul::StmtKind::AssignMulti(_, expr)
        | yul::StmtKind::Expr(expr) => check_yul_expr(ctx, expr),
        yul::StmtKind::If(cond, block) => {
            check_yul_expr(ctx, cond);
            check_yul_block(ctx, block);
        }
        yul::StmtKind::For(for_stmt) => {
            check_yul_block(ctx, &for_stmt.init);
            check_yul_expr(ctx, &for_stmt.cond);
            check_yul_block(ctx, &for_stmt.step);
            check_yul_block(ctx, &for_stmt.body);
        }
        yul::StmtKind::Switch(switch) => {
            check_yul_expr(ctx, &switch.selector);
            for case in switch.cases.iter() {
                check_yul_block(ctx, &case.body);
            }
        }
        yul::StmtKind::FunctionDef(func) => check_yul_block(ctx, &func.body),
        yul::StmtKind::VarDecl(_, Some(init)) => check_yul_expr(ctx, init),
        yul::StmtKind::Leave
        | yul::StmtKind::Break
        | yul::StmtKind::Continue
        | yul::StmtKind::VarDecl(_, None) => {}
    }
}

fn check_yul_expr(ctx: &LintContext, expr: &yul::Expr<'_>) {
    let yul::ExprKind::Call(call) = &expr.kind else { return };

    if matches!(call.name.name, kw::Shl | kw::Shr | kw::Sar)
        && let [left, right] = call.arguments.as_ref()
        && !matches!(left.kind, yul::ExprKind::Lit(_))
        && matches!(right.kind, yul::ExprKind::Lit(_))
    {
        ctx.emit(&INCORRECT_SHIFT, expr.span);
    }

    for arg in call.arguments.iter() {
        check_yul_expr(ctx, arg);
    }
}
