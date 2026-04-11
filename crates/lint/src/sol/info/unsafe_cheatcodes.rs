use super::UnsafeCheatcodes;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::hir::{self, ExprKind, ItemId, TypeKind};

declare_forge_lint!(
    UNSAFE_CHEATCODE_USAGE,
    Severity::Info,
    "unsafe-cheatcode",
    "usage of unsafe cheatcodes that can perform dangerous operations"
);

const UNSAFE_CHEATCODES: [&str; 9] = [
    "ffi",
    "readFile",
    "readLine",
    "writeFile",
    "writeLine",
    "removeFile",
    "closeFile",
    "setEnv",
    "deriveKey",
];

impl<'hir> LateLintPass<'hir> for UnsafeCheatcodes {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        if let ExprKind::Call(callee, _, _) = &expr.kind
            && let ExprKind::Member(receiver, member) = &callee.kind
            && UNSAFE_CHEATCODES.iter().any(|&c| c == member.as_str())
            && is_vm_receiver(hir, receiver)
        {
            ctx.emit(&UNSAFE_CHEATCODE_USAGE, member.span);
        }
    }
}

fn is_vm_receiver(hir: &hir::Hir<'_>, receiver: &hir::Expr<'_>) -> bool {
    match &receiver.kind {
        hir::ExprKind::Ident([hir::Res::Item(ItemId::Variable(id)), ..]) => {
            matches!(
                hir.variable(*id).ty.kind,
                TypeKind::Custom(ItemId::Contract(cid)) if is_vm_contract(hir, cid)
            )
        }
        // Support direct interface wrapping calls, e.g. `Vm(addr).ffi(...)`.
        hir::ExprKind::Call(
            hir::Expr {
                kind: hir::ExprKind::Ident([hir::Res::Item(ItemId::Contract(cid))]), ..
            },
            ..,
        ) => is_vm_contract(hir, *cid),
        _ => false,
    }
}

fn is_vm_contract(hir: &hir::Hir<'_>, contract_id: hir::ContractId) -> bool {
    hir.contract(contract_id).name.as_str() == "Vm"
}
