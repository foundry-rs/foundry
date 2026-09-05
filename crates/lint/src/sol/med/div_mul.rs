use super::DivideBeforeMultiply;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{builtins, is_revert_call, loop_update, tuple_elems},
    },
};
use solar::sema::{
    Gcx, Hir,
    builtins::Builtin,
    hir::{BinOpKind, Block, Expr, ExprKind, Function, Res, Stmt, StmtKind, VariableId},
};
use std::collections::HashSet;

declare_forge_lint!(
    DIVIDE_BEFORE_MULTIPLY,
    Severity::Med,
    "divide-before-multiply",
    "multiplication should occur before division to avoid loss of precision"
);

/// Locals whose current value is the result of a division.
type Tainted = HashSet<VariableId>;

impl<'gcx> LateLintPass<'gcx> for DivideBeforeMultiply {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        if let Some(body) = func.body {
            check_block(ctx, &gcx.hir, body, &mut Tainted::new());
        }
    }
}

/// Checks `block`, returning `false` once control cannot continue past a statement.
fn check_block<'gcx>(
    ctx: &LintContext,
    hir: &'gcx Hir<'gcx>,
    block: Block<'gcx>,
    tainted: &mut Tainted,
) -> bool {
    block.stmts.iter().all(|stmt| check_stmt(ctx, hir, stmt, tainted))
}

/// Checks the bodies of mutually exclusive branches and keeps the taint of every branch that
/// falls through, on top of the taint before the branch.
fn check_branches<'gcx>(
    ctx: &LintContext,
    hir: &'gcx Hir<'gcx>,
    blocks: impl Iterator<Item = Block<'gcx>>,
    tainted: &mut Tainted,
) {
    let mut merged = Tainted::new();
    for block in blocks {
        let mut branch_tainted = tainted.clone();
        if check_block(ctx, hir, block, &mut branch_tainted) {
            merged.extend(branch_tainted);
        }
    }
    tainted.extend(merged);
}

fn check_stmt<'gcx>(
    ctx: &LintContext,
    hir: &'gcx Hir<'gcx>,
    stmt: &'gcx Stmt<'gcx>,
    tainted: &mut Tainted,
) -> bool {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                check_expr(ctx, hir, init, tainted);
                set_taint(hir, *var_id, is_division_or_tainted(init, tainted), tainted);
            }
            true
        }
        StmtKind::DeclMulti(vars, expr) => {
            check_expr(ctx, hir, expr, tainted);
            for (var_id, is_tainted) in vars.iter().zip(rhs_taints(expr, vars.len(), tainted)) {
                if let Some(var_id) = var_id {
                    set_taint(hir, *var_id, is_tainted, tainted);
                }
            }
            true
        }
        StmtKind::Expr(expr) => {
            check_expr(ctx, hir, expr, tainted);
            !is_revert_call(expr)
        }
        StmtKind::Emit(expr) => {
            check_expr(ctx, hir, expr, tainted);
            true
        }
        StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            check_expr(ctx, hir, expr, tainted);
            false
        }
        StmtKind::Return(None) => false,
        StmtKind::If(cond, then_stmt, else_stmt) => {
            check_expr(ctx, hir, cond, tainted);
            let mut merged = Tainted::new();
            let mut falls_through = false;
            for branch in [Some(*then_stmt), *else_stmt] {
                let mut branch_tainted = tainted.clone();
                if branch.is_none_or(|stmt| check_stmt(ctx, hir, stmt, &mut branch_tainted)) {
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
            if check_block(ctx, hir, *block, &mut branch)
                && loop_update(*source)
                    .is_none_or(|update| check_stmt(ctx, hir, update, &mut branch))
            {
                tainted.extend(branch);
            }
            true
        }
        StmtKind::Try(try_stmt) => {
            check_expr(ctx, hir, &try_stmt.expr, tainted);
            check_branches(ctx, hir, try_stmt.clauses.iter().map(|c| c.block), tainted);
            true
        }
        StmtKind::Switch(switch) => {
            check_expr(ctx, hir, switch.selector, tainted);
            check_branches(ctx, hir, switch.cases.iter().map(|c| c.body), tainted);
            true
        }
        StmtKind::Block(block)
        | StmtKind::UncheckedBlock(block)
        | StmtKind::AssemblyBlock(block) => check_block(ctx, hir, *block, tainted),
        StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => true,
    }
}

fn check_expr<'gcx>(
    ctx: &LintContext,
    hir: &'gcx Hir<'gcx>,
    expr: &'gcx Expr<'gcx>,
    tainted: &mut Tainted,
) {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, op, rhs) => {
            check_expr(ctx, hir, rhs, tainted);
            check_expr(ctx, hir, lhs, tainted);
            match op.map(|op| op.kind) {
                None => match tuple_elems(lhs) {
                    Some(elems) => {
                        for (lhs, is_tainted) in
                            elems.iter().zip(rhs_taints(rhs, elems.len(), tainted))
                        {
                            if let Some(lhs) = lhs {
                                set_lhs_taint(hir, lhs, is_tainted, tainted);
                            }
                        }
                    }
                    None => set_lhs_taint(hir, lhs, is_division_or_tainted(rhs, tainted), tainted),
                },
                Some(BinOpKind::Mul) => {
                    let is_tainted = is_division_or_tainted(lhs, tainted)
                        || is_division_or_tainted(rhs, tainted);
                    if is_tainted {
                        ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
                    }
                    set_lhs_taint(hir, lhs, is_tainted, tainted);
                }
                Some(op) => set_lhs_taint(hir, lhs, op == BinOpKind::Div, tainted),
            }
        }
        ExprKind::Binary(left, op, right) => {
            check_expr(ctx, hir, left, tainted);
            check_expr(ctx, hir, right, tainted);
            if op.kind == BinOpKind::Mul
                && (is_division_or_tainted(left, tainted) || is_division_or_tainted(right, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Call(callee, args, named_args) => {
            check_expr(ctx, hir, callee, tainted);
            for arg in args.exprs() {
                check_expr(ctx, hir, arg, tainted);
            }
            for arg in named_args.iter().flat_map(|opts| opts.args) {
                check_expr(ctx, hir, &arg.value, tainted);
            }
            if is_yul_call(expr, &[Builtin::YulMul])
                && args.exprs().any(|arg| is_division_or_tainted(arg, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr(ctx, hir, cond, tainted);
            let mut then_tainted = tainted.clone();
            check_expr(ctx, hir, then_expr, &mut then_tainted);
            check_expr(ctx, hir, else_expr, tainted);
            tainted.extend(then_tainted);
        }
        ExprKind::Unary(op, inner) => {
            check_expr(ctx, hir, inner, tainted);
            if op.kind.has_side_effects() {
                set_lhs_taint(hir, inner, false, tainted);
            }
        }
        ExprKind::Array(exprs) => exprs.iter().for_each(|e| check_expr(ctx, hir, e, tainted)),
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().for_each(|e| check_expr(ctx, hir, e, tainted))
        }
        ExprKind::Index(base, index) => {
            check_expr(ctx, hir, base, tainted);
            if let Some(index) = index {
                check_expr(ctx, hir, index, tainted);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, hir, base, tainted);
            start.iter().chain(end).for_each(|e| check_expr(ctx, hir, e, tainted));
        }
        ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::YulMember(inner, _)
        | ExprKind::Payable(inner) => check_expr(ctx, hir, inner, tainted),
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
fn rhs_taints(rhs: &Expr<'_>, n: usize, tainted: &Tainted) -> Vec<bool> {
    match tuple_elems(rhs) {
        Some(elems) if elems.len() == n => {
            elems.iter().map(|e| e.is_some_and(|e| is_division_or_tainted(e, tainted))).collect()
        }
        _ => vec![is_division_or_tainted(rhs, tainted); n],
    }
}

fn set_lhs_taint(hir: &Hir<'_>, lhs: &Expr<'_>, is_tainted: bool, tainted: &mut Tainted) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(reses) => {
            for var_id in reses.iter().filter_map(Res::as_variable) {
                set_taint(hir, var_id, is_tainted, tainted);
            }
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().for_each(|e| set_lhs_taint(hir, e, is_tainted, tainted))
        }
        _ => {}
    }
}

fn set_taint(hir: &Hir<'_>, var_id: VariableId, is_tainted: bool, tainted: &mut Tainted) {
    if hir.variable(var_id).is_local_or_return() {
        if is_tainted {
            tainted.insert(var_id);
        } else {
            tainted.remove(&var_id);
        }
    }
}

/// The value of `expr` is a division result, directly or through a tainted local.
fn is_division_or_tainted(expr: &Expr<'_>, tainted: &Tainted) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Binary(_, op, _) => op.kind == BinOpKind::Div,
        ExprKind::Ident(reses) => {
            reses.iter().filter_map(Res::as_variable).any(|v| tainted.contains(&v))
        }
        ExprKind::Call(..) => is_yul_call(expr, &[Builtin::YulDiv, Builtin::YulSdiv]),
        ExprKind::YulMember(inner, _) => is_division_or_tainted(inner, tainted),
        _ => false,
    }
}

/// A two-argument call to one of the given Yul builtins.
fn is_yul_call(expr: &Expr<'_>, candidates: &[Builtin]) -> bool {
    matches!(&expr.peel_parens().kind, ExprKind::Call(callee, args, _)
        if args.len() == 2 && builtins(callee).any(|b| candidates.contains(&b)))
}
