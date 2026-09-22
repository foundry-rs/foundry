use super::DivideBeforeMultiply;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_revert_call, loop_update, tuple_elems},
    },
};
use solar::sema::{
    Gcx,
    builtins::Builtin,
    hir::{BinOpKind, Block, Expr, ExprKind, Function, Stmt, StmtKind, VariableId},
};
use std::collections::HashSet;

declare_forge_lint!(
    DIVIDE_BEFORE_MULTIPLY,
    Severity::Med,
    "divide-before-multiply",
    "division before multiplication may lose precision"
);

/// Locals whose current value is the result of a division.
type Tainted = HashSet<VariableId>;

impl<'gcx> LateLintPass<'gcx> for DivideBeforeMultiply {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        if let Some(body) = func.body {
            check_block(ctx, gcx, body, &mut Tainted::new());
        }
    }
}

/// Checks `block`, returning `false` once control cannot continue past a statement.
fn check_block<'gcx>(
    ctx: &LintContext,
    gcx: Gcx<'gcx>,
    block: Block<'gcx>,
    tainted: &mut Tainted,
) -> bool {
    block.stmts.iter().all(|stmt| check_stmt(ctx, gcx, stmt, tainted))
}

/// Checks the bodies of mutually exclusive branches and keeps the taint of every branch that
/// falls through, on top of the taint before the branch.
fn check_branches<'gcx>(
    ctx: &LintContext,
    gcx: Gcx<'gcx>,
    blocks: impl Iterator<Item = Block<'gcx>>,
    tainted: &mut Tainted,
) {
    let mut merged = Tainted::new();
    for block in blocks {
        let mut branch_tainted = tainted.clone();
        if check_block(ctx, gcx, block, &mut branch_tainted) {
            merged.extend(branch_tainted);
        }
    }
    tainted.extend(merged);
}

fn check_stmt<'gcx>(
    ctx: &LintContext,
    gcx: Gcx<'gcx>,
    stmt: &'gcx Stmt<'gcx>,
    tainted: &mut Tainted,
) -> bool {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = gcx.hir.variable(*var_id).initializer {
                check_expr(ctx, gcx, init, tainted);
                set_taint(gcx, *var_id, is_division_or_tainted(gcx, init, tainted), tainted);
            }
            true
        }
        StmtKind::DeclMulti(vars, expr) => {
            check_expr(ctx, gcx, expr, tainted);
            for (var_id, is_tainted) in vars.iter().zip(rhs_taints(gcx, expr, vars.len(), tainted))
            {
                if let Some(var_id) = var_id {
                    set_taint(gcx, *var_id, is_tainted, tainted);
                }
            }
            true
        }
        StmtKind::Expr(expr) => {
            check_expr(ctx, gcx, expr, tainted);
            !is_revert_call(gcx, expr)
        }
        StmtKind::Emit(expr) => {
            check_expr(ctx, gcx, expr, tainted);
            true
        }
        StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            check_expr(ctx, gcx, expr, tainted);
            false
        }
        StmtKind::Return(None) => false,
        StmtKind::If(cond, then_stmt, else_stmt) => {
            check_expr(ctx, gcx, cond, tainted);
            let mut merged = Tainted::new();
            let mut falls_through = false;
            for branch in [Some(*then_stmt), *else_stmt] {
                let mut branch_tainted = tainted.clone();
                if branch.is_none_or(|stmt| check_stmt(ctx, gcx, stmt, &mut branch_tainted)) {
                    merged.extend(branch_tainted);
                    falls_through = true;
                }
            }
            if falls_through {
                *tainted = merged;
            }
            falls_through
        }
        StmtKind::Loop(block, source) => {
            let mut branch = tainted.clone();
            if check_block(ctx, gcx, *block, &mut branch)
                && loop_update(*source)
                    .is_none_or(|update| check_stmt(ctx, gcx, update, &mut branch))
            {
                tainted.extend(branch);
            }
            true
        }
        StmtKind::Try(try_stmt) => {
            check_expr(ctx, gcx, &try_stmt.expr, tainted);
            check_branches(ctx, gcx, try_stmt.clauses.iter().map(|c| c.block), tainted);
            true
        }
        StmtKind::Switch(switch) => {
            check_expr(ctx, gcx, switch.selector, tainted);
            check_branches(ctx, gcx, switch.cases.iter().map(|c| c.body), tainted);
            true
        }
        StmtKind::Block(block)
        | StmtKind::UncheckedBlock(block)
        | StmtKind::AssemblyBlock(block) => check_block(ctx, gcx, *block, tainted),
        StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => true,
    }
}

fn check_expr<'gcx>(
    ctx: &LintContext,
    gcx: Gcx<'gcx>,
    expr: &'gcx Expr<'gcx>,
    tainted: &mut Tainted,
) {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, op, rhs) => {
            check_expr(ctx, gcx, rhs, tainted);
            check_expr(ctx, gcx, lhs, tainted);
            match op.map(|op| op.kind) {
                None => match tuple_elems(lhs) {
                    Some(elems) => {
                        for (lhs, is_tainted) in
                            elems.iter().zip(rhs_taints(gcx, rhs, elems.len(), tainted))
                        {
                            if let Some(lhs) = lhs {
                                set_lhs_taint(gcx, lhs, is_tainted, tainted);
                            }
                        }
                    }
                    None => {
                        set_lhs_taint(gcx, lhs, is_division_or_tainted(gcx, rhs, tainted), tainted)
                    }
                },
                Some(BinOpKind::Mul) => {
                    let is_tainted = is_division_or_tainted(gcx, lhs, tainted)
                        || is_division_or_tainted(gcx, rhs, tainted);
                    if is_tainted {
                        ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
                    }
                    set_lhs_taint(gcx, lhs, is_tainted, tainted);
                }
                Some(op) => set_lhs_taint(gcx, lhs, op == BinOpKind::Div, tainted),
            }
        }
        ExprKind::Binary(left, op, right) => {
            check_expr(ctx, gcx, left, tainted);
            check_expr(ctx, gcx, right, tainted);
            if op.kind == BinOpKind::Mul
                && (is_division_or_tainted(gcx, left, tainted)
                    || is_division_or_tainted(gcx, right, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Call(callee, args, named_args) => {
            check_expr(ctx, gcx, callee, tainted);
            for arg in args.exprs() {
                check_expr(ctx, gcx, arg, tainted);
            }
            for arg in named_args.iter().flat_map(|opts| opts.args) {
                check_expr(ctx, gcx, &arg.value, tainted);
            }
            if is_yul_call(gcx, expr, &[Builtin::YulMul])
                && args.exprs().any(|arg| is_division_or_tainted(gcx, arg, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr(ctx, gcx, cond, tainted);
            let mut then_tainted = tainted.clone();
            check_expr(ctx, gcx, then_expr, &mut then_tainted);
            check_expr(ctx, gcx, else_expr, tainted);
            tainted.extend(then_tainted);
        }
        ExprKind::Unary(op, inner) => {
            check_expr(ctx, gcx, inner, tainted);
            if op.kind.has_side_effects() {
                set_lhs_taint(gcx, inner, false, tainted);
            }
        }
        ExprKind::Array(exprs) => exprs.iter().for_each(|e| check_expr(ctx, gcx, e, tainted)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().for_each(|e| check_expr(ctx, gcx, e, tainted))
        }
        ExprKind::Index(base, index) => {
            check_expr(ctx, gcx, base, tainted);
            if let Some(index) = index {
                check_expr(ctx, gcx, index, tainted);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, gcx, base, tainted);
            start.iter().chain(end).for_each(|e| check_expr(ctx, gcx, e, tainted));
        }
        ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::YulMember(inner, _)
        | ExprKind::Payable(inner) => check_expr(ctx, gcx, inner, tainted),
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

/// Taint of each of the `n` slots assigned from `rhs`: elementwise for a tuple of matching arity,
/// otherwise the taint of the whole expression.
fn rhs_taints(gcx: Gcx<'_>, rhs: &Expr<'_>, n: usize, tainted: &Tainted) -> Vec<bool> {
    match tuple_elems(rhs) {
        Some(elems) if elems.len() == n => elems
            .iter()
            .map(|e| e.is_some_and(|e| is_division_or_tainted(gcx, e, tainted)))
            .collect(),
        _ => vec![is_division_or_tainted(gcx, rhs, tainted); n],
    }
}

fn set_lhs_taint(gcx: Gcx<'_>, lhs: &Expr<'_>, is_tainted: bool, tainted: &mut Tainted) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(_) => {
            if let Some(var_id) = gcx.resolved_variable(lhs) {
                set_taint(gcx, var_id, is_tainted, tainted);
            }
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().for_each(|e| set_lhs_taint(gcx, e, is_tainted, tainted))
        }
        _ => {}
    }
}

fn set_taint(gcx: Gcx<'_>, var_id: VariableId, is_tainted: bool, tainted: &mut Tainted) {
    if gcx.hir.variable(var_id).is_local_or_return() {
        if is_tainted {
            tainted.insert(var_id);
        } else {
            tainted.remove(&var_id);
        }
    }
}

/// The value of `expr` is a division result, directly or through a tainted local.
fn is_division_or_tainted(gcx: Gcx<'_>, expr: &Expr<'_>, tainted: &Tainted) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Binary(_, op, _) => op.kind == BinOpKind::Div,
        ExprKind::Ident(_) => gcx.resolved_variable(expr).is_some_and(|v| tainted.contains(&v)),
        ExprKind::Call(..) => is_yul_call(gcx, expr, &[Builtin::YulDiv, Builtin::YulSdiv]),
        ExprKind::YulMember(inner, _) => is_division_or_tainted(gcx, inner, tainted),
        _ => false,
    }
}

/// A two-argument call to one of the given Yul builtins.
fn is_yul_call(gcx: Gcx<'_>, expr: &Expr<'_>, candidates: &[Builtin]) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Call(callee, args, _)
        if args.len() == 2 && gcx.resolved_builtin(callee).is_some_and(|b| candidates.contains(&b)))
}
