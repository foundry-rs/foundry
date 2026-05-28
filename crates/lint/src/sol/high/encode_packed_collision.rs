use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::sym,
    sema::hir::{
        ContractId, ElementaryType, Expr, ExprKind, Hir, ItemId, Res, Type, TypeKind, VariableId,
    },
};

declare_forge_lint!(
    ENCODE_PACKED_COLLISION,
    Severity::High,
    "encode-packed-collision",
    "`abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible"
);

impl<'hir> LateLintPass<'hir> for EncodedPackedCollision {
    fn check_expr(&mut self, ctx: &LintContext, hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return };
        if member.name != sym::encodePacked || !is_abi_builtin(base) {
            return;
        }
        let dynamic_count = args.exprs().filter(|arg| is_dynamic_arg(hir, arg)).count();
        if dynamic_count >= 2 {
            ctx.emit(&ENCODE_PACKED_COLLISION, expr.span);
        }
    }
}

fn is_abi_builtin(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.peel_parens().kind,
        ExprKind::Ident(reses)
            if reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::abi))
    )
}

fn is_dynamic_arg(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    // String literals are always dynamic `string` type.
    if matches!(&expr.peel_parens().kind, ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Str(..)))
    {
        return true;
    }
    let Some(ty) = expr_type(hir, expr) else { return false };
    is_dynamic_type(&ty.kind)
}

const fn is_dynamic_type(kind: &TypeKind<'_>) -> bool {
    match kind {
        TypeKind::Elementary(ElementaryType::String | ElementaryType::Bytes) => true,
        TypeKind::Array(arr) => arr.size.is_none(),
        _ => false,
    }
}

fn expr_type<'hir>(hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) -> Option<&'hir Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            let var_id = var_resolution(resolutions)?;
            Some(&hir.variable(var_id).ty)
        }
        ExprKind::Call(callee, _, _) => call_return_type(hir, callee),
        ExprKind::Index(base, _) => match &expr_type(hir, base)?.kind {
            TypeKind::Array(array) => Some(&array.element),
            TypeKind::Mapping(mapping) => Some(&mapping.value),
            _ => None,
        },
        ExprKind::Member(base, member) => {
            let base_ty = expr_type(hir, base)?;
            let TypeKind::Custom(ItemId::Struct(sid)) = &base_ty.kind else { return None };
            hir.strukt(*sid)
                .fields
                .iter()
                .find(|&&fid| hir.variable(fid).name.is_some_and(|n| n.name == member.name))
                .map(|&fid| &hir.variable(fid).ty)
        }
        _ => None,
    }
}

fn call_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
) -> Option<&'hir Type<'hir>> {
    match &callee.peel_parens().kind {
        // Type cast: bytes(x), string(x) — the result type is the cast target
        ExprKind::Type(ty) => Some(ty),
        // Direct function call: getString(), getBytes()
        // If multiple function resolutions exist (overloads), we can't determine which was
        // called without argument types, return None to avoid false positives.
        ExprKind::Ident(resolutions) => {
            let fids: Vec<_> = resolutions
                .iter()
                .filter_map(|r| {
                    if let Res::Item(ItemId::Function(fid)) = r { Some(*fid) } else { None }
                })
                .collect();
            let [fid] = fids.as_slice() else { return None };
            let [ret] = hir.function(*fid).returns else { return None };
            Some(&hir.variable(*ret).ty)
        }
        // Member call: token.name(), token.symbol(), etc.
        // A contract may expose multiple entries for the same method name when a derived contract
        // overrides an inherited function. In that case all candidates share the same return type,
        // so we accept the match as long as every candidate agrees. If they disagree (genuine
        // overloads with different return types) we bail to avoid false positives.
        ExprKind::Member(recv, method) => {
            let cid = contract_id_of(hir, recv)?;
            let matches: Vec<_> = hir
                .contract_item_ids(cid)
                .filter_map(|item| {
                    let fid = item.as_function()?;
                    let f = hir.function(fid);
                    if f.name.is_some_and(|n| n.name == method.name) {
                        let [ret] = f.returns else { return None };
                        Some(&hir.variable(*ret).ty)
                    } else {
                        None
                    }
                })
                .collect();
            let [first, rest @ ..] = matches.as_slice() else { return None };
            // All candidates must agree on dynamic-ness; if any disagrees it's a genuine overload
            // and we cannot determine which was called.
            if rest.iter().any(|ty| is_dynamic_type(&ty.kind) != is_dynamic_type(&first.kind)) {
                return None;
            }
            Some(*first)
        }
        // Indirect call via a function-typed value
        _ => match &expr_type(hir, callee)?.kind {
            TypeKind::Function(f) => {
                let [ret] = f.returns else { return None };
                Some(&hir.variable(*ret).ty)
            }
            _ => None,
        },
    }
}

fn contract_id_of(hir: &Hir<'_>, expr: &Expr<'_>) -> Option<ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => match hir.variable(*vid).ty.kind {
                TypeKind::Custom(ItemId::Contract(cid)) => Some(cid),
                _ => None,
            },
            Res::Item(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        }),
        _ => None,
    }
}

fn var_resolution(resolutions: &[Res]) -> Option<VariableId> {
    resolutions
        .iter()
        .find_map(|r| if let Res::Item(ItemId::Variable(vid)) = r { Some(*vid) } else { None })
}
