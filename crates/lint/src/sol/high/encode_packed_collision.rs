use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{Ident, Symbol, sym},
    sema::{
        Gcx,
        hir::{ElementaryType, Expr, ExprKind, Hir, Res, TypeKind},
        ty::{Ty, TyKind},
    },
};

declare_forge_lint!(
    ENCODE_PACKED_COLLISION,
    Severity::High,
    "encode-packed-collision",
    "`abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible"
);

impl<'hir> LateLintPass<'hir> for EncodedPackedCollision {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir Hir<'hir>,
        expr: &'hir Expr<'hir>,
    ) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return };
        if member.name != sym::encodePacked || !is_abi_builtin(base) {
            return;
        }
        // Only non-literal dynamic args count: a top-level string/hex/unicode literal is a
        // compile-time constant. With at most one non-literal dynamic arg the packed encoding
        // is still injective, so there is no collision risk.
        let dynamic_count = args
            .exprs()
            .filter(|arg| {
                !matches!(
                    arg.peel_parens().kind,
                    ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Str(..))
                ) && is_dynamic_arg(gcx, arg)
            })
            .count();
        if dynamic_count >= 2 {
            ctx.emit(&ENCODE_PACKED_COLLISION, expr.span);
        }
    }
}

fn is_abi_builtin(expr: &Expr<'_>) -> bool {
    is_builtin_named(expr, sym::abi)
}

fn is_dynamic_arg<'hir>(gcx: Gcx<'hir>, expr: &'hir Expr<'hir>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        // String literals (and multi-line/hex string sequences) are always dynamic `string` type.
        ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Str(..)) => true,
        // Ternary: dynamic when both branches are dynamic. Handled here so that literal branches
        // (which have no expr_type) are correctly identified as dynamic.
        ExprKind::Ternary(_, then, else_) => {
            is_dynamic_arg(gcx, then) && is_dynamic_arg(gcx, else_)
        }
        // Calls: check well-known builtins that return bytes/string, then generic path.
        ExprKind::Call(callee, _, _) => {
            is_dynamic_call(callee)
                || gcx.type_of_expr(expr.id).is_some_and(ty_is_dynamic_bytes_string_or_array)
        }
        // Member access: prefer the resolved type so user-defined struct fields named like
        // builtin properties (e.g. `code`) do not get treated as dynamic bytes.
        ExprKind::Member(base, member) => {
            if let Some(ty) = gcx.type_of_expr(expr.id) {
                ty_is_dynamic_bytes_string_or_array(ty)
            } else {
                is_dynamic_builtin_member(base, member)
            }
        }
        _ => gcx.type_of_expr(expr.id).is_some_and(ty_is_dynamic_bytes_string_or_array),
    }
}

fn ty_is_dynamic_bytes_string_or_array(ty: Ty<'_>) -> bool {
    matches!(
        ty.peel_refs().kind,
        TyKind::Elementary(ElementaryType::Bytes | ElementaryType::String)
            | TyKind::DynArray(_)
            | TyKind::Slice(_)
    )
}

/// Returns `true` when `callee(args)` is statically known to return a dynamic type.
fn is_dynamic_call(callee: &Expr<'_>) -> bool {
    let callee = callee.peel_parens();
    match &callee.kind {
        // `new bytes(n)` / `new string(n)`: dynamic allocation.
        ExprKind::New(ty) => return is_dynamic_hir_type(&ty.kind),
        ExprKind::Member(recv, method) => {
            // abi.encode / abi.encodePacked / abi.encodeWithSelector / … -> bytes
            if is_abi_builtin(recv) && is_abi_encode_method(method.name) {
                return true;
            }
            // string.concat(…) -> string, bytes.concat(…) -> bytes
            if method.name == sym::concat
                && let ExprKind::Type(ty) = &recv.peel_parens().kind
                && is_dynamic_hir_type(&ty.kind)
            {
                return true;
            }
        }
        _ => {}
    }
    false
}

/// Returns `true` when `base.member` is a well-known builtin property of dynamic bytes type.
fn is_dynamic_builtin_member(base: &Expr<'_>, member: &Ident) -> bool {
    match member.name {
        // msg.data -> bytes calldata
        n if n == sym::data => is_builtin_named(base, sym::msg),
        // <any>.code, type(C).creationCode, type(C).runtimeCode -> bytes
        n if n == sym::code || n == sym::creationCode || n == sym::runtimeCode => true,
        _ => false,
    }
}

fn is_abi_encode_method(name: Symbol) -> bool {
    name == sym::encode
        || name == sym::encodePacked
        || name == sym::encodeWithSelector
        || name == sym::encodeWithSignature
        || name == sym::encodeCall
}

fn is_builtin_named(expr: &Expr<'_>, name: Symbol) -> bool {
    matches!(&expr.peel_parens().kind,
        ExprKind::Ident(reses) if reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == name))
    )
}

const fn is_dynamic_hir_type(kind: &TypeKind<'_>) -> bool {
    match kind {
        TypeKind::Elementary(ElementaryType::String | ElementaryType::Bytes) => true,
        TypeKind::Array(arr) => arr.size.is_none(),
        _ => false,
    }
}
