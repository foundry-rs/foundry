use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{LitKind, StrKind},
    sema::hir::{self, ElementaryType, ExprKind, ItemId, Res, TypeKind},
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
fn is_unsafe_typecast_hir(
    hir: &hir::Hir<'_>,
    source_expr: &hir::Expr<'_>,
    target_type: &hir::ElementaryType,
) -> bool {
    let mut source_types = Vec::<ElementaryType>::new();
    infer_source_types(&mut source_types, hir, source_expr);

    if source_types.is_empty() {
        return false;
    };

    source_types.iter().any(|source_ty| is_unsafe_elementary_typecast(source_ty, target_type))
}

/// Infers the elementary source type(s) of an expression.
///
/// Traverses an expression tree to find the original "source" types and pushes them
/// into `output`. For cast chains, finds the ultimate source type, not intermediate
/// cast results. For binary and ternary operations, collects types from all branches.
fn infer_source_types(output: &mut Vec<ElementaryType>, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) {
    match &expr.kind {
        // A type cast call: `Type(val)`, or a regular function call.
        ExprKind::Call(call_expr, args, ..) => {
            // Check if the called expression is a type, which indicates a cast.
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(..), .. }) =
                &call_expr.kind
                && let Some(inner) = args.exprs().next()
            {
                // Recurse to find the original (inner-most) source type.
                infer_source_types(output, hir, inner);
                return;
            }

            // For non-cast function calls, infer the return type from the function signature.
            resolve_call_return_types(output, hir, call_expr);
        }

        // Identifiers (variables)
        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(var_id))) = resolutions.first() {
                let variable = hir.variable(*var_id);
                if let TypeKind::Elementary(elem_type) = &variable.ty.kind {
                    output.push(*elem_type);
                }
            }
        }

        // Handle literal values
        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => output.push(ElementaryType::Bytes),
            LitKind::Str(..) => output.push(ElementaryType::String),
            LitKind::Address(_) => output.push(ElementaryType::Address(false)),
            LitKind::Bool(_) => output.push(ElementaryType::Bool),
            // Unnecessary to check numbers as assigning literal values that cannot fit into a type
            // throws a compiler error. Reference: <https://solang.readthedocs.io/en/latest/language/types.html>
            _ => {}
        },

        // Unary operations: Recurse to find the source type of the inner expression.
        ExprKind::Unary(_, inner_expr) => infer_source_types(output, hir, inner_expr),

        // Binary operations: recurse on both sides.
        ExprKind::Binary(lhs, _, rhs) => {
            infer_source_types(output, hir, lhs);
            infer_source_types(output, hir, rhs);
        }

        // Ternary: check both branches.
        ExprKind::Ternary(_, true_expr, false_expr) => {
            infer_source_types(output, hir, true_expr);
            infer_source_types(output, hir, false_expr);
        }

        // Complex expressions are not evaluated.
        _ => {}
    }
}

/// Resolves the return type(s) of a function call expression and pushes them to `output`.
///
/// Handles both direct function calls (`foo()`) and member function calls
/// (`contract.foo()`). For overloaded functions, collects return types from all
/// matching overloads to avoid false negatives.
fn resolve_call_return_types(
    output: &mut Vec<ElementaryType>,
    hir: &hir::Hir<'_>,
    call_expr: &hir::Expr<'_>,
) {
    match &call_expr.kind {
        // Direct function call: `foo()`
        ExprKind::Ident(resolutions) => {
            resolve_function_returns(output, hir, resolutions);
        }
        // Member function call: `contract.foo()`
        ExprKind::Member(contract_expr, func_ident) => {
            // Resolve the contract from the base expression.
            let contract_id = match &contract_expr.kind {
                // Variable with contract type: `myContract.foo()`
                ExprKind::Ident([Res::Item(ItemId::Variable(var_id)), ..]) => {
                    if let TypeKind::Custom(ItemId::Contract(cid)) = hir.variable(*var_id).ty.kind {
                        Some(cid)
                    } else {
                        None
                    }
                }
                // Contract type cast: `IFoo(addr).foo()`
                ExprKind::Call(
                    hir::Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
                    ..,
                ) => Some(*cid),
                _ => None,
            };

            if let Some(cid) = contract_id {
                // Find matching functions in the contract by name.
                for item in hir.contract_item_ids(cid) {
                    let Some(fid) = item.as_function() else { continue };
                    let func = hir.function(fid);
                    if func.name.is_some_and(|name| name.as_str() == func_ident.as_str())
                        && func.returns.len() == 1
                    {
                        let ret_var = hir.variable(func.returns[0]);
                        if let TypeKind::Elementary(elem_ty) = &ret_var.ty.kind {
                            output.push(*elem_ty);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Resolves elementary return types from function resolutions.
///
/// For overloaded functions with multiple resolutions, collects return types from all
/// matching overloads to avoid false negatives.
fn resolve_function_returns(
    output: &mut Vec<ElementaryType>,
    hir: &hir::Hir<'_>,
    resolutions: &[Res],
) {
    for res in resolutions {
        if let Res::Item(ItemId::Function(func_id)) = res {
            let func = hir.function(*func_id);
            // Only handle single-return functions.
            if func.returns.len() == 1 {
                let ret_var = hir.variable(func.returns[0]);
                if let TypeKind::Elementary(elem_ty) = &ret_var.ty.kind {
                    output.push(*elem_ty);
                }
            }
        }
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
