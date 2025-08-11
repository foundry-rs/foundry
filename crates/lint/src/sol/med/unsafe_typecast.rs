use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Snippet},
    sol::{Severity, SolLint},
};
use solar_ast::{LitKind, StrKind};
use solar_sema::hir::{self, ElementaryType, ExprKind, ItemId, Res, TypeKind};

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be checked"
);

impl<'hir> LateLintPass<'hir> for UnsafeTypecast {
    fn check_expr(
        &mut self,
        ctx: &LintContext<'_>,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // Check for type cast expressions: Type(value)
        if let ExprKind::Call(call, args, _) = &expr.kind
            && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call.kind
            && args.len() == 1
            && let Some(call_arg) = args.exprs().next()
            && is_unsafe_typecast_hir(hir, call_arg, ty)
        {
            ctx.emit_with_fix(
                &UNSAFE_TYPECAST,
                expr.span,
                Snippet::Block {
                    desc: Some("Consider disabling this lint if you're certain the cast is safe:"),
                    code: format!(
                        "// casting to '{abi_ty}' is safe because [explain why]\n// forge-lint: disable-next-line(unsafe-typecast)",
                        abi_ty = ty.to_abi_str()
                    )
                }
            );
        }
    }
}

fn is_unsafe_typecast_hir(
    hir: &hir::Hir<'_>,
    source_expr: &hir::Expr<'_>,
    target_type: &hir::ElementaryType,
) -> bool {
    let Some(source_elem_type) = infer_source_type(hir, source_expr) else {
        return false;
    };

    if let ElementaryType::Int(tgt_size) = target_type {
        if let ExprKind::Call(call_expr, args, _) = &source_expr.kind {
            if let ExprKind::Type(hir::Type {
                kind: TypeKind::Elementary(ElementaryType::UInt(_src_bits)),
                ..
            }) = &call_expr.kind
            {
                if let Some(inner) = args.exprs().next() {
                    if let Some(ElementaryType::UInt(orig_bits)) = infer_source_type(hir, inner) {
                        if orig_bits.bits() < tgt_size.bits() {
                            return false;
                        }
                    }
                }
            }
        }
    }

    is_unsafe_elementary_typecast(&source_elem_type, target_type)
}

/// Infers the elementary type of a source expression.
/// For cast chains, returns the ultimate source type, not intermediate cast results.
fn infer_source_type(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<ElementaryType> {
    match &expr.kind {
        // A type cast call: Type(val)
        ExprKind::Call(call_expr, _args, _) => {
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(elem_type), .. }) =
                &call_expr.kind
            {
                return Some(*elem_type);
            }
            None
        }

        // Identifiers (variables)
        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(var_id))) = resolutions.first() {
                let variable = hir.variable(*var_id);
                if let TypeKind::Elementary(elem_type) = &variable.ty.kind {
                    return Some(*elem_type);
                }
            }
            None
        }

        // Handle literal strings/hex
        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => Some(ElementaryType::Bytes),
            LitKind::Str(..) => Some(ElementaryType::String),
            LitKind::Address(_) => Some(ElementaryType::Address(false)),
            LitKind::Bool(_) => Some(ElementaryType::Bool),

            // Unnecessary to check numbers as assigning literal values which cannot fit into a type
            // throws a compiler error. Reference: <https://solang.readthedocs.io/en/latest/language/types.html>
            _ => None,
        },

        // Unary operations
        ExprKind::Unary(op, inner_expr) => match op.kind {
            solar_ast::UnOpKind::Neg => match infer_source_type(hir, inner_expr) {
                Some(ElementaryType::UInt(size)) => Some(ElementaryType::Int(size)),
                Some(signed_type @ ElementaryType::Int(_)) => Some(signed_type),
                _ => Some(ElementaryType::Int(solar_ast::TypeSize::ZERO)),
            },
            _ => infer_source_type(hir, inner_expr),
        },

        ExprKind::Binary(lhs, _, rhs) => {
            if let Some(ty) = infer_source_type(hir, lhs) {
                return Some(ty);
            }
            infer_source_type(hir, rhs)
        }
        _ => None,
    }
}

/// Checks if a type cast from source_type to target_type is unsafe.
fn is_unsafe_elementary_typecast(
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
        (ElementaryType::Bytes, ElementaryType::FixedBytes(_))
        | (ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        // Address to smaller uint (truncation) - address is 160 bits
        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        // Address to int (sign issues)
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}
