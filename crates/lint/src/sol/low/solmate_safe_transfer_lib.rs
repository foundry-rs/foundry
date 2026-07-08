use super::SolmateSafeTransferLib;
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
    SOLMATE_SAFE_TRANSFER_LIB,
    Severity::Low,
    "solmate-safe-transfer-lib",
    "Solmate's `SafeTransferLib` does not check that the token has code, so a transfer to a token-less address succeeds silently"
);

impl<'hir> LateLintPass<'hir> for SolmateSafeTransferLib {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        if let Some(body) = &func.body {
            let mut finder = TokenOpFinder { gcx, hir, ctx };
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

/// Looks for references to `SafeTransferLib`'s token operations, called or used as values.
struct TokenOpFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
}

impl<'hir> Visit<'hir> for TokenOpFinder<'_, '_, '_, 'hir> {
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
            && self.is_unchecked_token_op(function_id)
        {
            self.ctx.emit(&SOLMATE_SAFE_TRANSFER_LIB, expr.span);
        }
        self.walk_expr(expr)
    }
}

impl TokenOpFinder<'_, '_, '_, '_> {
    /// The single function an expression resolves to, for a callee or a reference used as a
    /// value. `type_of_expr` is the function the type checker resolved, so overload selection,
    /// override shadowing, the qualified and `using for` forms and import aliases are already
    /// accounted for.
    fn resolved_function(&self, expr: &Expr<'_>) -> Option<FunctionId> {
        let ty = self.gcx.type_of_expr(expr.peel_parens().id)?;
        match ty.kind {
            TyKind::Fn(function_ty) => function_ty.function_id,
            _ => None,
        }
    }

    /// Whether `function_id` is one of the token operations of solmate's `SafeTransferLib`.
    /// `safeTransferETH` stays out: sending ETH involves no token code, so the missing-code
    /// concern does not apply to it. A same-name function of another library (Uniswap's
    /// `TransferHelper` style) stays out through the resolution, and so does a same-name
    /// library from another package (Solady's `SafeTransferLib` checks token code on the
    /// empty-return path), which fails the provenance check.
    fn is_unchecked_token_op(&self, function_id: FunctionId) -> bool {
        let function = self.hir.function(function_id);
        let Some(name) = function.name else { return false };
        let Some(contract_id) = function.contract else { return false };
        // The name alone does not prove the declaration is solmate's: the declaring source
        // must come from a solmate package path (`lib/solmate`, `solmate/...`).
        if !self.is_solmate_source(function.source) {
            return false;
        }
        let contract = self.hir.contract(contract_id);
        matches!(name.as_str(), "safeTransfer" | "safeTransferFrom" | "safeApprove")
            && contract.kind.is_library()
            && contract.name.as_str() == "SafeTransferLib"
    }

    /// Whether a source file belongs to the solmate package, judged by a full path component.
    /// Matching a whole component rather than a substring keeps a vendored or patched copy under
    /// a misleading path such as `vendor/solmate-fixed/` from being recognized.
    fn is_solmate_source(&self, source_id: hir::SourceId) -> bool {
        match &self.hir.source(source_id).file.name {
            FileName::Real(path) => path.components().any(|component| {
                matches!(component, std::path::Component::Normal(name)
                    if name.eq_ignore_ascii_case("solmate"))
            }),
            _ => false,
        }
    }
}
