use super::TooManyDigits;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{Expr, ExprKind, Lit, LitKind, Stmt, StmtKind, yul};

declare_forge_lint!(
    TOO_MANY_DIGITS,
    Severity::Info,
    "too-many-digits",
    "numeric literal with many digits is error-prone; \
     use scientific notation, sub-denominations, or underscore separators"
);

impl<'ast> EarlyLintPass<'ast> for TooManyDigits {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        if let StmtKind::Assembly(assembly) = &stmt.kind {
            check_yul_block(ctx, &assembly.block);
        }
    }

    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        let ExprKind::Lit(lit, sub_denom) = &expr.kind else { return };
        check_lit(ctx, lit, sub_denom.is_some());
    }
}

fn check_lit(ctx: &LintContext, lit: &Lit<'_>, has_sub_denom: bool) {
    // Only plain integer literals. `LitKind::Address` (40-hex-digit address) is a
    // distinct variant and is therefore skipped automatically.
    if !matches!(lit.kind, LitKind::Number(_)) {
        return;
    }

    // Skip literals with a sub-denomination, e.g. `1000000 gwei`, `5 minutes`.
    if has_sub_denom {
        return;
    }

    let s = lit.symbol.as_str();
    let is_hex = is_hex_literal(s);

    // Match Slither's detector: skip only address-shaped hex constants, not all hex
    // constants. Long padded masks/selectors are still hard to review.
    if is_hex_address(s) {
        return;
    }

    // Skip if the user already used scientific notation (`1e18`).
    if !is_hex && (s.contains('e') || s.contains('E')) {
        return;
    }

    // 5+ consecutive zeros in the literal as written. Underscores are
    // preserved, so `1_000_000` passes while `1_000000` is flagged.
    if s.contains("00000") {
        ctx.emit(&TOO_MANY_DIGITS, lit.span);
    }
}

fn is_hex_literal(s: &str) -> bool {
    s.starts_with("0x") || s.starts_with("0X")
}

fn is_hex_address(s: &str) -> bool {
    let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) else { return false };
    hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit())
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
                if let Some(lit) = &case.constant {
                    check_lit(ctx, lit, false);
                }
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
    match &expr.kind {
        yul::ExprKind::Call(call) => {
            for arg in call.arguments.iter() {
                check_yul_expr(ctx, arg);
            }
        }
        yul::ExprKind::Lit(lit) => check_lit(ctx, lit, false),
        yul::ExprKind::Path(_) => {}
    }
}
