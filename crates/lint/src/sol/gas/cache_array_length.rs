use super::CacheArrayLength;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{for_each_lhs_var, function_ids},
    },
};
use solar::{
    ast::ElementaryType,
    interface::{Span, data_structures::Never, kw, sym},
    sema::{
        Gcx,
        hir::{
            self, BinOpKind, Expr, ExprKind, LoopSource, Res, StateMutability, Stmt, StmtKind,
            VariableId, Visit as _,
        },
        ty::TyKind,
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    CACHE_ARRAY_LENGTH,
    Severity::Gas,
    "cache-array-length",
    "array length read in loop condition should be cached outside the loop"
);

impl<'gcx> LateLintPass<'gcx> for CacheArrayLength {
    fn check_stmt(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, stmt: &'gcx Stmt<'gcx>) {
        let StmtKind::Loop(block, LoopSource::For { .. }) = &stmt.kind else { return };
        // `for (init; cond; update) body` lowers to `loop { if (cond) { body } else break }` with
        // the update kept on the loop source.
        let Some(Stmt { kind: StmtKind::If(condition, _, Some(else_stmt)), .. }) =
            block.stmts.first()
        else {
            return;
        };
        if !matches!(else_stmt.kind, StmtKind::Break) {
            return;
        }

        let mut reads = Vec::new();
        collect_length_reads(gcx, condition, &mut reads);
        if reads.is_empty() {
            return;
        }

        let mut facts = LoopFacts { gcx, written: Vec::new(), skip: false };
        let _ = facts.visit_stmt(stmt);
        if facts.skip {
            return;
        }
        for (span, var) in reads {
            if !facts.written.contains(&var) {
                ctx.emit(&CACHE_ARRAY_LENGTH, span);
            }
        }
    }
}

/// Collects `<state dynamic array>.length` reads compared against an identifier in `expr`.
fn collect_length_reads<'gcx>(
    gcx: Gcx<'gcx>,
    expr: &'gcx Expr<'gcx>,
    reads: &mut Vec<(Span, VariableId)>,
) {
    let ExprKind::Binary(lhs, op, rhs) = &expr.peel_parens().kind else { return };
    match op.kind {
        BinOpKind::And | BinOpKind::Or => {
            collect_length_reads(gcx, lhs, reads);
            collect_length_reads(gcx, rhs, reads);
        }
        BinOpKind::Lt
        | BinOpKind::Le
        | BinOpKind::Gt
        | BinOpKind::Ge
        | BinOpKind::Eq
        | BinOpKind::Ne => {
            for (side, other) in [(lhs, rhs), (rhs, lhs)] {
                let side = side.peel_parens();
                if matches!(other.peel_parens().kind, ExprKind::Ident(_))
                    && let ExprKind::Member(base, member) = &side.kind
                    && member.name == sym::length
                    && let Some(var) = state_dyn_array(gcx, base)
                {
                    reads.push((side.span, var));
                }
            }
        }
        _ => {}
    }
}

/// Loop body facts that make caching unsafe: variables written, array length mutations and
/// calls that may mutate state.
struct LoopFacts<'gcx> {
    gcx: Gcx<'gcx>,
    written: Vec<VariableId>,
    skip: bool,
}

impl<'gcx> hir::Visit<'gcx> for LoopFacts<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => {
                self.skip |= is_array_like(self.gcx, lhs);
                for_each_lhs_var(lhs, &mut |v| self.written.push(v));
            }
            ExprKind::Unary(op, inner) if op.kind.has_side_effects() => {
                for_each_lhs_var(inner, &mut |v| self.written.push(v));
            }
            ExprKind::Call(callee, ..) => {
                self.skip |= call_may_mutate_state(self.gcx, callee);
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

/// Whether a call may write storage; array `push`/`pop` count since they change the length.
fn call_may_mutate_state<'gcx>(gcx: Gcx<'gcx>, callee: &'gcx Expr<'gcx>) -> bool {
    let callee = callee.peel_parens();
    match &callee.kind {
        ExprKind::Type(_) => false,
        ExprKind::Ident(_) => {
            function_ids(callee).next().is_none_or(|f| gcx.hir.function(f).mutates_state())
        }
        ExprKind::Member(base, member)
            if matches!(member.name, sym::push | kw::Pop) && is_array_like(gcx, base) =>
        {
            true
        }
        _ => !matches!(
            gcx.type_of_expr(callee.id).map(|ty| ty.peel_refs().kind),
            Some(TyKind::Fn(f)) if f.state_mutability <= StateMutability::View
        ),
    }
}

fn is_array_like<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(|ty| {
        matches!(
            ty.peel_refs().kind,
            TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
        )
    })
}

/// The state variable `expr` names, if it is a dynamic array.
fn state_dyn_array<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> Option<VariableId> {
    let expr = expr.peel_parens();
    let ExprKind::Ident(reses) = &expr.kind else { return None };
    let var = reses.iter().find_map(Res::as_variable)?;
    (gcx.hir.variable(var).is_state_variable()
        && matches!(
            gcx.type_of_expr(expr.id).map(|ty| ty.peel_refs().kind),
            Some(TyKind::DynArray(_))
        ))
    .then_some(var)
}
