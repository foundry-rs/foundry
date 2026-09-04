use super::AssertStateChange;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{
            for_each_lhs_var, function_ids, is_address_like, is_builtin, receiver_contract_id,
            resolved_function, ty_contract_id,
        },
    },
};
use solar::{
    ast::{DataLocation, ElementaryType},
    interface::{kw, sym},
    sema::{
        Gcx, Hir,
        hir::{CallArgs, ContractId, Expr, ExprKind},
        ty::TyKind,
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    ASSERT_STATE_CHANGE,
    Severity::Med,
    "assert-state-change",
    "assert() should not contain state-modifying expressions"
);

impl<'hir> LateLintPass<'hir> for AssertStateChange {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir Hir<'hir>,
        expr: &'hir Expr<'hir>,
    ) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        if !is_builtin(callee, sym::assert) {
            return;
        }
        for arg in args.exprs() {
            // Point the diagnostic at the first sub-expression that mutates state.
            if let ControlFlow::Break(span) = arg.visit(&mut |e| {
                if is_state_change(gcx, e) {
                    ControlFlow::Break(e.span)
                } else {
                    ControlFlow::Continue(())
                }
            }) {
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

fn is_state_change<'hir>(gcx: Gcx<'hir>, expr: &Expr<'hir>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => is_storage_lvalue(gcx, lhs),
        ExprKind::Unary(op, lhs) => op.kind.has_side_effects() && is_storage_lvalue(gcx, lhs),
        ExprKind::Call(callee, args, _) => is_mutating_call(gcx, callee, args),
        _ => false,
    }
}

/// True if the lvalue is rooted in contract storage: a state variable or a local declared
/// `storage`, which aliases contract storage.
fn is_storage_lvalue(gcx: Gcx<'_>, expr: &Expr<'_>) -> bool {
    let mut found = false;
    for_each_lhs_var(expr, &mut |v| {
        let v = gcx.hir.variable(v);
        found |= v.is_state_variable() || v.data_location == Some(DataLocation::Storage);
    });
    found
}

fn is_mutating_call<'hir>(gcx: Gcx<'hir>, callee: &Expr<'hir>, args: &CallArgs<'hir>) -> bool {
    let hir = &gcx.hir;
    let mutates = |fid| hir.function(fid).mutates_state();
    if let ExprKind::Member(base, method) = &callee.kind {
        // `arr.push(..)` / `arr.pop()` on a storage array or `bytes`. The type check keeps
        // contract methods that happen to be named push/pop out of this heuristic.
        if matches!(method.name, sym::push | kw::Pop)
            && gcx.type_of_expr(base.peel_parens().id).is_some_and(|ty| {
                matches!(
                    ty.peel_refs().kind,
                    TyKind::DynArray(_)
                        | TyKind::Array(..)
                        | TyKind::Elementary(ElementaryType::Bytes)
                )
            })
            && is_storage_lvalue(gcx, base)
        {
            return true;
        }
        // Low-level address calls always transfer value or execute foreign code. The receiver
        // must be address-like so contract methods named send/call/transfer are not caught.
        if matches!(method.name, kw::Call | kw::Delegatecall | sym::send | sym::transfer)
            && is_address_like(gcx, base)
        {
            return true;
        }
        // Member calls on a contract: flag when any overload with this name and arity mutates,
        // so a mutating overload is not hidden behind a view one of the same arity.
        if let Some(cid) = contract_id_of(gcx, base)
            && hir.contract_item_ids(cid).filter_map(|item| item.as_function()).any(|fid| {
                let f = hir.function(fid);
                f.name.is_some_and(|n| n.name == method.name)
                    && f.parameters.len() == args.len()
                    && mutates(fid)
            })
        {
            return true;
        }
    }
    // Whatever the type checker resolved (including `using for` extensions), then the same
    // any-overload-mutates policy for bare internal calls.
    resolved_function(gcx, callee).is_some_and(mutates)
        || function_ids(callee)
            .filter(|&fid| hir.function(fid).parameters.len() == args.len())
            .any(mutates)
}

/// The contract a method-call receiver denotes, including `this`.
fn contract_id_of<'hir>(gcx: Gcx<'hir>, recv: &Expr<'hir>) -> Option<ContractId> {
    gcx.type_of_expr(recv.peel_parens().id)
        .and_then(ty_contract_id)
        .or_else(|| receiver_contract_id(gcx, recv))
}
