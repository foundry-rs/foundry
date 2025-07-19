use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_sema::hir::{self, ExprKind, TypeKind};

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be avoided"
);

impl<'hir> LateLintPass<'hir> for UnsafeTypecast {
    fn check_expr(
        &mut self,
        ctx: &LintContext<'_>,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // Check for type cast expressions: Type(value)
        if let ExprKind::Call(call_expr, args, _) = &expr.kind {
            // Check if this is a type cast (function call where the function is a type)
            if let ExprKind::Type(target_type) = &call_expr.kind {
                // We need exactly one argument for a type cast
                if args.len() == 1 {
                    if let Some(first_arg) = args.exprs().next() {
                        if is_unsafe_typecast_hir(hir, first_arg, target_type) {
                            ctx.emit(&UNSAFE_TYPECAST, expr.span);
                        }
                    }
                }
            }
        }
    }
}

/// Checks if a typecast from the source expression to target type is unsafe.
fn is_unsafe_typecast_hir(
    hir: &hir::Hir<'_>,
    source_expr: &hir::Expr<'_>,
    target_type: &hir::Type<'_>,
) -> bool {
    // Get target elementary type
    let target_elem_type = match &target_type.kind {
        TypeKind::Elementary(elem_type) => elem_type,
        _ => return false,
    };

    // Determine source type from the expression
    let source_elem_type = match infer_source_type(hir, source_expr) {
        Some(elem_type) => elem_type,
        None => return false,
    };

    is_unsafe_elementary_typecast(&source_elem_type, target_elem_type)
}

/// Infers the elementary type of a source expression.
/// For cast chains, returns the ultimate source type, not intermediate cast results.
fn infer_source_type(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ElementaryType> {
    match &expr.kind {
        // Type cast: Type(value) - recursively check the inner value
        ExprKind::Call(call_expr, args, _) => {
            if let ExprKind::Type(ty) = &call_expr.kind {
                if args.len() == 1 {
                    if let TypeKind::Elementary(_elem_type) = &ty.kind {
                        // For type casts, recursively check the source of the inner expression
                        // This allows us to see through cast chains like uint160(address_var)
                        if let Some(first_arg) = args.exprs().next() {
                            return infer_source_type(hir, first_arg);
                        }
                    }
                }
            }
            // For other function calls, try to infer from context or return None
            None
        }

        // ... rest of the function remains the same
        ExprKind::Lit(lit) => {
            match &lit.kind {
                solar_ast::LitKind::Number(num) => {
                    if is_negative_number_literal(num) {
                        Some(hir::ElementaryType::Int(solar_ast::TypeSize::ZERO))
                    } else {
                        Some(hir::ElementaryType::UInt(solar_ast::TypeSize::ZERO))
                    }
                }
                solar_ast::LitKind::Address(_) => Some(hir::ElementaryType::Address(false)),
                solar_ast::LitKind::Str(..) => Some(hir::ElementaryType::String),
                solar_ast::LitKind::Bool(_) => Some(hir::ElementaryType::Bool),
                _ => None,
            }
        }

        ExprKind::Ident(resolutions) => {
            if let Some(first_res) = resolutions.first() {
                match first_res {
                    hir::Res::Item(hir::ItemId::Variable(var_id)) => {
                        let variable = hir.variable(*var_id);
                        if let TypeKind::Elementary(elem_type) = &variable.ty.kind {
                            return Some(*elem_type);
                        }
                    }
                    _ => {}
                }
            }
            None
        }

        ExprKind::Unary(op, inner_expr) => {
            match op.kind {
                solar_ast::UnOpKind::Neg => {
                    match infer_source_type(hir, inner_expr) {
                        Some(hir::ElementaryType::UInt(size)) => {
                            Some(hir::ElementaryType::Int(size))
                        }
                        Some(signed_type @ hir::ElementaryType::Int(_)) => {
                            Some(signed_type)
                        }
                        _ => {
                            Some(hir::ElementaryType::Int(solar_ast::TypeSize::ZERO))
                        }
                    }
                }
                _ => {
                    infer_source_type(hir, inner_expr)
                }
            }
        }

        _ => None,
    }
}

/// Helper function to detect negative number literals
fn is_negative_number_literal(num: &num_bigint::BigInt) -> bool {
    num.sign().eq(&num_bigint::Sign::Minus)
}

/// Checks if a type cast from source_type to target_type is unsafe.
fn is_unsafe_elementary_typecast(
    source_type: &hir::ElementaryType,
    target_type: &hir::ElementaryType,
) -> bool {
    use hir::ElementaryType;
    use solar_ast::TypeSize;

    match (source_type, target_type) {
        // Numeric downcasts (smaller target size)
        (ElementaryType::UInt(source_size), ElementaryType::UInt(target_size)) => {
            source_size.bits() > target_size.bits()
        }
        (ElementaryType::Int(source_size), ElementaryType::Int(target_size)) => {
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
        (ElementaryType::Bytes, ElementaryType::FixedBytes(_)) => true,

        // String to fixed bytes (potential truncation)
        (ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        // Address to smaller uint (truncation) - address is 160 bits
        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        // Address to int (sign issues)
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}