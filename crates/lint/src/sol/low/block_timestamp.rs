use super::BlockTimestamp;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{BinOp, BinOpKind, Expr, ExprKind};

declare_forge_lint!(
    BLOCK_TIMESTAMP,
    Severity::Low,
    "block-timestamp",
    "usage of `block.timestamp` in a comparison may be manipulated by validators"
);

impl<'ast> EarlyLintPass<'ast> for BlockTimestamp {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Binary(lhs, BinOp { kind, .. }, rhs) = &expr.kind
            && is_cmp(*kind)
            && (contains_block_timestamp(lhs) || contains_block_timestamp(rhs))
        {
            ctx.emit(&BLOCK_TIMESTAMP, expr.span);
        }
    }
}

const fn is_cmp(kind: BinOpKind) -> bool {
    matches!(
        kind,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    )
}

/// Returns `true` if `expr` is `block.timestamp`.
fn is_block_timestamp(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if member.as_str() == "timestamp"
            && matches!(&base.kind, ExprKind::Ident(ident) if ident.as_str() == "block")
    )
}

/// Recursively checks if an expression tree contains `block.timestamp`.
fn contains_block_timestamp(expr: &Expr<'_>) -> bool {
    if is_block_timestamp(expr) {
        return true;
    }
    match &expr.kind {
        ExprKind::Unary(_, inner) => contains_block_timestamp(inner),
        ExprKind::Binary(lhs, _, rhs) => {
            contains_block_timestamp(lhs) || contains_block_timestamp(rhs)
        }
        ExprKind::Tuple(elems) => elems.iter().any(|e| {
            if let solar::interface::SpannedOption::Some(inner) = e.as_ref() {
                contains_block_timestamp(inner)
            } else {
                false
            }
        }),
        ExprKind::Call(callee, args) => {
            contains_block_timestamp(callee) || args.exprs().any(|e| contains_block_timestamp(e))
        }
        _ => false,
    }
}
