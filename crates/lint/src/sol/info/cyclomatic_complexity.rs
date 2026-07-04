use super::CyclomaticComplexity;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{self, Expr, ExprKind, Hir, Stmt, StmtKind, Visit},
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    CYCLOMATIC_COMPLEXITY,
    Severity::Info,
    "cyclomatic-complexity",
    "this function has a cyclomatic complexity above 11; consider splitting it into smaller functions"
);

/// The threshold Slither's detector of the same name uses: a function reports when its
/// complexity is strictly above this value.
const MAX_COMPLEXITY: usize = 11;

impl<'hir> LateLintPass<'hir> for CyclomaticComplexity {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        let _ = gcx;
        if let Some(body) = &func.body {
            let mut counter = DecisionCounter { hir, decisions: 0 };
            for stmt in body.stmts {
                let _ = counter.visit_stmt(stmt);
            }
            // For a structured program the complexity is one plus the decision points.
            if counter.decisions + 1 > MAX_COMPLEXITY {
                ctx.emit(&CYCLOMATIC_COMPLEXITY, func.keyword_span());
            }
        }
    }
}

/// Counts the decision points of a function body. For a structured program the cyclomatic
/// complexity `E - N + 2P` of the control-flow graph equals one plus the number of decision
/// points, so no graph needs building.
///
/// Loops count through their condition: solar desugars every `for`, `while` and `do while`
/// into `Loop { ... if (cond) ... }`, so the synthetic `if` carries the loop's decision and a
/// condition-less `for (;;)` correctly adds nothing. Boolean `&&` / `||` operators are not
/// counted, matching the control-flow graph Slither computes on.
struct DecisionCounter<'hir> {
    hir: &'hir Hir<'hir>,
    decisions: usize,
}

impl<'hir> Visit<'hir> for DecisionCounter<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            // One decision per `if`, the loop conditions included (see above).
            StmtKind::If(..) => self.decisions += 1,
            // The first clause is the `returns` one; each `catch` clause is a branch.
            StmtKind::Try(stmt_try) => {
                self.decisions += stmt_try.clauses.len().saturating_sub(1);
            }
            // Each case of a Yul switch beyond the first is a branch.
            StmtKind::Switch(switch) => {
                self.decisions += switch.cases.len().saturating_sub(1);
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // A ternary is an `if` in expression position.
        if matches!(expr.kind, ExprKind::Ternary(..)) {
            self.decisions += 1;
        }
        self.walk_expr(expr)
    }
}
