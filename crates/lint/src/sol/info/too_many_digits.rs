use super::TooManyDigits;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Expr, ExprKind, Lit, LitKind, Stmt, StmtKind, visit::Visit},
    data_structures::Never,
};
use std::ops::ControlFlow;

declare_forge_lint!(
    TOO_MANY_DIGITS,
    Severity::Info,
    "too-many-digits",
    "numeric literal with many digits is error-prone; \
     use scientific notation, sub-denominations, or underscore separators"
);

impl<'ast> EarlyLintPass<'ast> for TooManyDigits {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        // Yul literals are not `Expr`s, so `check_expr` never sees them.
        if let StmtKind::Assembly(assembly) = &stmt.kind {
            let _ = YulLits { ctx }.visit_yul_block(&assembly.block);
        }
    }

    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        // Skip literals with a sub-denomination, e.g. `1000000 gwei`, `5 minutes`.
        if let ExprKind::Lit(lit, None) = &expr.kind {
            check_lit(ctx, lit);
        }
    }
}

fn check_lit(ctx: &LintContext, lit: &Lit<'_>) {
    // Only plain integer literals; `LitKind::Address` is a distinct variant.
    if !matches!(lit.kind, LitKind::Number(_)) {
        return;
    }
    let s = lit.symbol.as_str();
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"));
    // Match Slither's detector: skip only address-shaped hex constants, not all hex constants
    // (long padded masks/selectors are still hard to review), and scientific notation (`1e18`).
    let is_hex_address = hex.is_some_and(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let is_scientific = hex.is_none() && s.contains(['e', 'E']);
    // 5+ consecutive zeros in the literal as written. Underscores are preserved, so
    // `1_000_000` passes while `1_000000` is flagged.
    if !is_hex_address && !is_scientific && s.contains("00000") {
        ctx.emit(&TOO_MANY_DIGITS, lit.span);
    }
}

/// Checks every literal of an assembly block, `case` labels included.
struct YulLits<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for YulLits<'_, '_> {
    type BreakValue = Never;

    fn visit_lit(&mut self, lit: &'ast Lit<'_>) -> ControlFlow<Self::BreakValue> {
        check_lit(self.ctx, lit);
        ControlFlow::Continue(())
    }
}
