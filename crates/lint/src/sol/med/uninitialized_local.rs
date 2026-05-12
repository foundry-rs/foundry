use super::UninitializedLocal;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::data_structures::Never,
    sema::{
        Hir,
        hir::{Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, VarKind, VariableId, Visit},
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    UNINITIALIZED_LOCAL,
    Severity::Med,
    "uninitialized-local",
    "local variable is read before being initialized"
);

impl<'hir> LateLintPass<'hir> for UninitializedLocal {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut checker = Checker { hir, uninitialized: HashSet::new(), findings: HashSet::new() };
        for stmt in body.stmts {
            let _ = checker.visit_stmt(stmt);
        }

        for vid in checker.findings {
            ctx.emit(&UNINITIALIZED_LOCAL, hir.variable(vid).span);
        }
    }
}

struct Checker<'hir> {
    hir: &'hir Hir<'hir>,
    /// Locals declared without an initializer that have not yet been written.
    uninitialized: HashSet<VariableId>,
    /// Variables that were read while uninitialized (deduplicated by variable).
    findings: HashSet<VariableId>,
}

impl<'hir> Visit<'hir> for Checker<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            StmtKind::DeclSingle(vid) => {
                let v = self.hir.variable(*vid);
                if matches!(v.kind, VarKind::Statement) && v.initializer.is_none() {
                    self.uninitialized.insert(*vid);
                }
                // Walk initializer (if any) to catch reads of other uninitialized vars.
                if let Some(init) = v.initializer {
                    let _ = self.visit_expr(init);
                }
                return ControlFlow::Continue(());
            }

            // For if/else: visit condition, then snapshot and walk each branch independently,
            // then union the post-branch sets (conservative: still uninitialized if any path
            // fails to write the variable).
            StmtKind::If(cond, then, else_) => {
                let _ = self.visit_expr(cond);

                let before = self.uninitialized.clone();

                let _ = self.visit_stmt(then);
                let after_then = self.uninitialized.clone();

                self.uninitialized = before;
                if let Some(else_stmt) = else_ {
                    let _ = self.visit_stmt(else_stmt);
                }
                let after_else = self.uninitialized.clone();

                self.uninitialized = after_then.union(&after_else).copied().collect();
                return ControlFlow::Continue(());
            }

            // Loops may execute zero times, so writes inside cannot be treated as guaranteed.
            // Visit the body to catch reads inside, but restore the uninitialized set afterwards.
            StmtKind::Loop(block, _) => {
                let before = self.uninitialized.clone();
                for s in block.stmts {
                    let _ = self.visit_stmt(s);
                }
                self.uninitialized = before;
                return ControlFlow::Continue(());
            }

            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // Plain `=`: visit RHS first (catches `x = x`), then mark LHS as written.
            ExprKind::Assign(lhs, None, rhs) => {
                let _ = self.visit_expr(rhs);
                mark_written(lhs, &mut self.uninitialized);
                // Still walk lhs in case it's a complex expression (e.g. array index).
                let _ = self.visit_expr(lhs);
                return ControlFlow::Continue(());
            }

            // Compound `op=`: both sides are read first, then lhs is written.
            ExprKind::Assign(lhs, Some(_), rhs) => {
                let _ = self.visit_expr(lhs);
                let _ = self.visit_expr(rhs);
                mark_written(lhs, &mut self.uninitialized);
                return ControlFlow::Continue(());
            }

            ExprKind::Ident(reses) => {
                for res in *reses {
                    if let Res::Item(ItemId::Variable(vid)) = res
                        && self.uninitialized.contains(vid)
                    {
                        self.findings.insert(*vid);
                        break;
                    }
                }
            }

            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// If `expr` is a direct identifier resolving to a local variable, remove it from `uninitialized`.
fn mark_written(expr: &Expr<'_>, uninitialized: &mut HashSet<VariableId>) {
    if let ExprKind::Ident(reses) = &expr.kind {
        for res in *reses {
            if let Res::Item(ItemId::Variable(vid)) = res {
                uninitialized.remove(vid);
            }
        }
    }
}
