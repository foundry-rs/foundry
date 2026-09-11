use super::WeakPrng;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::{U256, uint};
use solar::{
    ast::{BinOp, BinOpKind},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{Expr, ExprKind, Hir, SourceId, Visit},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    WEAK_PRNG,
    Severity::Med,
    "weak-prng",
    "weak randomness derived from a predictable on-chain value"
);

impl<'gcx> LateLintPass<'gcx> for WeakPrng {
    fn check_nested_source(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, id: SourceId) {
        if ctx.is_lint_enabled(WEAK_PRNG.id) {
            let _ = WeakPrngChecker { ctx, gcx }.visit_nested_source(id);
        }
    }
}

struct WeakPrngChecker<'a, 's, 'gcx> {
    ctx: &'a LintContext<'s, 'a>,
    gcx: Gcx<'gcx>,
}

impl<'gcx> Visit<'gcx> for WeakPrngChecker<'_, '_, 'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    /// Emits once per outermost `<..> % <..>` or `keccak256(..)` fed by a predictable source.
    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        let is_randomness = match &expr.peel_parens().kind {
            ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs) => {
                !is_timestamp_time_bucket(self.gcx, lhs, rhs)
                    && (contains_predictable_source(self.gcx, lhs)
                        || contains_predictable_source(self.gcx, rhs))
            }
            ExprKind::Call(callee, args, _) => {
                self.gcx.resolved_builtin(callee) == Some(Builtin::Keccak256)
                    && args.exprs().any(|arg| contains_predictable_source(self.gcx, arg))
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

fn contains_predictable_source<'gcx>(gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) -> bool {
    PredictableSourceFinder { gcx }.visit_expr(expr).is_break()
}

struct PredictableSourceFinder<'gcx> {
    gcx: Gcx<'gcx>,
}

impl<'gcx> Visit<'gcx> for PredictableSourceFinder<'gcx> {
    type BreakValue = ();

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<()> {
        match &expr.peel_parens().kind {
            // `block.timestamp % 1 days` is a time bucket, not a random draw.
            ExprKind::Binary(lhs, BinOp { kind: BinOpKind::Rem, .. }, rhs)
                if is_timestamp_time_bucket(self.gcx, lhs, rhs) =>
            {
                ControlFlow::Continue(())
            }
            _ if matches!(
                self.gcx.resolved_builtin(expr),
                Some(
                    Builtin::BlockTimestamp
                        | Builtin::BlockNumber
                        | Builtin::BlockCoinbase
                        | Builtin::BlockPrevrandao
                        | Builtin::BlockDifficulty
                )
            ) =>
            {
                ControlFlow::Break(())
            }
            ExprKind::Call(callee, ..)
                if self.gcx.resolved_builtin(callee) == Some(Builtin::Blockhash) =>
            {
                ControlFlow::Break(())
            }
            _ => self.walk_expr(expr),
        }
    }
}

/// `block.timestamp % <multiple of one day>`.
fn is_timestamp_time_bucket(gcx: Gcx<'_>, lhs: &Expr<'_>, rhs: &Expr<'_>) -> bool {
    const SECONDS_PER_DAY: U256 = uint!(86400_U256);
    gcx.resolved_builtin(lhs) == Some(Builtin::BlockTimestamp)
        && gcx
            .try_eval_const(rhs)
            .ok()
            .and_then(|v| v.as_u256())
            .is_some_and(|v| v >= SECONDS_PER_DAY && v % SECONDS_PER_DAY == U256::ZERO)
}
