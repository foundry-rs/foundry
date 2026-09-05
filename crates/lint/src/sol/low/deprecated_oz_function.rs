use super::DeprecatedOzFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{OPENZEPPELIN_ROOTS, resolved_function, source_in_package},
    },
};
use solar::sema::{
    Gcx,
    hir::{Expr, ExprKind, FunctionId},
};

declare_forge_lint!(
    DEPRECATED_OZ_FUNCTION,
    Severity::Low,
    "deprecated-oz-function",
    "OpenZeppelin deprecated this function: `_grantRole` replaces `_setupRole`, `safeIncreaseAllowance` / `safeDecreaseAllowance` replace `safeApprove`"
);

impl<'gcx> LateLintPass<'gcx> for DeprecatedOzFunction {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        // A name or member expression typed as a function is a resolved reference, called or
        // used as a value: judge the single declaration the type checker selected (overloads,
        // overrides, `super.`, `using for` and import aliases already accounted for).
        if matches!(expr.kind, ExprKind::Ident(..) | ExprKind::Member(..))
            && let Some(function_id) = resolved_function(gcx, expr)
            && is_deprecated_oz(gcx, function_id)
        {
            ctx.emit(&DEPRECATED_OZ_FUNCTION, expr.span);
        }
    }
}

/// Whether `function_id` is one of the functions OpenZeppelin deprecated: `SafeERC20.safeApprove`
/// and `AccessControl._setupRole` (plus their upgradeable variants). Extensions inherit these
/// functions rather than redeclare them, so resolution still lands on the canonical declaration;
/// a same-name function of an unrelated contract or library stays out, and so does a same-name
/// local declaration, which fails the provenance check.
fn is_deprecated_oz(gcx: Gcx<'_>, function_id: FunctionId) -> bool {
    let function = gcx.hir.function(function_id);
    let (Some(name), Some(contract_id)) = (function.name, function.contract) else { return false };
    if !source_in_package(&gcx.hir, function.source, OPENZEPPELIN_ROOTS) {
        return false;
    }
    let contract = gcx.hir.contract(contract_id);
    matches!(
        (name.as_str(), contract.kind.is_library(), contract.name.as_str()),
        ("safeApprove", true, "SafeERC20" | "SafeERC20Upgradeable")
            | ("_setupRole", false, "AccessControl" | "AccessControlUpgradeable")
    )
}
