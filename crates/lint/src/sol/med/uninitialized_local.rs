use super::UninitializedLocal;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{Span, data_structures::Never},
    sema::{
        Hir,
        hir::{
            Expr, ExprKind, Function, ItemId, LoopSource, Res, Stmt, StmtKind, TypeKind, VarKind,
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
        _gcx: solar::sema::Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let Some(body) = func.body else { return };

        let mut checker = Checker { hir, uninitialized: HashSet::new(), findings: HashMap::new() };
        for stmt in body.stmts {
            let _ = checker.visit_stmt(stmt);
        }

        for (_vid, read_span) in checker.findings {
            ctx.emit(&UNINITIALIZED_LOCAL, read_span);
        }
    }
}

struct Checker<'hir> {
    hir: &'hir Hir<'hir>,
    /// Locals declared without an initializer that have not yet been written.
    uninitialized: HashSet<VariableId>,
    /// First read span per variable that was read while uninitialized.
    findings: HashMap<VariableId, Span>,
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
                let is_value_type =
                    matches!(v.ty.kind, TypeKind::Elementary(ty) if ty.is_value_type());
                if matches!(v.kind, VarKind::Statement) && v.initializer.is_none() && is_value_type
                {
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

                let then_exits = branch_always_exits(then);
                let else_exits = else_.is_some_and(branch_always_exits);
                self.uninitialized = match (then_exits, else_exits) {
                    (true, _) => after_else,
                    (_, true) => after_then,
                    _ => after_then.union(&after_else).copied().collect(),
                };
                return ControlFlow::Continue(());
            }

            // do-while always executes the body once, so writes are guaranteed.
            // for/while may execute zero times, so writes must be discarded.
            StmtKind::Loop(block, source) => {
                let before = self.uninitialized.clone();
                for s in block.stmts {
                    let _ = self.visit_stmt(s);
                }
                if !matches!(source, LoopSource::DoWhile) {
                    self.uninitialized = before;
                }
                return ControlFlow::Continue(());
            }

            // Each try/catch clause is an independent execution path; treat like if/else branches.
            StmtKind::Try(t) => {
                let _ = self.visit_expr(&t.expr);
                let mut clause_states: Vec<HashSet<VariableId>> = Vec::new();
                for clause in t.clauses {
                    let before = self.uninitialized.clone();
                    for s in clause.block.stmts {
                        let _ = self.visit_stmt(s);
                    }
                    clause_states.push(self.uninitialized.clone());
                    self.uninitialized = before;
                }
                // Union across all clause post-states: variable stays uninitialized if any clause
                // fails to write it.
                self.uninitialized = clause_states
                    .iter()
                    .fold(HashSet::new(), |acc, s| acc.union(s).copied().collect());
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

            // `delete x` is an explicit write to the zero value — not a read.
            ExprKind::Delete(target) => {
                mark_written(target, &mut self.uninitialized);
                let _ = self.visit_expr(target);
                return ControlFlow::Continue(());
            }

            ExprKind::Ident(reses) => {
                for res in *reses {
                    if let Res::Item(ItemId::Variable(vid)) = res
                        && self.uninitialized.contains(vid)
                    {
                        self.findings.entry(*vid).or_insert(expr.span);
                        break;
                    }
                }
            }

            _ => {}
        }
        self.walk_expr(expr)
    }
}

fn branch_always_exits(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Revert(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.last().is_some_and(branch_always_exits)
        }
        StmtKind::If(_, t, Some(e)) => branch_always_exits(t) && branch_always_exits(e),
        _ => false,
    }
}

/// Remove `expr` from `uninitialized` if it is a direct identifier or a tuple of identifiers.
fn mark_written(expr: &Expr<'_>, uninitialized: &mut HashSet<VariableId>) {
    match &expr.kind {
        ExprKind::Ident(reses) => {
            for res in *reses {
                if let Res::Item(ItemId::Variable(vid)) = res {
                    uninitialized.remove(vid);
                }
            }
        }
        ExprKind::Tuple(elems) => {
            for elem in elems.iter().flatten() {
                mark_written(elem, uninitialized);
            }
        }
        _ => {}
    }
}
