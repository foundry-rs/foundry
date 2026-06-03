use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{LitKind, StrKind},
    interface::sym,
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

fn infer_source_types(output: &mut Vec<ElementaryType>, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) {
    match &expr.kind {
        ExprKind::Call(call_expr, args, ..) => {
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(..), .. }) =
                &call_expr.kind
                && let Some(inner) = args.exprs().next()
            {
                infer_source_types(output, hir, inner);
                return;
            }

            resolve_call_return_types(output, hir, call_expr, args.len());
        }

        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(var_id))) = resolutions.first() {
                let variable = hir.variable(*var_id);
                if let TypeKind::Elementary(elem_type) = &variable.ty.kind {
                    output.push(*elem_type);
                }
            }
        }

        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => output.push(ElementaryType::Bytes),
            LitKind::Str(..) => output.push(ElementaryType::String),
            LitKind::Address(_) => output.push(ElementaryType::Address(false)),
            LitKind::Bool(_) => output.push(ElementaryType::Bool),
            _ => {}
        },

        ExprKind::Unary(_, inner_expr) => infer_source_types(output, hir, inner_expr),

        ExprKind::Binary(lhs, _, rhs) => {
            infer_branch_type(output, hir, lhs);
            infer_branch_type(output, hir, rhs);
        }

        ExprKind::Ternary(_, true_expr, false_expr) => {
            infer_branch_type(output, hir, true_expr);
            infer_branch_type(output, hir, false_expr);
        }

        ExprKind::Member(base, member) => {
            if let Some(elem_ty) = resolve_member_elem_type(hir, base, member) {
                output.push(elem_ty);
            }
        }

        ExprKind::Index(_, _) => {
            if let Some(elem_ty) = resolve_index_elem_type(hir, expr) {
                output.push(elem_ty);
            }
        }

        _ => {}
    }
}

fn infer_branch_type(output: &mut Vec<ElementaryType>, hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) {
    if let ExprKind::Call(call_expr, _, ..) = &expr.kind
        && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call_expr.kind
    {
        output.push(*ty);
        return;
    }
    infer_source_types(output, hir, expr);
}

fn resolve_call_return_types(
    output: &mut Vec<ElementaryType>,
    hir: &hir::Hir<'_>,
    call_expr: &hir::Expr<'_>,
    args_count: usize,
) {
    match &call_expr.kind {
        ExprKind::Ident(resolutions) => {
            resolve_function_returns(output, hir, resolutions);
        }
        ExprKind::Member(base_expr, func_ident) => {
            let contract_id = match &base_expr.kind {
                ExprKind::Ident(reses) => {
                    if let Some(Res::Item(ItemId::Variable(var_id))) = reses.first() {
                        if let TypeKind::Custom(ItemId::Contract(cid)) =
                            hir.variable(*var_id).ty.kind
                        {
                            Some(cid)
                        } else {
                            None
                        }
                    } else if let Some(Res::Item(ItemId::Contract(cid))) = reses.first() {
                        Some(*cid)
                    } else {
                        None
                    }
                }
                ExprKind::Call(
                    hir::Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
                    ..,
                ) => Some(*cid),
                _ => None,
            };

            if let Some(cid) = contract_id {
                let mut candidates: Vec<ElementaryType> = Vec::new();
                for item in hir.contract_item_ids(cid) {
                    let Some(fid) = item.as_function() else { continue };
                    let func = hir.function(fid);
                    if func.name.is_some_and(|name| name.as_str() == func_ident.as_str())
                        && func.parameters.len() == args_count
                        && func.returns.len() == 1
                    {
                        let ret_var = hir.variable(func.returns[0]);
                        if let TypeKind::Elementary(elem_ty) = &ret_var.ty.kind {
                            candidates.push(*elem_ty);
                        }
                    }
                }
                if let Some(&first) = candidates.first() {
                    if candidates.iter().all(|t| t == &first) {
                        output.push(first);
                    }
                }
            }
        }
        _ => {}
    }
}

fn resolve_function_returns(
    output: &mut Vec<ElementaryType>,
    hir: &hir::Hir<'_>,
    resolutions: &[Res],
) {
    for res in resolutions {
        if let Res::Item(ItemId::Function(func_id)) = res {
            let func = hir.function(*func_id);
            if func.returns.len() == 1 {
                let ret_var = hir.variable(func.returns[0]);
                if let TypeKind::Elementary(elem_ty) = &ret_var.ty.kind {
                    output.push(*elem_ty);
                }
            }
        }
    }
}

fn resolve_expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &hir::Expr<'_>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var_id = reses.iter().find_map(|r| {
                if let Res::Item(ItemId::Variable(vid)) = r { Some(*vid) } else { None }
            })?;
            Some(&hir.variable(var_id).ty)
        }
        ExprKind::Index(base, _) => {
            let base_ty = resolve_expr_type(hir, base)?;
            match &base_ty.kind {
                TypeKind::Array(arr) => Some(&arr.element),
                TypeKind::Mapping(m) => Some(&m.value),
                _ => None,
            }
        }
        ExprKind::Member(base, member) => {
            let base_ty = resolve_expr_type(hir, base)?;
            if let TypeKind::Custom(ItemId::Struct(sid)) = &base_ty.kind {
                return hir
                    .strukt(*sid)
                    .fields
                    .iter()
                    .map(|&fid| hir.variable(fid))
                    .find(|f| f.name.is_some_and(|n| n.as_str() == member.as_str()))
                    .map(|f| &f.ty);
            }
            None
        }
        _ => None,
    }
}

fn resolve_member_elem_type(
    hir: &hir::Hir<'_>,
    base: &hir::Expr<'_>,
    member: &solar::interface::Ident,
) -> Option<ElementaryType> {
    let member_name = member.as_str();

    if let ExprKind::Ident(reses) = &base.kind {
        if let Some(Res::Builtin(builtin)) = reses.first() {
            let builtin_name = builtin.name();

            if builtin_name == sym::block {
                return match member_name {
                    "timestamp" | "number" | "basefee" | "chainid" | "gaslimit" | "prevrandao"
                    | "difficulty" | "blobbasefee" => Some(ElementaryType::UInt(uint_size(256))),
                    _ => None,
                };
            }

            if builtin_name == sym::msg {
                return match member_name {
                    "value" | "gas" => Some(ElementaryType::UInt(uint_size(256))),
                    "sender" => Some(ElementaryType::Address(false)),
                    _ => None,
                };
            }

            if builtin_name == sym::tx {
                return match member_name {
                    "gasprice" => Some(ElementaryType::UInt(uint_size(256))),
                    _ => None,
                };
            }

            return None;
        }
    }

    let base_ty = resolve_expr_type(hir, base)?;

    match member_name {
        "balance" => {
            if matches!(&base_ty.kind, TypeKind::Elementary(ElementaryType::Address(_))) {
                return Some(ElementaryType::UInt(uint_size(256)));
            }
        }
        "length" => match &base_ty.kind {
            TypeKind::Array(arr) if arr.size.is_none() => {
                return Some(ElementaryType::UInt(uint_size(256)));
            }
            TypeKind::Elementary(ElementaryType::Bytes) => {
                return Some(ElementaryType::UInt(uint_size(256)));
            }
            TypeKind::Elementary(ElementaryType::FixedBytes(_)) => {
                return Some(ElementaryType::UInt(uint_size(8)));
            }
            _ => {}
        },
        "selector" => {
            if matches!(&base_ty.kind, TypeKind::Function(_)) {
                return Some(ElementaryType::FixedBytes(fixed_bytes_size(4)));
            }
        }
        _ => {}
    }

    if let TypeKind::Custom(ItemId::Struct(struct_id)) = &base_ty.kind {
        return hir
            .strukt(*struct_id)
            .fields
            .iter()
            .map(|&field_id| hir.variable(field_id))
            .find(|field| field.name.is_some_and(|n| n.as_str() == member_name))
            .and_then(
                |field| {
                    if let TypeKind::Elementary(e) = &field.ty.kind { Some(*e) } else { None }
                },
            );
    }

    None
}

fn resolve_index_elem_type(
    hir: &hir::Hir<'_>,
    index_expr: &hir::Expr<'_>,
) -> Option<ElementaryType> {
    let base_ty = resolve_expr_type(hir, index_expr)?;
    if let TypeKind::Elementary(e) = &base_ty.kind { Some(*e) } else { None }
}

fn uint_size(bits: u16) -> solar::ast::TypeSize {
    solar::ast::TypeSize::new_int_bits(bits)
}

fn fixed_bytes_size(bytes: u8) -> solar::ast::TypeSize {
    solar::ast::TypeSize::new_fb_bytes(bytes)
}

const fn is_unsafe_elementary_typecast(
    source_type: &ElementaryType,
    target_type: &ElementaryType,
) -> bool {
    match (source_type, target_type) {
        (ElementaryType::UInt(source_size), ElementaryType::UInt(target_size))
        | (ElementaryType::Int(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() > target_size.bits()
        }

        (ElementaryType::Int(_), ElementaryType::UInt(_)) => true,

        (ElementaryType::UInt(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() >= target_size.bits()
        }

        (ElementaryType::FixedBytes(source_size), ElementaryType::FixedBytes(target_size)) => {
            source_size.bytes() > target_size.bytes()
        }

        (ElementaryType::Bytes | ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}
