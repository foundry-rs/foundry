use super::CostlyLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::DataLocation,
    interface::data_structures::Never,
    sema::{
        Gcx, Hir,
        builtins::Builtin,
        hir::{self, Expr, ExprKind, Function, Res, Stmt, StmtKind, Visit as _},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(COSTLY_LOOP, Severity::Gas, "costly-loop", "storage write inside a loop");

impl<'hir> LateLintPass<'hir> for CostlyLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let mut finder = LoopWriteFinder { ctx, gcx, hir, loop_depth: 0 };
        let _ = finder.visit_function(func);
    }
}

struct LoopWriteFinder<'a, 'hir> {
    ctx: &'a LintContext<'a, 'a>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    loop_depth: u32,
}

impl<'hir> hir::Visit<'hir> for LoopWriteFinder<'_, 'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_stmt(&mut self, stmt: &'hir Stmt<'hir>) -> ControlFlow<Self::BreakValue> {
        let is_loop = matches!(stmt.kind, StmtKind::Loop(..));
        self.loop_depth += is_loop as u32;
        let flow = self.walk_stmt(stmt);
        self.loop_depth -= is_loop as u32;
        flow
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if self.loop_depth > 0 {
            let lvalue = match &expr.kind {
                ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => Some(lhs),
                ExprKind::Unary(op, inner) if op.kind.has_side_effects() => Some(inner),
                _ => None,
            };
            if lvalue.is_some_and(|lvalue| lvalue_is_state_var(self.gcx, self.hir, lvalue)) {
                self.ctx.emit(&COSTLY_LOOP, expr.span);
            }
        }
        self.walk_expr(expr)
    }
}

/// Returns `true` if the lvalue expression ultimately writes to a storage variable.
///
/// Peels through index accesses, member accesses, and slices to find a state variable or an
/// expression that returns a storage reference.
fn lvalue_is_state_var(gcx: Gcx<'_>, hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .find_map(Res::as_variable)
            .is_some_and(|id| hir.variable(id).is_state_variable()),
        ExprKind::Call(callee, ..) => {
            gcx.resolved_builtin(callee) == Some(Builtin::ArrayPush0)
                || gcx
                    .type_of_expr(expr.id)
                    .is_some_and(|ty| ty.loc() == Some(DataLocation::Storage))
        }
        ExprKind::Index(base, _)
        | ExprKind::Slice(base, _, _)
        | ExprKind::Member(base, _)
        | ExprKind::Payable(base) => lvalue_is_state_var(gcx, hir, base),
        ExprKind::Tuple(exprs) => exprs.iter().flatten().any(|e| lvalue_is_state_var(gcx, hir, e)),
        _ => false,
    }
}
