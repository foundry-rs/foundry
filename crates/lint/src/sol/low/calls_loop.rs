use super::{
    CallsLoop,
    payable_loop::{LoopItem, for_each_loop_item},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_address_like, is_builtin},
    },
};
use solar::{
    ast::{StateMutability, Visibility},
    interface::{kw, sym},
    sema::{
        Gcx, Ty,
        hir::{ContractId, Expr, ExprKind, Function, FunctionId},
        ty::TyKind,
    },
};

declare_forge_lint!(CALLS_LOOP, Severity::Low, "calls-loop", "external call inside a loop");

impl<'gcx> LateLintPass<'gcx> for CallsLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_loop_item(gcx, func, false, |item| {
            if let LoopItem::Expr(expr) = item
                && let ExprKind::Call(callee, ..) = &expr.kind
                && is_external_call(gcx, callee)
            {
                ctx.emit(&CALLS_LOOP, expr.span);
            }
        });
    }
}

/// An interaction with another contract.
enum ExternalCall {
    /// `new C(..)`, or `.call`/`.delegatecall`/`.send`/`.transfer` on an address.
    Opaque,
    /// `.staticcall` on an address.
    Static,
    /// High-level call on a contract-typed receiver (`this` included), with the callee's state
    /// mutability when the type checker knows it.
    Member(Option<StateMutability>),
}

/// Classifies `callee` when calling it leaves the contract. `using for` bindings and `super`
/// dispatch run in this contract and are not external.
fn classify<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> Option<ExternalCall> {
    let callee = callee.peel_parens();
    if let ExprKind::New(ty) = &callee.kind {
        return matches!(gcx.type_of_hir_ty(ty).kind, TyKind::Contract(_))
            .then_some(ExternalCall::Opaque);
    }
    let ExprKind::Member(base, member) = &callee.kind else { return None };
    if matches!(
        member.name,
        kw::Call | kw::Delegatecall | kw::Staticcall | sym::send | sym::transfer
    ) && is_address_like(gcx, base)
    {
        let external =
            if member.name == kw::Staticcall { ExternalCall::Static } else { ExternalCall::Opaque };
        return Some(external);
    }
    let contract_receiver = is_builtin(base, sym::this)
        || gcx
            .type_of_expr(base.peel_parens().id)
            .is_some_and(|ty| matches!(ty.kind, TyKind::Contract(_)));
    let attached = gcx.resolved_callee(callee.id).is_some_and(|c| c.attached);
    (contract_receiver && !attached)
        .then(|| ExternalCall::Member(gcx.type_of_expr(callee.id).and_then(Ty::state_mutability)))
}

/// True if calling `callee` interacts with another contract (or deploys one).
pub(super) fn is_external_call<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    classify(gcx, callee).is_some()
}

/// Like [`is_external_call`], but excludes calls that cannot affect log ordering or observable
/// state: `staticcall` and high-level `view`/`pure` callees (including `this.*`). Unknown
/// callees are conservatively treated as state-mutating.
pub(super) fn is_state_mutating_external_call<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    match classify(gcx, callee) {
        Some(ExternalCall::Opaque) => true,
        Some(ExternalCall::Member(mutability)) => {
            !matches!(mutability, Some(StateMutability::View | StateMutability::Pure))
        }
        Some(ExternalCall::Static) | None => false,
    }
}

/// The base-chain function `super.<member>(..)` dispatches to from `enclosing_contract`: the first
/// arity-matching `internal`/`public` function of that name in its linearization.
pub(super) fn resolved_super_function_ids<'gcx>(
    gcx: Gcx<'gcx>,
    enclosing_contract: Option<ContractId>,
    callee: &'gcx Expr<'gcx>,
    explicit_arg_count: usize,
) -> impl Iterator<Item = FunctionId> + 'gcx {
    let target = || {
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return None };
        if !is_builtin(base, sym::super_) {
            return None;
        }
        let bases = gcx.hir.contract(enclosing_contract?).linearized_bases;
        bases.iter().skip(1).flat_map(|&id| gcx.hir.contract(id).functions()).find(|&id| {
            let func = gcx.hir.function(id);
            func.name.is_some_and(|name| name.name == member.name)
                && func.parameters.len() == explicit_arg_count
                && matches!(func.visibility, Visibility::Internal | Visibility::Public)
        })
    };
    target().into_iter()
}
