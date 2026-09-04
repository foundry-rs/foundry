use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, analysis::cast_type},
};
use solar::{
    ast::{BinOpKind, LitKind, StrKind},
    sema::{
        Gcx,
        hir::{self, ElementaryType, ExprKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be checked"
);

impl<'gcx> LateLintPass<'gcx> for UnsafeTypecast {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) {
        if let ExprKind::Call(call, args, _) = &expr.kind
            && let Some(ty) = cast_type(call)
            && args.len() == 1
            && let Some(arg) = args.exprs().next()
            && !is_bounded_by_mask(arg, ty)
        {
            let mut sources = Vec::new();
            source_types(gcx, arg, &mut sources);
            if sources.iter().any(|source| is_unsafe_elementary_typecast(*source, ty)) {
                ctx.emit_with_suggestion(
                    &UNSAFE_TYPECAST,
                    expr.span,
                    Suggestion::example(format!(
                        "// casting to '{}' is safe because [explain why]\n// forge-lint: disable-next-line(unsafe-typecast)",
                        ty.to_abi_str()
                    ))
                    .with_desc("consider disabling this lint if you're certain the cast is safe"),
                );
            }
        }
    }
}

/// `x & MASK` where the mask literal fits the unsigned target bounds the value to its range.
fn is_bounded_by_mask(source: &hir::Expr<'_>, target: ElementaryType) -> bool {
    let ElementaryType::UInt(target_size) = target else { return false };
    let ExprKind::Binary(lhs, op, rhs) = &source.peel_parens().kind else { return false };
    op.kind == BinOpKind::BitAnd
        && [lhs, rhs].into_iter().any(|expr| {
            matches!(
                expr.peel_parens().kind,
                ExprKind::Lit(hir::Lit { kind: LitKind::Number(mask), .. })
                    if mask.bit_len() <= target_size.bits() as usize
            )
        })
}

/// Collects the ultimate elementary source type(s) of `expr` into `out`, looking through cast
/// chains and unary operators and gathering both sides of binary operations.
fn source_types<'gcx>(gcx: Gcx<'gcx>, expr: &hir::Expr<'gcx>, out: &mut Vec<ElementaryType>) {
    match &expr.kind {
        ExprKind::Call(callee, args, _) if cast_type(callee).is_some() => {
            if let Some(inner) = args.exprs().next() {
                source_types(gcx, inner, out);
            }
        }
        // Solar types string literals as literal types rather than `string`/`bytes`.
        ExprKind::Lit(hir::Lit { kind: LitKind::Str(StrKind::Hex, ..), .. }) => {
            out.push(ElementaryType::Bytes)
        }
        ExprKind::Lit(hir::Lit { kind: LitKind::Str(..), .. }) => out.push(ElementaryType::String),
        ExprKind::Unary(_, inner) => source_types(gcx, inner, out),
        ExprKind::Binary(lhs, _, rhs) => {
            source_types(gcx, lhs, out);
            source_types(gcx, rhs, out);
        }
        _ => {
            if let Some(ty) = gcx.type_of_expr(expr.peel_parens().id) {
                match ty.peel_refs().kind {
                    TyKind::Elementary(ty) => out.push(ty),
                    TyKind::StringLiteral(true, _) => out.push(ElementaryType::String),
                    TyKind::StringLiteral(false, _) => out.push(ElementaryType::Bytes),
                    _ => {}
                }
            }
        }
    }
}

/// Whether casting `source` to `target` can lose data, precision or sign.
const fn is_unsafe_elementary_typecast(source: ElementaryType, target: ElementaryType) -> bool {
    match (source, target) {
        (ElementaryType::UInt(from), ElementaryType::UInt(to))
        | (ElementaryType::Int(from), ElementaryType::Int(to)) => from.bits() > to.bits(),
        (ElementaryType::Int(_), ElementaryType::UInt(_)) => true,
        (ElementaryType::UInt(from), ElementaryType::Int(to)) => from.bits() >= to.bits(),
        (ElementaryType::FixedBytes(from), ElementaryType::FixedBytes(to)) => {
            from.bytes() > to.bytes()
        }
        (ElementaryType::Bytes | ElementaryType::String, ElementaryType::FixedBytes(_)) => true,
        // `address` is 160 bits.
        (ElementaryType::Address(_), ElementaryType::UInt(to)) => to.bits() < 160,
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,
        _ => false,
    }
}
