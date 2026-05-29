use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{Ident, Symbol, sym},
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
        // Only non-literal dynamic args count: a top-level string/hex/unicode literal is a
        // compile-time constant. With at most one non-literal dynamic arg the packed encoding
        // is still injective, so there is no collision risk.
        let dynamic_count = args
            .exprs()
            .filter(|arg| {
                !matches!(
                    arg.peel_parens().kind,
                    ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Str(..))
                ) && is_dynamic_arg(hir, arg)
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

fn is_dynamic_arg(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        // String literals (and multi-line/hex string sequences) are always dynamic `string` type.
        ExprKind::Lit(lit) if matches!(lit.kind, ast::LitKind::Str(..)) => true,
        // Ternary: dynamic when both branches are dynamic. Handled here so that literal branches
        // (which have no expr_type) are correctly identified as dynamic.
        ExprKind::Ternary(_, then, else_) => {
            is_dynamic_arg(hir, then) && is_dynamic_arg(hir, else_)
        }
        // Calls: check well-known builtins that return bytes/string, then generic path.
        ExprKind::Call(callee, args, _) => is_dynamic_call(hir, callee, args.exprs().count()),
        // Member access: prefer the resolved type so user-defined struct fields named like
        // builtin properties (e.g. `code`) do not get treated as dynamic bytes.
        ExprKind::Member(base, member) => {
            if let Some(ty) = expr_type(hir, expr) {
                is_dynamic_type(&ty.kind)
            } else {
                is_dynamic_builtin_member(base, member)
            }
        }
        _ => expr_type(hir, expr).is_some_and(|ty| is_dynamic_type(&ty.kind)),
    }
}

/// Returns `true` when `callee(args)` is statically known to return a dynamic type.
fn is_dynamic_call<'hir>(hir: &'hir Hir<'hir>, callee: &'hir Expr<'hir>, n_args: usize) -> bool {
    let callee = callee.peel_parens();
    match &callee.kind {
        // `new bytes(n)` / `new string(n)`: dynamic allocation.
        ExprKind::New(ty) => return is_dynamic_type(&ty.kind),
        ExprKind::Member(recv, method) => {
            // abi.encode / abi.encodePacked / abi.encodeWithSelector / … -> bytes
            if is_abi_builtin(recv) && is_abi_encode_method(method.name) {
                return true;
            }
            // string.concat(…) -> string, bytes.concat(…) -> bytes
            if method.name == sym::concat
                && let ExprKind::Type(ty) = &recv.peel_parens().kind
                && is_dynamic_type(&ty.kind)
            {
                return true;
            }
        }
        _ => {}
    }
    call_return_type(hir, callee, Some(n_args)).is_some_and(|ty| is_dynamic_type(&ty.kind))
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
        ExprKind::Call(callee, args, _) => {
            call_return_type(hir, callee, Some(args.exprs().count()))
        }
        ExprKind::Index(base, _) => match &expr_type(hir, base)?.kind {
            TypeKind::Array(array) => Some(&array.element),
            TypeKind::Mapping(mapping) => Some(&mapping.value),
            _ => None,
        },
        // Slice expressions (e.g. `data[:4]`) preserve the base type.
        ExprKind::Slice(base, ..) => expr_type(hir, base),
        ExprKind::Member(base, member) => {
            let base_ty = expr_type(hir, base)?;
            let TypeKind::Custom(ItemId::Struct(sid)) = &base_ty.kind else { return None };
            hir.strukt(*sid)
                .fields
                .iter()
                .find(|&&fid| hir.variable(fid).name.is_some_and(|n| n.name == member.name))
                .map(|&fid| &hir.variable(fid).ty)
        }
        // Ternary: use then-branch type if both branches agree on dynamic-ness.
        ExprKind::Ternary(_, then, else_) => {
            let then_ty = expr_type(hir, then)?;
            let else_ty = expr_type(hir, else_)?;
            (is_dynamic_type(&then_ty.kind) == is_dynamic_type(&else_ty.kind)).then_some(then_ty)
        }
        _ => None,
    }
}

fn call_return_type<'hir>(
    hir: &'hir Hir<'hir>,
    callee: &'hir Expr<'hir>,
    // Number of arguments in the outer call; used to disambiguate same-name overloads by arity.
    n_call_args: Option<usize>,
) -> Option<&'hir Type<'hir>> {
    match &callee.peel_parens().kind {
        // Type cast: bytes(x), string(x); the result type is the cast target
        ExprKind::Type(ty) => Some(ty),
        // Direct function call: getString(), getBytes()
        // If multiple function resolutions remain after arity filtering we can't determine which
        // overload was called, so return None to avoid false positives.
        ExprKind::Ident(resolutions) => {
            let fids: Vec<_> = resolutions
                .iter()
                .filter_map(|r| {
                    if let Res::Item(ItemId::Function(fid)) = r {
                        let f = hir.function(*fid);
                        n_call_args.is_none_or(|n| f.parameters.len() == n).then_some(*fid)
                    } else {
                        None
                    }
                })
                .collect();
            let [fid] = fids.as_slice() else { return None };
            let [ret] = hir.function(*fid).returns else { return None };
            Some(&hir.variable(*ret).ty)
        }
        // Member call: token.name(), token.symbol(), etc.
        // A contract may expose multiple entries for the same method name when a derived contract
        // overrides an inherited function. In that case all candidates share the same return type,
        // so we accept the match as long as every candidate agrees on dynamic-ness. If they
        // disagree (genuine overloads with different return types) we bail to avoid false
        // positives. Arity filtering further narrows candidates before the agreement check.
        ExprKind::Member(recv, method) => {
            let cid = contract_id_of(hir, recv)?;
            let matches: Vec<_> = hir
                .contract_item_ids(cid)
                .filter_map(|item| {
                    let fid = item.as_function()?;
                    let f = hir.function(fid);
                    let name_matches = f.name.is_some_and(|n| n.name == method.name);
                    let arity_matches = n_call_args.is_none_or(|n| f.parameters.len() == n);
                    if name_matches && arity_matches {
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
        // Bare identifier resolved to a contract variable or contract type.
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => match hir.variable(*vid).ty.kind {
                TypeKind::Custom(ItemId::Contract(cid)) => Some(cid),
                _ => None,
            },
            Res::Item(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        }),
        // Interface/contract cast: IERC20Metadata(addr) or MyContract(addr).
        // The callee can be either an Ident resolving to a contract item or a Type node.
        ExprKind::Call(callee, ..) => match &callee.peel_parens().kind {
            ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
                Res::Item(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            }),
            ExprKind::Type(ty) => match &ty.kind {
                TypeKind::Custom(ItemId::Contract(cid)) => Some(*cid),
                _ => None,
            },
            _ => None,
        },
        // General fallback: compute the expression's type.
        // This covers struct field access (cfg.token) and array index access (tokens[i]).
        _ => match &expr_type(hir, expr)?.kind {
            TypeKind::Custom(ItemId::Contract(cid)) => Some(*cid),
            _ => None,
        },
    }
}

fn var_resolution(resolutions: &[Res]) -> Option<VariableId> {
    resolutions
        .iter()
        .find_map(|r| if let Res::Item(ItemId::Variable(vid)) = r { Some(*vid) } else { None })
}
