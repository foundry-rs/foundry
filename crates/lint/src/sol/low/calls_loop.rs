use super::{
    CallsLoop,
    payable_loop::{LoopItem, for_each_loop_item},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::StateMutability,
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{Expr, ExprKind, Function},
        ty::{TyFnKind, TyKind},
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
    /// Contract creation or a mutating address builtin.
    Opaque,
    /// `.staticcall` on an address.
    Static,
    /// High-level external or library call, including an external function pointer.
    Member(StateMutability),
}

/// Classifies calls by their checked function kind, including function pointers and library
/// delegate calls. Internal `using for` bindings and `super` dispatch stay internal.
fn classify<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> Option<ExternalCall> {
    let callee = callee.peel_parens();
    if matches!(
        gcx.resolved_builtin(callee),
        Some(Builtin::AddressPayableSend | Builtin::AddressPayableTransfer)
    ) {
        return Some(ExternalCall::Opaque);
    }
    let TyKind::Fn(function) = gcx.type_of_expr(callee.id)?.kind else { return None };
    match function.kind() {
        TyFnKind::External | TyFnKind::DelegateCall => {
            Some(ExternalCall::Member(function.state_mutability))
        }
        TyFnKind::BareStaticCall => Some(ExternalCall::Static),
        TyFnKind::BareCall | TyFnKind::BareDelegateCall | TyFnKind::Creation => {
            Some(ExternalCall::Opaque)
        }
        _ => None,
    }
}

/// True if calling `callee` interacts with another contract (or deploys one).
pub(super) fn is_external_call<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    classify(gcx, callee).is_some()
}

/// Like [`is_external_call`], but excludes calls that cannot affect log ordering or observable
/// state: `staticcall` and high-level `view`/`pure` callees (including `this.*`).
pub(super) fn is_state_mutating_external_call<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    match classify(gcx, callee) {
        Some(ExternalCall::Opaque) => true,
        Some(ExternalCall::Member(mutability)) => {
            !matches!(mutability, StateMutability::View | StateMutability::Pure)
        }
        Some(ExternalCall::Static) | None => false,
    }
}
