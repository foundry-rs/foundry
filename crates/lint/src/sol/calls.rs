use solar::{
    ast::{Expr, ExprKind},
    interface::kw,
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
