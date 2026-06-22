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

impl<'hir> LateLintPass<'hir> for NonReentrantNotFirst {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if !matches!(
            func.kind,
            FunctionKind::Function | FunctionKind::Fallback | FunctionKind::Receive
        ) {
            return;
        }

        func.modifiers
            .iter()
            .enumerate()
            .filter(|(index, modifier)| {
                *index != 0 && modifier_is_named(hir, modifier, "nonReentrant")
            })
            .for_each(|(_, modifier)| ctx.emit(&NON_REENTRANT_NOT_FIRST, modifier.span));
    }
}

fn modifier_is_named(hir: &hir::Hir<'_>, modifier: &hir::Modifier<'_>, name: &str) -> bool {
    modifier.id.as_function().is_some_and(|function_id| {
        let modifier_fn = hir.function(function_id);
        matches!(modifier_fn.kind, FunctionKind::Modifier)
            && modifier_fn.name.is_some_and(|ident| ident.as_str() == name)
    })
}
