use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::sym,
    sema::hir::{ElementaryType, Expr, ExprKind, Hir, ItemId, Res, Type, TypeKind, VariableId},
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
        ExprKind::Ident(resolutions) => {
            let fid = resolutions.iter().find_map(|r| {
                if let Res::Item(ItemId::Function(fid)) = r { Some(*fid) } else { None }
            })?;
            let [ret] = hir.function(fid).returns else { return None };
            Some(&hir.variable(*ret).ty)
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

fn var_resolution(resolutions: &[Res]) -> Option<VariableId> {
    resolutions
        .iter()
        .find_map(|r| if let Res::Item(ItemId::Variable(vid)) = r { Some(*vid) } else { None })
}
