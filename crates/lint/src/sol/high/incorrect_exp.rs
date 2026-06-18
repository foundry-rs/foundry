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
        hir::{self, ElementaryType, Expr, ExprKind, TypeKind},
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
            && let Some(base) = plain_decimal_int_lit(ctx, lhs)
            && (base == U256::from(2u64) || base == U256::from(10u64))
            && plain_decimal_int_lit(ctx, rhs).is_some()
        {
            ctx.emit(&INCORRECT_EXP, expr.span);
        }
    }
}

/// Returns the value of a plain decimal integer literal, looking through parentheses and integer
/// casts (`uint256(10)`), or `None` for anything else.
///
/// Only literals written as plain decimal digits (`10`, `1_000`) qualify. Hex literals (`0x..`) are
/// bitwise intent, and scientific notation (`1e1`, which solar evaluates to `10`) is not the plain
/// integer literal the `^`/`**` typo involves. A sub-denomination (`2 wei`, `2 seconds`) is dropped
/// from the HIR but still present in the source span, so it is filtered out too. All of these are
/// left alone: this lint prefers a false negative to a false positive that would annoy developers.
fn plain_decimal_int_lit(ctx: &LintContext, expr: &Expr<'_>) -> Option<U256> {
    let expr = peel_int_casts(expr);
    if let ExprKind::Lit(lit) = &expr.kind
        && let LitKind::Number(value) = &lit.kind
    {
        let s = lit.symbol.as_str();
        if !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit() || b == b'_')
            // The source span must be exactly those digits. This rejects a sub-denomination such as
            // `2 wei` (dropped from the HIR but still in the source). If the source is unavailable,
            // err toward not flagging.
            && ctx.span_to_snippet(expr.span).is_some_and(|src| src.trim() == s)
        {
            return Some(*value);
        }
    }
    None
}

/// Looks through parentheses and integer casts (`uint256(x)`, `int8(x)`), returning the innermost
/// operand. A misplaced `^` can hide behind such a cast (`uint256(10) ^ 18`), which a bare literal
/// check would miss. Non-integer casts (`bytes32(x)`, `address(x)`) are left alone, since xor of a
/// `bytesN` bit pattern is a legitimate operation.
fn peel_int_casts<'a, 'hir>(expr: &'a Expr<'hir>) -> &'a Expr<'hir> {
    let expr = expr.peel_parens();
    if let ExprKind::Call(callee, args, _) = &expr.kind
        && let ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(ElementaryType::Int(_) | ElementaryType::UInt(_)),
            ..
        }) = &callee.peel_parens().kind
        && args.len() == 1
        && let Some(inner) = args.exprs().next()
    {
        return peel_int_casts(inner);
    }
    expr
}
