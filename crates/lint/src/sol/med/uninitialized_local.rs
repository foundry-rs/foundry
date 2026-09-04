use super::UninitializedLocal;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{branch_always_exits, for_each_lhs_var},
    },
};
use solar::{
    interface::{Span, data_structures::Never},
    sema::{
        Gcx, Hir,
        hir::{
            Expr, ExprKind, Function, LoopSource, Res, Stmt, StmtKind, TypeKind, VarKind,
            VariableId, Visit,
        },
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};

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
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };
        let mut checker = Checker { hir, uninitialized: HashSet::new(), findings: HashMap::new() };
        for stmt in body.stmts {
            let _ = checker.visit_stmt(stmt);
        }
        for span in checker.findings.into_values() {
            ctx.emit(&UNINITIALIZED_LOCAL, span);
        }
    }
}

struct Checker<'hir> {
    hir: &'hir Hir<'hir>,
    /// Value-type locals declared without an initializer that have not yet been written on
    /// every path.
    uninitialized: HashSet<VariableId>,
    /// First read span per variable that was read while uninitialized.
    findings: HashMap<VariableId, Span>,
}

impl Checker<'_> {
    fn mark_written(&mut self, lhs: &Expr<'_>) {
        for_each_lhs_var(lhs, &mut |v| {
            self.uninitialized.remove(&v);
        });
    }
}

impl<'hir> Visit<'hir> for Checker<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Never> {
        match &stmt.kind {
            StmtKind::DeclSingle(vid) => {
                let v = self.hir.variable(*vid);
                if v.kind == VarKind::Statement
                    && v.initializer.is_none()
                    && matches!(v.ty.kind, TypeKind::Elementary(ty) if ty.is_value_type())
                {
                    self.uninitialized.insert(*vid);
                }
            }
            // A variable stays uninitialized if any branch that falls through fails to write it.
            StmtKind::If(cond, then, else_) => {
                self.visit_expr(cond)?;
                let before = self.uninitialized.clone();
                self.visit_stmt(then)?;
                let after_then = std::mem::replace(&mut self.uninitialized, before);
                if let Some(else_) = else_ {
                    self.visit_stmt(else_)?;
                }
                if branch_always_exits(then) {
                    // Only the else path continues; keep its state.
                } else if else_.is_some_and(branch_always_exits) {
                    self.uninitialized = after_then;
                } else {
                    self.uninitialized.extend(after_then);
                }
                return ControlFlow::Continue(());
            }
            // `do-while` runs its body once, so its writes are guaranteed; `for`/`while` may run
            // zero times, so theirs are discarded.
            StmtKind::Loop(block, source) => {
                let before = self.uninitialized.clone();
                for s in block.stmts {
                    self.visit_stmt(s)?;
                }
                if *source != LoopSource::DoWhile {
                    self.uninitialized = before;
                }
                return ControlFlow::Continue(());
            }
            // Each clause is an independent path, like `if`/`else` branches.
            StmtKind::Try(t) => {
                self.visit_expr(&t.expr)?;
                let before = self.uninitialized.clone();
                let mut merged = HashSet::new();
                for clause in t.clauses {
                    self.uninitialized = before.clone();
                    for s in clause.block.stmts {
                        self.visit_stmt(s)?;
                    }
                    merged.extend(self.uninitialized.drain());
                }
                self.uninitialized = merged;
                return ControlFlow::Continue(());
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Never> {
        match &expr.kind {
            // Compound `op=` reads the lhs first; plain `=` reads only the rhs (catches `x = x`).
            // The lhs is still walked afterwards for reads inside e.g. an index expression.
            ExprKind::Assign(lhs, op, rhs) => {
                if op.is_some() {
                    self.visit_expr(lhs)?;
                }
                self.visit_expr(rhs)?;
                self.mark_written(lhs);
                if op.is_none() {
                    self.visit_expr(lhs)?;
                }
                ControlFlow::Continue(())
            }
            // `delete x` is an explicit write to the zero value, not a read.
            ExprKind::Delete(target) => {
                self.mark_written(target);
                self.visit_expr(target)
            }
            ExprKind::Ident(reses) => {
                if let Some(vid) = reses
                    .iter()
                    .filter_map(Res::as_variable)
                    .find(|v| self.uninitialized.contains(v))
                {
                    self.findings.entry(vid).or_insert(expr.span);
                }
                ControlFlow::Continue(())
            }
            _ => self.walk_expr(expr),
        }
    }
}
