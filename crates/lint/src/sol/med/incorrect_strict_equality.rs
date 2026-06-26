use super::IncorrectStrictEquality;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, ContractKind},
    interface::sym,
    sema::{
        Gcx, Hir,
        hir::{ElementaryType, Expr, ExprKind, ItemId, Res, StructId, Type, TypeKind},
    },
};

declare_forge_lint!(
    INCORRECT_STRICT_EQUALITY,
    Severity::Med,
    "incorrect-strict-equality",
    "dangerous strict equality check on an externally-influenced value"
);

impl<'hir> LateLintPass<'hir> for IncorrectStrictEquality {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        expr: &'hir Expr<'hir>,
    ) {
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
            && (contains_externally_influenced(hir, lhs)
                || contains_externally_influenced(hir, rhs))
        {
            ctx.emit(&INCORRECT_STRICT_EQUALITY, expr.span);
        }
    }
}

/// Recursively checks whether an expression tree contains an externally-influenced
/// balance read. This makes the lint fire on cases like
/// `address(this).balance + 1 == target` or `target == token.balanceOf(address(this)) - 1`.
fn contains_externally_influenced<'hir>(hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> bool {
    let expr = expr.peel_parens();
    if is_externally_influenced(hir, expr) {
        return true;
    }
    match &expr.kind {
        ExprKind::Unary(_, inner) => contains_externally_influenced(hir, inner),
        ExprKind::Binary(lhs, _, rhs) => {
            contains_externally_influenced(hir, lhs) || contains_externally_influenced(hir, rhs)
        }
        ExprKind::Ternary(_, then, else_) => {
            contains_externally_influenced(hir, then) || contains_externally_influenced(hir, else_)
        }
        ExprKind::Call(_, args, _) => args.exprs().any(|a| contains_externally_influenced(hir, a)),
        _ => false,
    }
}

/// Returns `true` if `expr` is `<address>.balance` or `<expr>.balanceOf(...)`.
fn is_externally_influenced<'hir>(hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> bool {
    match &expr.peel_parens().kind {
        // `<expr>.balance`, only flag when we can prove the receiver is an `address`.
        // Otherwise any user-defined struct field named `balance` would trigger this lint.
        ExprKind::Member(base, member) => {
            member.as_str() == "balance" && is_address_expr(hir, base)
        }

        // `<expr>.balanceOf(...)`, ERC-20 style external call. We match by name, since
        // `balanceOf` is overwhelmingly an ERC-20 / token method.
        // Skip calls where the receiver resolves to a library to avoid false positives
        // on internal library helpers named `balanceOf`.
        ExprKind::Call(callee, _, _) => {
            if let ExprKind::Member(base, m) = &callee.peel_parens().kind
                && m.as_str() == "balanceOf"
            {
                // Skip if the receiver resolves to a library contract.
                !matches!(&base.peel_parens().kind, ExprKind::Ident(reses) if reses.iter().any(|r| {
                    matches!(r, Res::Item(ItemId::Contract(cid)) if hir.contract(*cid).kind == ContractKind::Library)
                }))
            } else {
                false
            }
        }

        _ => false,
    }
}

/// Conservatively returns `true` if `expr` is provably of type `address`
/// (or `address payable`).
///
/// Returning `false` simply skips the lint, so being conservative is preferred over
/// being exhaustive (see `docs/incorrect-strict-equality.md`).
fn is_address_expr<'hir>(hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        // `payable(x)` always returns `address payable`.
        ExprKind::Payable(_) => true,

        // `address(x)` cast, or a function call whose single return type is address.
        ExprKind::Call(callee, _, _) => {
            let callee = callee.peel_parens();
            // Type cast: `address(x)` / `address payable(x)`.
            if matches!(
                &callee.kind,
                ExprKind::Type(Type { kind: TypeKind::Elementary(ElementaryType::Address(_)), .. })
            ) {
                return true;
            }
            // Function call returning a single `address`.
            if let ExprKind::Ident(reses) = &callee.kind {
                return reses.iter().any(|r| {
                    if let Res::Item(ItemId::Function(fid)) = r {
                        let func = hir.function(*fid);
                        if let [ret] = func.returns {
                            return matches!(
                                hir.variable(*ret).ty.kind,
                                TypeKind::Elementary(ElementaryType::Address(_))
                            );
                        }
                    }
                    false
                });
            }
            false
        }

        // Identifier resolving to a variable declared as `address` / `address payable`.
        ExprKind::Ident(reses) => reses.iter().any(|r| {
            matches!(
                r,
                Res::Item(ItemId::Variable(vid))
                    if matches!(
                        hir.variable(*vid).ty.kind,
                        TypeKind::Elementary(ElementaryType::Address(_))
                    )
            )
        }),

        ExprKind::Member(base, member) => {
            let name = member.as_str();
            // Built-in members that return `address`: `msg.sender`, `tx.origin`, `block.coinbase`.
            if let ExprKind::Ident(reses) = &base.peel_parens().kind {
                let is_builtin = reses.iter().any(|r| {
                    matches!(
                        r,
                        Res::Builtin(b) if {
                            let base_sym = b.name();
                            (base_sym == sym::msg && name == "sender")
                                || (base_sym == sym::tx && name == "origin")
                                || (base_sym == sym::block && name == "coinbase")
                        }
                    )
                });
                if is_builtin {
                    return true;
                }
            }
            // Struct field whose declared type is `address` (e.g. `holder.owner`).
            matches!(struct_field_type(hir, base, name), Some(ElementaryType::Address(_)))
        }

        // Indexing into an array/mapping of `address` (e.g. `holders[i]`).
        ExprKind::Index(base, _) => {
            matches!(indexed_element_type(hir, base), Some(ElementaryType::Address(_)))
        }

        _ => false,
    }
}

/// Resolves the declared elementary type of `field_name` on `base`, when `base` is
/// known to be a struct value.
fn struct_field_type<'hir>(
    hir: &'hir Hir<'hir>,
    base: &Expr<'hir>,
    field_name: &str,
) -> Option<ElementaryType> {
    let strukt_id = struct_of(hir, base)?;
    let strukt = hir.strukt(strukt_id);
    for fid in strukt.fields {
        let v = hir.variable(*fid);
        if let Some(name) = v.name
            && name.as_str() == field_name
            && let TypeKind::Elementary(elem) = v.ty.kind
        {
            return Some(elem);
        }
    }
    None
}

/// Returns the element type of `base` when it is an array or the value type when it is
/// a mapping, restricted to elementary types.
fn indexed_element_type<'hir>(hir: &'hir Hir<'hir>, base: &Expr<'hir>) -> Option<ElementaryType> {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return None };
    let var = reses.iter().find_map(|r| match r {
        Res::Item(ItemId::Variable(vid)) => Some(hir.variable(*vid)),
        _ => None,
    })?;
    match &var.ty.kind {
        TypeKind::Array(arr) => match arr.element.kind {
            TypeKind::Elementary(elem) => Some(elem),
            _ => None,
        },
        TypeKind::Mapping(m) => match m.value.kind {
            TypeKind::Elementary(elem) => Some(elem),
            _ => None,
        },
        _ => None,
    }
}

/// Returns the [`StructId`] of `expr` when it is a (possibly chained) struct value.
fn struct_of<'hir>(hir: &'hir Hir<'hir>, expr: &Expr<'hir>) -> Option<StructId> {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Ident(reses) => reses.iter().find_map(|r| match r {
            Res::Item(ItemId::Variable(vid)) => match hir.variable(*vid).ty.kind {
                TypeKind::Custom(ItemId::Struct(sid)) => Some(sid),
                _ => None,
            },
            _ => None,
        }),
        ExprKind::Member(inner, member) => {
            let strukt_id = struct_of(hir, inner)?;
            let strukt = hir.strukt(strukt_id);
            for fid in strukt.fields {
                let v = hir.variable(*fid);
                if let Some(name) = v.name
                    && name.as_str() == member.as_str()
                    && let TypeKind::Custom(ItemId::Struct(sid)) = v.ty.kind
                {
                    return Some(sid);
                }
            }
            None
        }
        _ => None,
    }
}
