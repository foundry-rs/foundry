use super::IncorrectExp;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, LitKind},
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind},
    },
};

declare_forge_lint!(
    INCORRECT_EXP,
    Severity::High,
    "incorrect-exp",
    "`^` is bitwise xor, not exponentiation; use `**`"
);

impl<'hir> LateLintPass<'hir> for IncorrectExp {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        _hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // `a ^ b` between two decimal integer literals is almost always a mistake for `a ** b`:
        // `^` is bitwise xor in Solidity, so `10 ^ 18` is `24`, not `10 ** 18`. Hex literals are
        // excluded since xor of bit patterns is a legitimate operation.
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::BitXor)
            && is_decimal_int_lit(lhs)
            && is_decimal_int_lit(rhs)
        {
            ctx.emit(&INCORRECT_EXP, expr.span);
        }
    }
}

/// True for a non-hex integer literal (the form a misplaced `^` usually involves). Solidity has no
/// binary or octal literals, so only hex (`0x`) needs to be excluded; xor of a hex bit pattern is a
/// legitimate operation.
fn is_decimal_int_lit(expr: &Expr<'_>) -> bool {
    if let ExprKind::Lit(lit) = &expr.peel_parens().kind
        && matches!(lit.kind, LitKind::Number(_))
    {
        let s = lit.symbol.as_str();
        return !s.starts_with("0x") && !s.starts_with("0X");
    }
    false
}
