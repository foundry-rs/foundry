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

impl<'gcx> LateLintPass<'gcx> for CyclomaticComplexity {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        func: &'gcx hir::Function<'gcx>,
    ) {
        // Modifier definitions are never reported, matching Slither which iterates only
        // declared and top-level functions. Yul helpers declared inside `assembly {}` DO
        // report: Slither scores them as functions of their own.
        if func.kind == hir::FunctionKind::Modifier || func.body.is_none() {
            return;
        }
        // Visiting the whole function rather than only the body statements also counts
        // decision points in modifier-invocation and base-constructor call arguments. For a
        // structured program the complexity is one plus the decision points.
        let mut counter = DecisionCounter { hir: &gcx.hir, decisions: 0 };
        let _ = counter.visit_function(func);
        if counter.decisions + 1 > MAX_COMPLEXITY {
            // A Yul helper's span starts at its name rather than a `function` keyword.
            let span = match func.name {
                Some(name) if func.is_yul => name.span,
                _ => func.keyword_span(),
            };
            ctx.emit(&CYCLOMATIC_COMPLEXITY, span);
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
struct DecisionCounter<'gcx> {
    hir: &'gcx Hir<'gcx>,
    decisions: usize,
}

impl<'gcx> Visit<'gcx> for DecisionCounter<'gcx> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        self.decisions += match &stmt.kind {
            StmtKind::If(..) => 1,
            // The first clause is the `returns` one; each `catch` clause is a branch.
            StmtKind::Try(stmt_try) => stmt_try.clauses.len().saturating_sub(1),
            // Each non-default case of a Yul switch is a branch; the `default` clause
            // (`constant == None`) opens no decision of its own.
            StmtKind::Switch(switch) => {
                switch.cases.iter().filter(|c| c.constant.is_some()).count()
            }
            _ => 0,
        };
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        // A ternary is an `if` in expression position.
        self.decisions += usize::from(matches!(expr.kind, ExprKind::Ternary(..)));
        self.walk_expr(expr)
    }
}
