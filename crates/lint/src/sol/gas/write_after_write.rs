use super::WriteAfterWrite;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::Span,
    sema::{
        Hir,
        hir::{Block, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, VariableId},
    },
};
use std::collections::HashMap;

declare_forge_lint!(
    WRITE_AFTER_WRITE,
    Severity::Gas,
    "write-after-write",
    "redundant storage write; value overwritten before being read"
);

impl<'hir> LateLintPass<'hir> for WriteAfterWrite {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            let mut pending = HashMap::default();
            check_block(ctx, hir, body, &mut pending);
        }
    }
}

fn check_block<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) {
    for stmt in block.stmts {
        check_stmt(ctx, hir, stmt, pending);
    }
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) {
    match &stmt.kind {
        StmtKind::Expr(expr) => {
            process_expr(ctx, hir, expr.peel_parens(), pending);
        }
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                collect_reads(ctx, hir, init, pending);
            }
        }
        StmtKind::DeclMulti(_, expr) => {
            collect_reads(ctx, hir, expr, pending);
        }
        // return/revert/break/continue are terminal; code after them is unreachable,
        // so the "pending" writes will never be overwritten. Reads still matter for the
        // value carried by return/revert, but after processing we must clear because no
        // subsequent statement in the same block can execute.
        StmtKind::Return(Some(expr)) => {
            collect_reads(ctx, hir, expr, pending);
            pending.clear();
        }
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {
            pending.clear();
        }
        StmtKind::Emit(expr) => {
            collect_reads(ctx, hir, expr, pending);
        }
        StmtKind::Revert(expr) => {
            collect_reads(ctx, hir, expr, pending);
            pending.clear();
        }
        // Branches and loops: recurse with a fresh map so intra-body pairs are still
        // caught, then clear the outer pending conservatively since any branch may
        // observe or skip the outer write.
        StmtKind::If(cond, then_stmt, else_stmt) => {
            collect_reads(ctx, hir, cond, pending);
            pending.clear();
            let mut branch_pending = HashMap::default();
            check_stmt(ctx, hir, then_stmt, &mut branch_pending);
            if let Some(else_stmt) = else_stmt {
                let mut else_pending = HashMap::default();
                check_stmt(ctx, hir, else_stmt, &mut else_pending);
            }
        }
        StmtKind::Loop(block, _) => {
            pending.clear();
            let mut loop_pending = HashMap::default();
            check_block(ctx, hir, *block, &mut loop_pending);
        }
        StmtKind::Try(try_stmt) => {
            collect_reads(ctx, hir, &try_stmt.expr, pending);
            pending.clear();
            for clause in try_stmt.clauses {
                let mut clause_pending = HashMap::default();
                check_block(ctx, hir, clause.block, &mut clause_pending);
            }
        }
        // Nested blocks are sequential; share the same pending map so reads inside
        // them properly invalidate outer writes.
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            check_block(ctx, hir, *block, pending);
        }
        // The placeholder `_` in a modifier body invokes the modified function, which
        // can freely read any storage variable. Conservatively clear everything.
        StmtKind::Placeholder => {
            pending.clear();
        }
        // Inline assembly or parse errors: we can't reason about what is read or
        // written, so clear conservatively (same approach as unprotected_initializer).
        StmtKind::Err(_) => {
            pending.clear();
        }
    }
}

/// Process an expression that appears as a statement, tracking writes and reads.
fn process_expr<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) {
    match &expr.kind {
        ExprKind::Assign(lhs, op, rhs) => {
            // RHS is always evaluated (read) before the assignment takes effect.
            collect_reads(ctx, hir, rhs, pending);

            if op.is_none() {
                // Plain `=`: check if LHS is a simple bare state variable.
                if let Some(var_id) = simple_state_var_id(hir, lhs) {
                    if let Some(&prev_span) = pending.get(&var_id) {
                        ctx.emit(&WRITE_AFTER_WRITE, prev_span);
                    }
                    pending.insert(var_id, expr.span);
                } else {
                    // Non-simple LHS (index/member access): the base variable is read
                    // as part of the slot computation, so treat it as a read.
                    collect_reads(ctx, hir, lhs, pending);
                }
            } else {
                // Compound assignment (+=, etc.) reads the current value of LHS first.
                collect_reads(ctx, hir, lhs, pending);
            }
        }
        ExprKind::Unary(op, inner) if op.kind.has_side_effects() => {
            // Pre/post inc/dec: read-then-write, so just consume any pending write.
            collect_reads(ctx, hir, inner, pending);
        }
        ExprKind::Delete(inner) => {
            // `delete x` is a pure write with no read of the previous value.
            if let Some(var_id) = simple_state_var_id(hir, inner) {
                if let Some(&prev_span) = pending.get(&var_id) {
                    ctx.emit(&WRITE_AFTER_WRITE, prev_span);
                }
                pending.insert(var_id, expr.span);
            } else {
                collect_reads(ctx, hir, inner, pending);
            }
        }
        // Any function/method call can observe state through re-entrancy or view
        // calls, so conservatively treat it as reading everything pending.
        ExprKind::Call(_, _, _) => {
            pending.clear();
        }
        // For any other expression used as a statement, scan for reads.
        _ => collect_reads(ctx, hir, expr, pending),
    }
}

/// Remove any state variable mentioned in `expr` from `pending` (it was read).
/// For nested assignments, delegates to `process_expr` so writes are handled correctly.
fn collect_reads<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(id)) = res
                    && hir.variable(*id).is_state_variable()
                {
                    pending.remove(id);
                }
            }
        }
        ExprKind::Assign(_, _, _) => {
            // A nested assignment (e.g. `uint256 z = (x = v)`) writes to its LHS, not
            // reads it. Delegate to process_expr so the write is tracked correctly.
            process_expr(ctx, hir, expr.peel_parens(), pending);
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_reads(ctx, hir, lhs, pending);
            collect_reads(ctx, hir, rhs, pending);
        }
        ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => {
            collect_reads(ctx, hir, inner, pending);
        }
        ExprKind::Ternary(cond, t, f) => {
            collect_reads(ctx, hir, cond, pending);
            collect_reads(ctx, hir, t, pending);
            collect_reads(ctx, hir, f, pending);
        }
        // Any call may observe storage through re-entrancy or view semantics.
        ExprKind::Call(_, _, _) => {
            pending.clear();
        }
        ExprKind::Index(base, index) => {
            collect_reads(ctx, hir, base, pending);
            if let Some(idx) = index {
                collect_reads(ctx, hir, idx, pending);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_reads(ctx, hir, base, pending);
            if let Some(s) = start {
                collect_reads(ctx, hir, s, pending);
            }
            if let Some(e) = end {
                collect_reads(ctx, hir, e, pending);
            }
        }
        ExprKind::Member(base, _) => collect_reads(ctx, hir, base, pending),
        ExprKind::Tuple(exprs) => {
            for e in exprs.iter().flatten() {
                collect_reads(ctx, hir, e, pending);
            }
        }
        ExprKind::Array(exprs) => {
            for e in *exprs {
                collect_reads(ctx, hir, e, pending);
            }
        }
        ExprKind::Delete(inner) => collect_reads(ctx, hir, inner, pending),
        ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

/// Returns `Some(id)` if the expression is a bare state variable identifier (no indexing/member).
fn simple_state_var_id(hir: &Hir<'_>, expr: &Expr<'_>) -> Option<VariableId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions.iter().find_map(|res| match res {
            Res::Item(ItemId::Variable(id)) if hir.variable(*id).is_state_variable() => Some(*id),
            _ => None,
        }),
        _ => None,
    }
}
