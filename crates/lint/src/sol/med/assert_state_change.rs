use super::AssertStateChange;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::UnOpKind,
    interface::{Span, kw, sym},
    sema::{
        Hir,
        hir::{ContractId, Expr, ExprKind, FunctionId, ItemId, Res, TypeKind},
    },
};

declare_forge_lint!(
    ASSERT_STATE_CHANGE,
    Severity::Med,
    "assert-state-change",
    "assert() should not contain state-modifying expressions"
);

impl<'hir> LateLintPass<'hir> for AssertStateChange {
    fn check_expr(&mut self, ctx: &LintContext, hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        if !is_assert(callee) {
            return;
        }

        for arg in args.exprs() {
            if let Some(span) = find_state_change(hir, arg) {
                ctx.emit_with_msg(
                    &ASSERT_STATE_CHANGE,
                    span,
                    "assert() argument contains a state-modifying expression; \
                     assert() is for invariants, hoist the mutation before the assert, \
                     or use require() for validation",
                );
            }
        }
    }
}

fn is_assert(callee: &Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.kind else { return false };
    reses.iter().any(|r| matches!(r, Res::Builtin(b) if b.name() == sym::assert))
}

/// Recursively searches `expr` for the first sub-expression that modifies state.
/// Returns its span so the diagnostic points at exactly where the mutation occurs.
fn find_state_change<'hir>(hir: &Hir<'hir>, expr: &'hir Expr<'hir>) -> Option<Span> {
    match &expr.kind {
        // x = y, x += y, etc., only when the lvalue targets a state variable
        ExprKind::Assign(lhs, _, rhs) => {
            if lvalue_is_state_var(hir, lhs) {
                return Some(expr.span);
            }
            find_state_change(hir, lhs).or_else(|| find_state_change(hir, rhs))
        }

        // delete x, only when x is a state variable
        ExprKind::Delete(inner) => {
            if lvalue_is_state_var(hir, inner) {
                return Some(expr.span);
            }
            find_state_change(hir, inner)
        }

        // ++x, x++, --x, x--, only when x is a state variable
        ExprKind::Unary(op, inner)
            if matches!(
                op.kind,
                UnOpKind::PreInc | UnOpKind::PostInc | UnOpKind::PreDec | UnOpKind::PostDec
            ) =>
        {
            if lvalue_is_state_var(hir, inner) {
                return Some(expr.span);
            }
            find_state_change(hir, inner)
        }

        ExprKind::Call(callee, args, named_args) => {
            // arr.push(...) / arr.pop() on a state variable are mutations
            if let ExprKind::Member(base, method) = &callee.kind
                && (method.name == sym::push || method.name.as_str() == "pop")
                && lvalue_is_state_var(hir, base)
            {
                return Some(expr.span);
            }

            // Known always-mutating member calls: .call(), .delegatecall(), .send(), .transfer()
            if let ExprKind::Member(_, method) = &callee.kind {
                let n = method.name;
                if n == kw::Call || n == kw::Delegatecall || n == sym::send || n == sym::transfer {
                    return Some(expr.span);
                }
            }

            // Resolvable contract member calls: check mutates_state() via HIR.
            // We collect all overloads with the same name and arity, then only flag
            // when every candidate mutates state to avoid FP from view overloads.
            let candidates = resolve_member_overloads(hir, callee, args.len());
            if !candidates.is_empty()
                && candidates.iter().all(|&fid| hir.function(fid).mutates_state())
            {
                return Some(expr.span);
            }

            // Bare-identifier internal function calls: same all-must-mutate policy.
            let reses = match &callee.peel_parens().kind {
                ExprKind::Ident(r) => *r,
                _ => &[],
            };
            let fn_reses: Vec<FunctionId> = reses
                .iter()
                .filter_map(|res| {
                    if let Res::Item(ItemId::Function(fid)) = res { Some(*fid) } else { None }
                })
                .collect();
            if !fn_reses.is_empty() && fn_reses.iter().all(|&fid| hir.function(fid).mutates_state())
            {
                return Some(expr.span);
            }

            // Recurse into callee, positional args, and named args
            find_state_change(hir, callee)
                .or_else(|| args.exprs().find_map(|a| find_state_change(hir, a)))
                .or_else(|| {
                    named_args
                        .iter()
                        .flat_map(|na| na.iter())
                        .find_map(|na| find_state_change(hir, &na.value))
                })
        }

        ExprKind::Unary(_, inner) | ExprKind::Member(inner, _) | ExprKind::Payable(inner) => {
            find_state_change(hir, inner)
        }
        ExprKind::Binary(lhs, _, rhs) => {
            find_state_change(hir, lhs).or_else(|| find_state_change(hir, rhs))
        }
        ExprKind::Ternary(cond, t, f) => find_state_change(hir, cond)
            .or_else(|| find_state_change(hir, t))
            .or_else(|| find_state_change(hir, f)),
        ExprKind::Index(base, idx) => {
            find_state_change(hir, base).or_else(|| idx.and_then(|i| find_state_change(hir, i)))
        }
        ExprKind::Slice(base, start, end) => find_state_change(hir, base)
            .or_else(|| start.and_then(|s| find_state_change(hir, s)))
            .or_else(|| end.and_then(|e| find_state_change(hir, e))),
        ExprKind::Array(exprs) => exprs.iter().find_map(|e| find_state_change(hir, e)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().copied().flatten().find_map(|e| find_state_change(hir, e))
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => None,
    }
}

/// Returns all overloads of the called member function that match the call's argument count.
/// Matching by arity narrows overload candidates so the caller can apply an all-must-mutate
/// policy and avoid flagging call sites that resolve to a view overload.
fn resolve_member_overloads<'hir>(
    hir: &Hir<'hir>,
    callee: &'hir Expr<'hir>,
    arg_count: usize,
) -> Vec<FunctionId> {
    let ExprKind::Member(base, method) = &callee.peel_parens().kind else { return vec![] };
    let Some(cid) = contract_id_of(hir, base) else { return vec![] };
    hir.contract_item_ids(cid)
        .filter_map(|item| {
            let fid = item.as_function()?;
            let f = hir.function(fid);
            (f.name.is_some_and(|n| n.name == method.name) && f.parameters.len() == arg_count)
                .then_some(fid)
        })
        .collect()
}

/// Extracts the contract ID from an expression that is a contract variable or interface cast.
fn contract_id_of<'hir>(hir: &Hir<'hir>, expr: &'hir Expr<'hir>) -> Option<ContractId> {
    match &expr.peel_parens().kind {
        // `token.foo()` where `token` is a state/local variable of contract type
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            if let TypeKind::Custom(ItemId::Contract(cid)) = hir.variable(*id).ty.kind {
                Some(cid)
            } else {
                None
            }
        }
        // `IToken(addr).foo()` — explicit interface cast
        ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
            ..,
        ) => Some(*cid),
        _ => None,
    }
}

/// Returns `true` if the lvalue expression ultimately targets a storage variable.
/// Peels through index, slice, member, and payable wrappers to find the root identifier.
fn lvalue_is_state_var(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            hir.variable(*id).is_state_variable()
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base) => lvalue_is_state_var(hir, base),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().any(|e| lvalue_is_state_var(hir, e)),
        _ => false,
    }
}
