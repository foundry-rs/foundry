use super::IncorrectExp;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use solar::{
    ast::{BinOpKind, LitKind},
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, TypeKind},
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
        // `a ^ b` between integer literals is almost always a mistake for `a ** b`: `^` is bitwise
        // xor in Solidity, so `10 ^ 18` is `24`, not `10 ** 18`.
        //
        // To stay precise, the base is restricted to `2` and `10` (bit widths and decimals, the
        // only bases people write as powers) and hex operands are left alone. This mirrors GCC's
        // and Clang's `-Wxor-used-as-pow`; Clippy's `suspicious_xor_used_as_pow`, which drops the
        // base restriction, is allow-by-default precisely because of the resulting false positives.
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::BitXor)
            && let Some(base) = decimal_int_lit(lhs)
            && (base == U256::from(2u64) || base == U256::from(10u64))
            && decimal_int_lit(rhs).is_some()
        {
            ctx.emit(&INCORRECT_EXP, expr.span);
        }
    }
}

/// Returns the value of a decimal integer literal, looking through parentheses and elementary-type
/// casts (`uint256(10)`), or `None` for anything else.
///
/// Solidity has no binary or octal literals, so only hex (`0x`) is excluded: xor of a hex bit
/// pattern is a legitimate operation, and writing a number in hex is a strong signal the author
/// really means bitwise work.
fn decimal_int_lit(expr: &Expr<'_>) -> Option<U256> {
    if let ExprKind::Lit(lit) = &peel_casts(expr).kind
        && let LitKind::Number(value) = &lit.kind
    {
        let s = lit.symbol.as_str();
        if !s.starts_with("0x") && !s.starts_with("0X") {
            return Some(*value);
        }
    }
    None
}

/// Looks through parentheses and elementary-type casts (`uint256(x)`, `int8(x)`), returning the
/// innermost operand. A misplaced `^` can hide behind a cast (`uint256(10) ^ 18`), which a bare
/// literal check would miss.
fn peel_casts<'a, 'hir>(expr: &'a Expr<'hir>) -> &'a Expr<'hir> {
    let expr = expr.peel_parens();
    if let ExprKind::Call(callee, args, _) = &expr.kind
        && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(_), .. }) =
            &callee.peel_parens().kind
        && args.len() == 1
        && let Some(inner) = args.exprs().next()
    {
        return peel_casts(inner);
    }
    expr
}
