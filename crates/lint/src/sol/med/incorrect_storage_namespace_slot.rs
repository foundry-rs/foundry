use super::IncorrectStorageNamespaceSlot;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use foundry_common::{erc7201, namespaced_storage::storage_namespace};
use solar::{
    interface::diagnostics::Applicability,
    sema::{Gcx, hir},
};

declare_forge_lint!(
    INCORRECT_STORAGE_NAMESPACE_SLOT,
    Severity::Med,
    "incorrect-storage-namespace-slot",
    "storage slot does not match the struct's ERC-7201 namespace"
);

impl<'gcx> LateLintPass<'gcx> for IncorrectStorageNamespaceSlot {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) {
        // Matches `$.slot := value` in inline assembly, where `$` points to a namespaced struct.
        if let hir::ExprKind::Assign(lhs, None, value) = expr.kind
            && let hir::ExprKind::YulMember(pointer, member) = lhs.kind
            && member.as_str() == "slot"
            && let hir::ExprKind::Ident([hir::Res::Item(hir::ItemId::Variable(pointer))]) =
                pointer.kind
            && let hir::TypeKind::Custom(hir::ItemId::Struct(strukt)) =
                gcx.hir.variable(*pointer).ty.kind
            && let Ok(Some(namespace)) = storage_namespace(gcx, gcx.hir.strukt(strukt))
            && let Some(slot) = gcx.const_eval().eval_expr(value).ok().and_then(|v| v.as_word())
        {
            let expected = erc7201(namespace);
            if slot == Into::<U256>::into(expected) {
                return;
            }
            ctx.emit_with_suggestion(
                &INCORRECT_STORAGE_NAMESPACE_SLOT,
                value.span,
                Suggestion::fix(expected.to_string(), Applicability::MaybeIncorrect)
                    .with_desc("consider using the annotated namespace's ERC-7201 root"),
            );
        }
    }
}
