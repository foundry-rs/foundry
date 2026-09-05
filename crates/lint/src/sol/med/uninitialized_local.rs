use super::UninitializedLocal;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{branch_always_exits, for_each_lhs_var, loop_stmts},
    },
};
use solar::{
    ast::ElementaryType,
    interface::{Span, data_structures::Never},
    sema::{
        Gcx, Hir,
        hir::{
            BinOpKind, Block, Expr, ExprKind, Function, LoopSource, Res, Stmt, StmtKind, TypeKind,
            UnOpKind, VarKind, VariableId, Visit,
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

impl<'gcx> LateLintPass<'gcx> for UninitializedLocal {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        let Some(body) = func.body else { return };
        let mut checker =
            Checker { hir: &gcx.hir, uninitialized: HashSet::new(), findings: HashMap::new() };
        for stmt in body.stmts {
            let _ = checker.visit_stmt(stmt);
        }
        for span in checker.findings.into_values() {
            ctx.emit(&UNINITIALIZED_LOCAL, span);
        }
    }
}

struct Checker<'gcx> {
    hir: &'gcx Hir<'gcx>,
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

/// The loop statement of a conventional `for (uint i; i < n; i++)` header whose counter relies on
/// its implicit zero. The header lowers to `{ decl; loop { if cond { body } else break } }` with
/// the update on the loop source; matching the wrapper's span keeps declarations outside the
/// header distinct.
fn defaulted_counter_loop<'gcx>(
    hir: &Hir<'gcx>,
    block: &'gcx Block<'gcx>,
) -> Option<&'gcx Stmt<'gcx>> {
    if let [Stmt { kind: StmtKind::DeclSingle(vid), .. }, loop_stmt] = block.stmts
        && block.span == loop_stmt.span
        && let StmtKind::Loop(body, LoopSource::For { update: Some(update) }) = &loop_stmt.kind
        && let var = hir.variable(*vid)
        && var.initializer.is_none()
        && matches!(var.ty.kind, TypeKind::Elementary(ElementaryType::UInt(_)))
        && let [Stmt { kind: StmtKind::If(cond, _, Some(else_)), .. }] = body.stmts
        && matches!(else_.kind, StmtKind::Break)
        && let ExprKind::Binary(left, op, right) = &cond.peel_parens().kind
        && ((matches!(op.kind, BinOpKind::Lt | BinOpKind::Le) && left.as_variable() == Some(*vid))
            || (matches!(op.kind, BinOpKind::Gt | BinOpKind::Ge)
                && right.as_variable() == Some(*vid)))
        && let StmtKind::Expr(update) = &update.kind
        && let ExprKind::Unary(op, target) = &update.peel_parens().kind
        && matches!(op.kind, UnOpKind::PreInc | UnOpKind::PostInc)
        && target.as_variable() == Some(*vid)
    {
        Some(loop_stmt)
    } else {
        None
    }
}

impl<'gcx> Visit<'gcx> for Checker<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Never> {
        match &stmt.kind {
            StmtKind::Block(block) => {
                if let Some(loop_stmt) = defaulted_counter_loop(self.hir, block) {
                    // Skip only the counter's declaration; all reads in the loop still run
                    // through the ordinary checker, including reads of other locals.
                    return self.visit_stmt(loop_stmt);
                }
            }
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
                for s in loop_stmts(*block, *source) {
                    self.visit_stmt(s)?;
                }
                if !matches!(source, LoopSource::DoWhile) {
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

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Never> {
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
