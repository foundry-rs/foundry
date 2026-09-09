use super::IncorrectModifier;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::block_outcome},
};
use solar::{
    ast::FunctionKind,
    sema::{Gcx, hir::Function},
};

declare_forge_lint!(
    INCORRECT_MODIFIER,
    Severity::Low,
    "incorrect-modifier",
    "modifier can finish without executing the modified function"
);

impl<'gcx> LateLintPass<'gcx> for IncorrectModifier {
    fn check_function(&mut self, ctx: &LintContext, _gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        if func.kind == FunctionKind::Modifier
            && func.body.is_some_and(|body| block_outcome(body).can_skip_placeholder())
        {
            ctx.emit(&INCORRECT_MODIFIER, func.span);
        }
    }
}
