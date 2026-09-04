use super::NonReentrantNotFirst;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{self, FunctionKind},
};

declare_forge_lint!(
    NON_REENTRANT_NOT_FIRST,
    Severity::Med,
    "non-reentrant-not-first",
    "`nonReentrant` should be the first modifier"
);

impl<'gcx> LateLintPass<'gcx> for NonReentrantNotFirst {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        if !matches!(
            func.kind,
            FunctionKind::Function | FunctionKind::Fallback | FunctionKind::Receive
        ) {
            return;
        }
        for modifier in func.modifiers.iter().skip(1) {
            let is_non_reentrant = modifier.id.as_function().is_some_and(|id| {
                gcx.hir.function(id).name.is_some_and(|name| name.as_str() == "nonReentrant")
            });
            if is_non_reentrant {
                ctx.emit(&NON_REENTRANT_NOT_FIRST, modifier.span);
            }
        }
    }
}
