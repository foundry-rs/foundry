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

/// Checks if a HIR expression is any call with an explicit gas option.
pub(crate) fn is_call_with_gas_limit(expr: &hir::Expr<'_>) -> bool {
    let Some((_, opts)) = call_with_options(expr) else {
        return false;
    };

    opts.args.iter().any(|opt| opt.name.name == kw::Gas)
}

fn call_with_options<'hir>(
    expr: &'hir hir::Expr<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, &'hir hir::CallOptions<'hir>)> {
    let hir::ExprKind::Call(callee, _, Some(opts)) = &expr.peel_parens().kind else {
        return None;
    };
    Some((callee, opts))
}
