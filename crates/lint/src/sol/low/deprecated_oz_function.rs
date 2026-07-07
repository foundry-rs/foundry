use super::DeprecatedOzFunction;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::source_map::FileName,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, FunctionId, Hir, Visit},
        ty::TyKind,
    },
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    DEPRECATED_OZ_FUNCTION,
    Severity::Low,
    "deprecated-oz-function",
    "OpenZeppelin deprecated this function: `_grantRole` replaces `_setupRole`, `safeIncreaseAllowance` / `safeDecreaseAllowance` replace `safeApprove`"
);

impl<'hir> LateLintPass<'hir> for DeprecatedOzFunction {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if let Some(body) = &func.body {
            let mut finder = DeprecatedRefFinder { gcx, hir, ctx };
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

/// Looks for references to the deprecated OpenZeppelin functions, called or used as values.
struct DeprecatedRefFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
}

impl<'hir> Visit<'hir> for DeprecatedRefFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // A name or member expression typed as a function is a resolved reference, called or
        // used as a value: judge the single declaration the type checker selected. The callee
        // of a call is visited by the default walk, so calls need no dedicated arm.
        if matches!(expr.kind, ExprKind::Ident(..) | ExprKind::Member(..))
            && let Some(function_id) = self.resolved_function(expr)
            && self.is_deprecated_oz(function_id)
        {
            self.ctx.emit(&DEPRECATED_OZ_FUNCTION, expr.span);
        }
        self.walk_expr(expr)
    }
}

impl DeprecatedRefFinder<'_, '_, '_, '_> {
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

    /// Whether a source file belongs to an OpenZeppelin package, judged by its path. A
    /// vendored copy under a path that does not name OpenZeppelin is not recognized.
    fn is_openzeppelin_source(&self, source_id: hir::SourceId) -> bool {
        match &self.hir.source(source_id).file.name {
            FileName::Real(path) => path.to_string_lossy().to_lowercase().contains("openzeppelin"),
            _ => false,
        }
    }
}
