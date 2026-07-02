use super::EnumerableLoopRemoval;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::Span,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, Hir, Stmt, StmtKind, Visit},
        ty::TyKind,
    },
};
use std::{collections::HashSet, convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    ENUMERABLE_LOOP_REMOVAL,
    Severity::High,
    "enumerable-loop-removal",
    "`remove` on an EnumerableSet inside a loop that iterates it with `at` corrupts the iteration"
);

impl<'hir> LateLintPass<'hir> for EnumerableLoopRemoval {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir hir::Function<'hir>,
    ) {
        // EnumerableSet removal is swap-and-pop, so removing while iterating the set by index
        // in the same loop skips elements or reads out-of-bounds indices. The safe pattern
        // (collect during the loop, remove in a later loop without `at`) stays clean.
        if let Some(body) = &func.body {
            let mut finder = LoopFinder { gcx, hir, ctx, emitted: HashSet::new() };
            // A `Block` has no dedicated visit hook: walk its statements directly.
            for stmt in body.stmts {
                let _ = finder.visit_stmt(stmt);
            }
        }
    }
}

/// Walks a function body and, for each loop, flags the EnumerableSet `remove` calls of its
/// subtree when that same subtree also iterates an EnumerableSet with `at`.
struct LoopFinder<'ctx, 's, 'c, 'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    ctx: &'ctx LintContext<'s, 'c>,
    // A loop nested in a flagged loop sees the same calls: dedupe emissions by span.
    emitted: HashSet<Span>,
}

impl<'hir> Visit<'hir> for LoopFinder<'_, '_, '_, 'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        // Every loop form lowers to `StmtKind::Loop`, so one case covers for/while/do-while.
        if let StmtKind::Loop(body, _) = &stmt.kind {
            let mut ops = SetOpsCollector {
                gcx: self.gcx,
                hir: self.hir,
                removes: Vec::new(),
                has_at: false,
            };
            for inner in body.stmts {
                let _ = ops.visit_stmt(inner);
            }
            // Only the combination is dangerous: `remove` without `at` in the loop is the
            // recommended collect-then-remove pattern, `at` without `remove` is a plain read.
            if ops.has_at {
                for span in ops.removes {
                    if self.emitted.insert(span) {
                        self.ctx.emit(&ENUMERABLE_LOOP_REMOVAL, span);
                    }
                }
            }
        }
        self.walk_stmt(stmt)
    }
}

/// Collects the EnumerableSet `remove` call spans of a subtree and whether the subtree also
/// calls `at` on an EnumerableSet (not necessarily the same instance).
struct SetOpsCollector<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    removes: Vec<Span>,
    has_at: bool,
}

impl<'hir> Visit<'hir> for SetOpsCollector<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        // Only method-call shapes matter: `set.remove(value)` and `set.at(index)` where the
        // receiver is a struct declared in a library named `EnumerableSet`.
        if let ExprKind::Call(callee, _, _) = &expr.kind
            && let ExprKind::Member(receiver, member) = &callee.peel_parens().kind
            && receiver_is_enumerable_set(self.gcx, self.hir, receiver)
        {
            if member.as_str() == "remove" {
                self.removes.push(expr.span);
            } else if member.as_str() == "at" {
                self.has_at = true;
            }
        }
        self.walk_expr(expr)
    }
}

/// Whether `receiver` is a struct defined in a library (or contract) named `EnumerableSet`,
/// matching OpenZeppelin's `EnumerableSet.AddressSet` / `UintSet` / `Bytes32Set`.
fn receiver_is_enumerable_set<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    receiver: &Expr<'_>,
) -> bool {
    let Some(ty) = gcx.type_of_expr(receiver.peel_parens().id) else { return false };
    let TyKind::Struct(id) = ty.peel_refs().kind else { return false };
    let Some(contract_id) = hir.strukt(id).contract else { return false };
    hir.contract(contract_id).name.as_str() == "EnumerableSet"
}
