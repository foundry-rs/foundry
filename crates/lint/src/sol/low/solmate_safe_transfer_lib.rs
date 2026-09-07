use super::SolmateSafeTransferLib;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{resolved_function, source_in_package},
    },
};
use solar::sema::{
    Gcx,
    hir::{Expr, ExprKind, FunctionId},
};

declare_forge_lint!(
    SOLMATE_SAFE_TRANSFER_LIB,
    Severity::Low,
    "solmate-safe-transfer-lib",
    "Solmate's `SafeTransferLib` does not check that the token has code, so a transfer to a token-less address succeeds silently"
);

impl<'gcx> LateLintPass<'gcx> for SolmateSafeTransferLib {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        // A name or member expression typed as a function is a resolved reference, called or
        // used as a value: judge the single declaration the type checker selected (overloads,
        // overrides, `using for` and import aliases already accounted for).
        if matches!(expr.kind, ExprKind::Ident(..) | ExprKind::Member(..))
            && let Some(function_id) = resolved_function(gcx, expr)
            && is_unchecked_token_op(gcx, function_id)
        {
            ctx.emit(&SOLMATE_SAFE_TRANSFER_LIB, expr.span);
        }
    }
}

/// Whether `function_id` is one of the token operations of solmate's `SafeTransferLib`.
/// `safeTransferETH` stays out: sending ETH involves no token code. A same-name function of
/// another library (Uniswap's `TransferHelper` style) stays out through the resolution, and so
/// does a same-name library from another package (Solady's `SafeTransferLib` checks token code
/// on the empty-return path), which fails the provenance check: the declaring source must come
/// from a solmate package path (`lib/solmate`, `solmate/...`). Matching a whole path component
/// rather than a substring keeps a vendored or patched copy under a misleading path such as
/// `vendor/solmate-fixed/` from being recognized.
fn is_unchecked_token_op(gcx: Gcx<'_>, function_id: FunctionId) -> bool {
    let function = gcx.hir.function(function_id);
    let (Some(name), Some(contract_id)) = (function.name, function.contract) else { return false };
    let contract = gcx.hir.contract(contract_id);
    matches!(name.as_str(), "safeTransfer" | "safeTransferFrom" | "safeApprove")
        && contract.kind.is_library()
        && contract.name.as_str() == "SafeTransferLib"
        && source_in_package(&gcx.hir, function.source, &["solmate"])
}
