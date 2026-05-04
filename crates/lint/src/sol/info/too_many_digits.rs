use super::TooManyDigits;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{Expr, ExprKind, LitKind};

declare_forge_lint!(
    TOO_MANY_DIGITS,
    Severity::Info,
    "too-many-digits",
    "numeric literal with many digits is error-prone; \
     use scientific notation, sub-denominations, or underscore separators"
);

impl<'ast> EarlyLintPass<'ast> for TooManyDigits {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        let ExprKind::Lit(lit, sub_denom) = &expr.kind else { return };

        // Only plain integer literals. `LitKind::Address` (40-hex-digit address) is a
        // distinct variant and is therefore skipped automatically.
        if !matches!(lit.kind, LitKind::Number(_)) {
            return;
        }

        // Skip literals with a sub-denomination, e.g. `1000000 gwei`, `5 minutes`.
        if sub_denom.is_some() {
            return;
        }

        let s = lit.symbol.as_str();

        // Skip hex literals — long zero runs in hex are usually intentional (masks,
        // selectors, bit patterns) and there is no scientific-notation alternative.
        if s.starts_with("0x") || s.starts_with("0X") {
            return;
        }

        // Skip if the user already used scientific notation (`1e18`).
        if s.contains('e') || s.contains('E') {
            return;
        }

        // 5+ consecutive zeros in the literal as written. Underscores are
        // preserved, so `1_000_000` passes while `1_000000` is flagged.
        if s.contains("00000") {
            ctx.emit(&TOO_MANY_DIGITS, lit.span);
        }
    }
}
