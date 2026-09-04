use super::DeprecatedOzFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::resolved_function},
};
use solar::{
    interface::source_map::FileName,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, FunctionId, Hir},
    },
};
use std::path::Component;

declare_forge_lint!(
    DEPRECATED_OZ_FUNCTION,
    Severity::Low,
    "deprecated-oz-function",
    "OpenZeppelin deprecated this function: `_grantRole` replaces `_setupRole`, `safeIncreaseAllowance` / `safeDecreaseAllowance` replace `safeApprove`"
);

impl<'hir> LateLintPass<'hir> for DeprecatedOzFunction {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        expr: &'hir Expr<'hir>,
    ) {
        // A name or member expression typed as a function is a resolved reference, called or
        // used as a value: judge the single declaration the type checker selected (overloads,
        // overrides, `super.`, `using for` and import aliases already accounted for).
        if matches!(expr.kind, ExprKind::Ident(..) | ExprKind::Member(..))
            && let Some(function_id) = resolved_function(gcx, expr)
            && is_deprecated_oz(hir, function_id)
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
fn is_deprecated_oz(hir: &Hir<'_>, function_id: FunctionId) -> bool {
    let function = hir.function(function_id);
    let (Some(name), Some(contract_id)) = (function.name, function.contract) else { return false };
    if !is_openzeppelin_source(hir, function.source) {
        return false;
    }
    let contract = hir.contract(contract_id);
    matches!(
        (name.as_str(), contract.kind.is_library(), contract.name.as_str()),
        ("safeApprove", true, "SafeERC20" | "SafeERC20Upgradeable")
            | ("_setupRole", false, "AccessControl" | "AccessControlUpgradeable")
    )
}

/// Whether a source file belongs to an OpenZeppelin package, judged by a full path component
/// against the package roots (the npm scope and the git-submodule directories). Matching a
/// whole component rather than a substring keeps a same-name local declaration under a
/// misleading path such as `src/not-openzeppelin/` from being recognized.
fn is_openzeppelin_source(hir: &Hir<'_>, source_id: hir::SourceId) -> bool {
    const PACKAGE_ROOTS: [&str; 3] =
        ["@openzeppelin", "openzeppelin-contracts", "openzeppelin-contracts-upgradeable"];
    let FileName::Real(path) = &hir.source(source_id).file.name else { return false };
    path.components().any(|component| {
        matches!(component, Component::Normal(name)
            if PACKAGE_ROOTS.iter().any(|root| name.eq_ignore_ascii_case(root)))
    })
}
