use solar::{
    ast::{Expr, ExprKind},
    interface::kw,
    sema::hir,
};

/// Checks if an expression is a low-level call.
///
/// Detects patterns like:
/// - `target.call(...)`
/// - `target.delegatecall(...)`
/// - `target.staticcall(...)`
/// - `target.call{value: x}(...)`
pub(crate) const fn is_low_level_call(expr: &Expr<'_>) -> bool {
    if let ExprKind::Call(call_expr, _args) = &expr.kind {
        let callee = match &call_expr.kind {
            ExprKind::CallOptions(inner_expr, _) => inner_expr,
            _ => call_expr,
        };

        if let ExprKind::Member(_, member) = &callee.kind {
            return matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall);
        }
    }
    false
}

/// Checks if a HIR expression is a low-level call with an explicit gas option.
///
/// Detects patterns like:
/// - `target.call{gas: gasLimit}(...)`
/// - `target.delegatecall{gas: gasLimit}(...)`
/// - `target.staticcall{gas: gasLimit}(...)`
pub(crate) fn is_low_level_call_with_gas_limit(expr: &hir::Expr<'_>) -> bool {
    let hir::ExprKind::Call(callee, _, Some(opts)) = &expr.kind else {
        return false;
    };

    if !matches!(
        &callee.peel_parens().kind,
        hir::ExprKind::Member(_, member)
            if matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
    ) {
        return false;
    }

    opts.iter().any(|opt| opt.name.name == kw::Gas)
}
