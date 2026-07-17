use super::DeprecatedOzFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::source_map::FileName,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, FunctionId, Hir},
        ty::TyKind,
    },
};

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
        // used as a value: judge the single declaration the type checker selected. The linter
        // visits every expression in the unit, so a reference in a function header (a modifier
        // or base-constructor argument) is caught too, not only one inside a function body.
        if matches!(expr.kind, ExprKind::Ident(..) | ExprKind::Member(..)) {
            let resolver = DeprecatedRefResolver { gcx, hir };
            if let Some(function_id) = resolver.resolved_function(expr)
                && resolver.is_deprecated_oz(function_id)
            {
                ctx.emit(&DEPRECATED_OZ_FUNCTION, expr.span);
            }
        }
    }
}

/// Resolves an expression to the deprecated OpenZeppelin function it references, if any.
struct DeprecatedRefResolver<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
}

impl DeprecatedRefResolver<'_> {
    /// The single function an expression resolves to, for a callee or a reference used as a
    /// value. `type_of_expr` is the function the type checker resolved, so overload selection,
    /// override shadowing, `super.`, the qualified and `using for` forms and import aliases
    /// are already accounted for.
    fn resolved_function(&self, expr: &Expr<'_>) -> Option<FunctionId> {
        let ty = self.gcx.type_of_expr(expr.peel_parens().id)?;
        match ty.kind {
            TyKind::Fn(function_ty) => function_ty.function_id,
            _ => None,
        }
    }

    /// Whether `function_id` is one of the functions OpenZeppelin deprecated, identified by
    /// the exact name of the declaring contract or library: `SafeERC20.safeApprove` and
    /// `AccessControl._setupRole` (plus their upgradeable variants). Extensions inherit these
    /// functions rather than redeclare them, so resolution still lands on the canonical
    /// declaration; a same-name function of an unrelated contract or library stays out, and
    /// so does a same-name local declaration, which fails the provenance check.
    fn is_deprecated_oz(&self, function_id: FunctionId) -> bool {
        let function = self.hir.function(function_id);
        let Some(name) = function.name else { return false };
        let Some(contract_id) = function.contract else { return false };
        // The name alone does not prove the declaration is OpenZeppelin's: a local library
        // or contract may share it. The declaring source must come from an OpenZeppelin
        // package path (`lib/openzeppelin-contracts`, `@openzeppelin/...`).
        if !self.is_openzeppelin_source(function.source) {
            return false;
        }
        let contract = self.hir.contract(contract_id);
        let contract_name = contract.name.as_str();
        // `safeApprove` lives in the SafeERC20 library, `_setupRole` in the AccessControl
        // contract: requiring the matching declaration kind tightens the match.
        if name.as_str() == "safeApprove" {
            contract.kind.is_library()
                && matches!(contract_name, "SafeERC20" | "SafeERC20Upgradeable")
        } else if name.as_str() == "_setupRole" {
            !contract.kind.is_library()
                && matches!(contract_name, "AccessControl" | "AccessControlUpgradeable")
        } else {
            false
        }
    }

    /// Whether a source file belongs to an OpenZeppelin package, judged by a full path
    /// component against the package roots (the npm scope and the git-submodule directories).
    /// Matching a whole component rather than a substring keeps a same-name local declaration
    /// under a misleading path such as `src/not-openzeppelin/` from being recognized.
    fn is_openzeppelin_source(&self, source_id: hir::SourceId) -> bool {
        const OPENZEPPELIN_PACKAGE_ROOTS: [&str; 3] =
            ["@openzeppelin", "openzeppelin-contracts", "openzeppelin-contracts-upgradeable"];
        match &self.hir.source(source_id).file.name {
            FileName::Real(path) => path.components().any(|component| {
                matches!(component, std::path::Component::Normal(name)
                    if OPENZEPPELIN_PACKAGE_ROOTS.iter().any(|root| name.eq_ignore_ascii_case(root)))
            }),
            _ => false,
        }
    }
}
