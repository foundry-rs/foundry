use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{LitKind, StrKind},
    sema::{
        Gcx,
        hir::{self, ElementaryType, ExprKind, TypeKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be checked"
);

impl<'hir> LateLintPass<'hir> for UnsafeTypecast {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // Check for type cast expressions: Type(value)
        if let ExprKind::Call(call, args, _) = &expr.kind
            && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call.kind
            && args.len() == 1
            && let Some(call_arg) = args.exprs().next()
            && is_unsafe_typecast_hir(gcx, call_arg, ty)
        {
            ctx.emit_with_suggestion(
                &UNSAFE_TYPECAST,
                expr.span,
                Suggestion::example(
                    format!(
                        "// casting to '{abi_ty}' is safe because [explain why]\n// forge-lint: disable-next-line(unsafe-typecast)",
                        abi_ty = ty.to_abi_str()
            )).with_desc("consider disabling this lint if you're certain the cast is safe"));
        }
    }
}

/// Determines if a typecast is potentially unsafe (could lose data or precision).
fn is_unsafe_typecast_hir<'hir>(
    gcx: Gcx<'hir>,
    source_expr: &hir::Expr<'hir>,
    target_type: &hir::ElementaryType,
) -> bool {
    let mut source_types = Vec::<ElementaryType>::new();
    infer_source_types(Some(&mut source_types), gcx, source_expr);

    if source_types.is_empty() {
        return false;
    };

    source_types.iter().any(|source_ty| is_unsafe_elementary_typecast(source_ty, target_type))
}

/// Infers the elementary source type(s) of an expression.
///
/// This function traverses an expression tree to find the original "source" types.
/// For cast chains, it returns the ultimate source type, not intermediate cast results.
/// For binary operations, it collects types from both sides into the `output` vector.
///
/// # Returns
/// An `Option<ElementaryType>` containing the inferred type of the expression if it can be
/// resolved to a single source (like variables, literals, or unary expressions).
/// Returns `None` for expressions complex expressions (like binary operations).
fn infer_source_types<'hir>(
    mut output: Option<&mut Vec<ElementaryType>>,
    gcx: Gcx<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<ElementaryType> {
    let mut track = |ty: ElementaryType| -> Option<ElementaryType> {
        if let Some(output) = output.as_mut() {
            output.push(ty);
        }
        Some(ty)
    };

    match &expr.kind {
        // A type cast call: `Type(val)`
        ExprKind::Call(call_expr, args, ..) => {
            // Check if the called expression is a type, which indicates a cast.
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(..), .. }) =
                &call_expr.kind
                && let Some(inner) = args.exprs().next()
            {
                // Recurse to find the original (inner-most) source type.
                return infer_source_types(output, gcx, inner);
            }
            expr_elementary_type(gcx, expr).and_then(track)
        }

        // Handle string literals explicitly; Solar records them as literal types rather than
        // elementary `string`/`bytes`.
        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => track(ElementaryType::Bytes),
            LitKind::Str(..) => track(ElementaryType::String),
            _ => expr_elementary_type(gcx, expr).and_then(track),
        },

        // Identifiers and other simple typed expressions.
        ExprKind::Ident(_) => expr_elementary_type(gcx, expr).and_then(track),

        // Unary operations: Recurse to find the source type of the inner expression.
        ExprKind::Unary(_, inner_expr) => infer_source_types(output, gcx, inner_expr),

        // Binary operations
        ExprKind::Binary(lhs, _, rhs) => {
            if let Some(mut output) = output {
                // Recurse on both sides to find and collect all source types.
                infer_source_types(Some(&mut output), gcx, lhs);
                infer_source_types(Some(&mut output), gcx, rhs);
            }
            None
        }

        _ => expr_elementary_type(gcx, expr).and_then(track),
    }
}

fn expr_elementary_type<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> Option<ElementaryType> {
    match gcx.type_of_expr(expr.peel_parens().id)?.peel_refs().kind {
        TyKind::Elementary(ty) => Some(ty),
        TyKind::StringLiteral(true, _) => Some(ElementaryType::String),
        TyKind::StringLiteral(false, _) => Some(ElementaryType::Bytes),
        _ => None,
    }
}

/// Checks if a type cast from source_type to target_type is unsafe.
const fn is_unsafe_elementary_typecast(
    source_type: &ElementaryType,
    target_type: &ElementaryType,
) -> bool {
    match (source_type, target_type) {
        // Numeric downcasts (smaller target size)
        (ElementaryType::UInt(source_size), ElementaryType::UInt(target_size))
        | (ElementaryType::Int(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() > target_size.bits()
        }

        // Signed to unsigned conversion (potential loss of sign)
        (ElementaryType::Int(_), ElementaryType::UInt(_)) => true,

        // Unsigned to signed conversion with same or smaller size
        (ElementaryType::UInt(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() >= target_size.bits()
        }

        // Fixed bytes to smaller fixed bytes
        (ElementaryType::FixedBytes(source_size), ElementaryType::FixedBytes(target_size)) => {
            source_size.bytes() > target_size.bytes()
        }

        // Dynamic bytes to fixed bytes (potential truncation)
        (ElementaryType::Bytes | ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        // Address to smaller uint (truncation) - address is 160 bits
        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        // Address to int (sign issues)
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}
