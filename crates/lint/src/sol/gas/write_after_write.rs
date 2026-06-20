use super::WriteAfterWrite;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::Span,
    sema::{
        Hir,
        hir::{
            BinOpKind, Block, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, VariableId,
        },
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
        _gcx: solar::sema::Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            let mut pending = HashMap::default();
            check_block(ctx, hir, body, &mut pending);
        }
    }
}

/// Whether control flow continues past this statement/block.
#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

fn check_block<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) -> Flow {
    for stmt in block.stmts {
        if check_stmt(ctx, hir, stmt, pending) == Flow::Stop {
            return Flow::Stop;
        }
    }
    Flow::Continue
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    pending: &mut HashMap<VariableId, Span>,
) -> Flow {
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
            return Flow::Stop;
        }
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {
            pending.clear();
            return Flow::Stop;
        }
        StmtKind::Emit(expr) => {
            // Emit only logs; it doesn't invoke external code that could observe state.
            // Walk the call args directly so pending is not cleared.
            if let ExprKind::Call(callee, args, named_args) = &expr.peel_parens().kind {
                collect_reads(ctx, hir, callee, pending);
                for arg in args.exprs() {
                    collect_reads(ctx, hir, arg, pending);
                }
                walk_named_args(ctx, hir, named_args, pending);
            } else {
                collect_reads(ctx, hir, expr, pending);
            }
        }
        StmtKind::Revert(expr) => {
            collect_reads(ctx, hir, expr, pending);
            pending.clear();
            return Flow::Stop;
        }
        // Branches and loops: recurse with a fresh map so intra-body pairs are still
        // caught, then clear the outer pending conservatively since any branch may
        // observe or skip the outer write.
        // Propagate Stop only when both branches unconditionally stop (no else = Continue).
        StmtKind::If(cond, then_stmt, else_stmt) => {
            collect_reads(ctx, hir, cond, pending);
            pending.clear();
            let mut branch_pending = HashMap::default();
            let then_flow = check_stmt(ctx, hir, then_stmt, &mut branch_pending);
            if let Some(else_stmt) = else_stmt {
                let mut else_pending = HashMap::default();
                let else_flow = check_stmt(ctx, hir, else_stmt, &mut else_pending);
                if then_flow == Flow::Stop && else_flow == Flow::Stop {
                    return Flow::Stop;
                }
            }
        }
        StmtKind::Loop(block, _) => {
            pending.clear();
            let mut loop_pending = HashMap::default();
            check_block(ctx, hir, *block, &mut loop_pending);
            // A loop may execute zero times, so it never guarantees Stop for outer flow.
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
        // them properly invalidate outer writes. Propagate terminal flow outward.
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            return check_block(ctx, hir, *block, pending);
        }
        // The placeholder `_` in a modifier body invokes the modified function, which
        // can freely read any storage variable. Conservatively clear everything.
        StmtKind::Placeholder => {
            pending.clear();
        }
        // Inline assembly or parse errors: we can't reason about what is read or
        // written, so clear conservatively (same approach as unprotected_initializer).
        StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
            pending.clear();
        }
    }
    Flow::Continue
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
                // Plain `=`: recursively handle the LHS as a write target.
                process_assignment_lhs(ctx, hir, lhs, expr.span, pending);
            } else {
                // Compound assignment (+=, etc.) reads the current value of LHS first.
                collect_reads(ctx, hir, lhs, pending);
            }
        }
        ExprKind::Unary(op, inner) if op.kind.has_side_effects() => {
            // Pre/post inc/dec: reads the variable, then writes it.
            collect_reads(ctx, hir, inner, pending);
            if let Some(var_id) = simple_state_var_id(hir, inner) {
                pending.insert(var_id, expr.span);
            }
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
        // Walk callee, arguments, and call options first — all are evaluated before the call.
        ExprKind::Call(callee, args, named_args) => {
            collect_reads(ctx, hir, callee, pending);
            for arg in args.exprs() {
                collect_reads(ctx, hir, arg, pending);
            }
            walk_named_args(ctx, hir, named_args, pending);
            pending.clear();
        }
        // For any other expression used as a statement, scan for reads.
        _ => collect_reads(ctx, hir, expr, pending),
    }
}

/// Recursively handle a plain-`=` assignment LHS, tracking each component as a write.
/// For tuple destructuring `(x, y) = ...`, each element is processed independently.
fn process_assignment_lhs<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    lhs: &'hir Expr<'hir>,
    assign_span: Span,
    pending: &mut HashMap<VariableId, Span>,
) {
    match &lhs.peel_parens().kind {
        ExprKind::Tuple(exprs) => {
            for e in exprs.iter().flatten() {
                process_assignment_lhs(ctx, hir, e, e.span, pending);
            }
        }
        _ => {
            if let Some(var_id) = simple_state_var_id(hir, lhs) {
                if let Some(&prev_span) = pending.get(&var_id) {
                    ctx.emit(&WRITE_AFTER_WRITE, prev_span);
                }
                pending.insert(var_id, assign_span);
            } else {
                // Non-simple LHS (index/member access): slot computation reads the base.
                collect_reads(ctx, hir, lhs, pending);
            }
        }
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
        // Short-circuit operators: LHS always evaluates, RHS may not.
        // Clear outer pending before RHS to avoid false-positive WAW in the conditional path.
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            collect_reads(ctx, hir, lhs, pending);
            pending.clear();
            let mut rhs_pending = HashMap::default();
            collect_reads(ctx, hir, rhs, &mut rhs_pending);
        }
        ExprKind::Binary(lhs, _, rhs) => {
            collect_reads(ctx, hir, lhs, pending);
            collect_reads(ctx, hir, rhs, pending);
        }
        ExprKind::Unary(_, inner) | ExprKind::Payable(inner) => {
            collect_reads(ctx, hir, inner, pending);
        }
        // Ternary arms are mutually exclusive; analyze each independently with a fresh
        // pending to avoid false-positive WAW between branches.
        ExprKind::Ternary(cond, t, f) => {
            collect_reads(ctx, hir, cond, pending);
            pending.clear();
            let mut then_pending = HashMap::default();
            let mut else_pending = HashMap::default();
            collect_reads(ctx, hir, t, &mut then_pending);
            collect_reads(ctx, hir, f, &mut else_pending);
        }
        // Any call may observe storage through re-entrancy or view semantics.
        // Walk callee, arguments, and call options first — all evaluated before the call.
        ExprKind::Call(callee, args, named_args) => {
            collect_reads(ctx, hir, callee, pending);
            for arg in args.exprs() {
                collect_reads(ctx, hir, arg, pending);
            }
            walk_named_args(ctx, hir, named_args, pending);
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
        // A nested `delete` is a write; delegate to process_expr for correct tracking.
        ExprKind::Delete(_) => {
            process_expr(ctx, hir, expr.peel_parens(), pending);
        }
        ExprKind::Lit(_) | ExprKind::New(_) | ExprKind::TypeCall(_) | ExprKind::Type(_) => {}
        ExprKind::YulMember(..) | ExprKind::Err(_) => {
            pending.clear();
        }
    }
}

/// Walk named call arguments (e.g. `{value: expr, gas: expr}`) for reads and writes.
fn walk_named_args<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    named_args: &Option<&'hir solar::sema::hir::CallOptions<'hir>>,
    pending: &mut HashMap<VariableId, Span>,
) {
    if let Some(named) = named_args {
        for na in named.args {
            collect_reads(ctx, hir, &na.value, pending);
        }
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
