use super::WeakPrng;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOp, BinOpKind, Expr, ExprKind, LitKind, SourceUnit, visit::Visit},
    interface::{kw, sym},
};
use std::ops::ControlFlow;

declare_forge_lint!(
    WEAK_PRNG,
    Severity::Med,
    "weak-prng",
    "weak randomness derived from a predictable on-chain value"
);

impl<'ast> EarlyLintPass<'ast> for WeakPrng {
    fn check_full_source_unit(&mut self, ctx: &LintContext<'ast, '_>, ast: &'ast SourceUnit<'ast>) {
        if ctx.is_lint_enabled(WEAK_PRNG.id) {
            let _ = WeakPrngChecker { ctx }.visit_source_unit(ast);
        }
    }
}

struct WeakPrngChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for WeakPrngChecker<'_, '_> {
    type BreakValue = ();

    /// Emits once per outermost `<..> % <..>` or `keccak256(..)` fed by a predictable source.
    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<()> {
        let is_randomness = match &expr.peel_parens().kind {
            ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs) => {
                !is_timestamp_time_bucket(lhs, rhs)
                    && (contains_predictable_source(lhs) || contains_predictable_source(rhs))
            }
            ExprKind::Call(callee, args) => {
                is_ident(callee, kw::Keccak256) && args.exprs().any(contains_predictable_source)
            }
            _ => false,
        };
        if is_randomness {
            self.ctx.emit(&WEAK_PRNG, expr.span);
            return ControlFlow::Continue(());
        }
        self.walk_expr(expr)
    }
}

fn contains_predictable_source<'ast>(expr: &'ast Expr<'ast>) -> bool {
    PredictableSourceFinder.visit_expr(expr).is_break()
}

struct PredictableSourceFinder;

impl<'ast> Visit<'ast> for PredictableSourceFinder {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<()> {
        match &expr.peel_parens().kind {
            // `block.timestamp % 1 days` is a time bucket, not a random draw.
            ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs)
                if is_timestamp_time_bucket(lhs, rhs) =>
            {
                ControlFlow::Continue(())
            }
            ExprKind::Member(base, member)
                if is_ident(base, sym::block)
                    && matches!(
                        member.name,
                        kw::Timestamp
                            | kw::Number
                            | kw::Coinbase
                            | kw::Prevrandao
                            | kw::Difficulty
                    ) =>
            {
                ControlFlow::Break(())
            }
            ExprKind::Call(callee, _) if is_ident(callee, kw::Blockhash) => ControlFlow::Break(()),
            _ => self.walk_expr(expr),
        }
    }
}

/// `block.timestamp % <multiple of one day>`.
fn is_timestamp_time_bucket(lhs: &Expr<'_>, rhs: &Expr<'_>) -> bool {
    const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
    matches!(&lhs.peel_parens().kind, ExprKind::Member(base, member)
        if is_ident(base, sym::block) && member.name == kw::Timestamp)
        && const_eval_u64(rhs).is_some_and(|v| v >= SECONDS_PER_DAY && v % SECONDS_PER_DAY == 0)
}

fn const_eval_u64(expr: &Expr<'_>) -> Option<u64> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit, sub) => {
            let LitKind::Number(value) = &lit.kind else { return None };
            u64::try_from(value).ok()?.checked_mul(sub.map_or(1, |s| s.value()))
        }
        ExprKind::Binary(lhs, op, rhs) => {
            let (lhs, rhs) = (const_eval_u64(lhs)?, const_eval_u64(rhs)?);
            match op.kind {
                BinOpKind::Add => lhs.checked_add(rhs),
                BinOpKind::Sub => lhs.checked_sub(rhs),
                BinOpKind::Mul => lhs.checked_mul(rhs),
                BinOpKind::Div => lhs.checked_div(rhs),
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_ident(expr: &Expr<'_>, name: solar::interface::Symbol) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Ident(ident) if ident.name == name)
}
