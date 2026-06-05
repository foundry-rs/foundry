use super::DivideBeforeMultiply;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx, Hir,
    builtins::Builtin,
    hir::{BinOpKind, Block, Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, VariableId},
};
use std::collections::HashSet;

declare_forge_lint!(
    DIVIDE_BEFORE_MULTIPLY,
    Severity::Med,
    "divide-before-multiply",
    "multiplication should occur before division to avoid loss of precision"
);

impl<'hir> LateLintPass<'hir> for DivideBeforeMultiply {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            let mut tainted = HashSet::default();
            check_block(ctx, hir, body, &mut tainted);
        }
    }
}

fn check_block<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    block: Block<'hir>,
    tainted: &mut HashSet<VariableId>,
) {
    for stmt in block.stmts {
        check_stmt(ctx, hir, stmt, tainted);
    }
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    stmt: &'hir Stmt<'hir>,
    tainted: &mut HashSet<VariableId>,
) {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                check_expr(ctx, hir, init, tainted);
                update_taint(
                    hir,
                    *var_id,
                    expr_value_is_division_or_tainted(init, tainted),
                    tainted,
                );
            }
        }
        StmtKind::DeclMulti(vars, expr) => {
            check_expr(ctx, hir, expr, tainted);
            update_multi_decl_taint(hir, vars, expr, tainted);
        }
        StmtKind::Expr(expr) => check_expr(ctx, hir, expr, tainted),
        StmtKind::Emit(expr) | StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            check_expr(ctx, hir, expr, tainted)
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            check_expr(ctx, hir, cond, tainted);

            let mut then_tainted = tainted.clone();
            check_stmt(ctx, hir, then_stmt, &mut then_tainted);

            if let Some(else_stmt) = else_stmt {
                let mut else_tainted = tainted.clone();
                check_stmt(ctx, hir, else_stmt, &mut else_tainted);
                *tainted = union_taints(&then_tainted, &else_tainted);
            } else {
                *tainted = union_taints(tainted, &then_tainted);
            }
        }
        StmtKind::Loop(block, _) => {
            let mut loop_tainted = tainted.clone();
            check_block(ctx, hir, *block, &mut loop_tainted);
            *tainted = union_taints(tainted, &loop_tainted);
        }
        StmtKind::Try(try_stmt) => {
            check_expr(ctx, hir, &try_stmt.expr, tainted);
            let mut merged_taint = tainted.clone();
            for clause in try_stmt.clauses {
                let mut clause_tainted = tainted.clone();
                check_block(ctx, hir, clause.block, &mut clause_tainted);
                merged_taint = union_taints(&merged_taint, &clause_tainted);
            }
            *tainted = merged_taint;
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            check_block(ctx, hir, *block, tainted)
        }
        StmtKind::AssemblyBlock(block) => check_block(ctx, hir, *block, tainted),
        StmtKind::Switch(switch) => {
            check_expr(ctx, hir, switch.selector, tainted);
            let mut merged_taint = tainted.clone();
            for case in switch.cases {
                let mut case_tainted = tainted.clone();
                check_block(ctx, hir, case.body, &mut case_tainted);
                merged_taint = union_taints(&merged_taint, &case_tainted);
            }
            *tainted = merged_taint;
        }
        StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Placeholder
        | StmtKind::Err(_) => {}
    }
}

fn check_expr<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    expr: &'hir Expr<'hir>,
    tainted: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, op, rhs) => {
            check_expr(ctx, hir, rhs, tainted);
            check_expr(ctx, hir, lhs, tainted);

            match op {
                None => {
                    update_assignment_taint(hir, lhs, rhs, tainted);
                }
                Some(op) if op.kind == BinOpKind::Mul => {
                    let lhs_tainted = expr_is_division_result_or_tainted(lhs, tainted);
                    let rhs_tainted = expr_is_division_result_or_tainted(rhs, tainted);
                    if lhs_tainted || rhs_tainted {
                        ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
                    }
                    update_lhs_taint(hir, lhs, lhs_tainted || rhs_tainted, tainted);
                }
                Some(op) if op.kind == BinOpKind::Div => {
                    update_lhs_taint(hir, lhs, true, tainted);
                }
                Some(_) => {}
            }
        }
        ExprKind::Binary(left, op, right) => {
            check_expr(ctx, hir, left, tainted);
            check_expr(ctx, hir, right, tainted);

            if op.kind == BinOpKind::Mul
                && (expr_is_division_result_or_tainted(left, tainted)
                    || expr_is_division_result_or_tainted(right, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                check_expr(ctx, hir, expr, tainted);
            }
        }
        ExprKind::Call(callee, args, named_args) => {
            check_expr(ctx, hir, callee, tainted);
            for arg in args.exprs() {
                check_expr(ctx, hir, arg, tainted);
            }
            if let Some(named_args) = named_args {
                for arg in named_args.args {
                    check_expr(ctx, hir, &arg.value, tainted);
                }
            }

            if is_yul_multiplication_call(expr)
                && args.exprs().any(|arg| expr_is_division_result_or_tainted(arg, tainted))
            {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
        ExprKind::Delete(inner)
        | ExprKind::Index(inner, None)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner) => check_expr(ctx, hir, inner, tainted),
        ExprKind::Index(base, Some(index)) => {
            check_expr(ctx, hir, base, tainted);
            check_expr(ctx, hir, index, tainted);
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, hir, base, tainted);
            if let Some(start) = start {
                check_expr(ctx, hir, start, tainted);
            }
            if let Some(end) = end {
                check_expr(ctx, hir, end, tainted);
            }
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr(ctx, hir, cond, tainted);
            let mut then_tainted = tainted.clone();
            check_expr(ctx, hir, then_expr, &mut then_tainted);
            let mut else_tainted = tainted.clone();
            check_expr(ctx, hir, else_expr, &mut else_tainted);
            *tainted = union_taints(&then_tainted, &else_tainted);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                check_expr(ctx, hir, expr, tainted);
            }
        }
        ExprKind::Unary(_, inner) => check_expr(ctx, hir, inner, tainted),
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_) => {}
        ExprKind::YulMember(inner, _) => check_expr(ctx, hir, inner, tainted),
        ExprKind::Err(_) => {}
    }
}

fn update_multi_decl_taint(
    hir: &Hir<'_>,
    vars: &[Option<VariableId>],
    expr: &Expr<'_>,
    tainted: &mut HashSet<VariableId>,
) {
    if let ExprKind::Tuple(exprs) = &expr.peel_parens().kind
        && exprs.len() == vars.len()
    {
        let rhs_taints: Vec<_> = exprs
            .iter()
            .map(|expr| expr.is_some_and(|expr| expr_value_is_division_or_tainted(expr, tainted)))
            .collect();
        for (var_id, rhs_tainted) in vars.iter().zip(rhs_taints) {
            if let Some(var_id) = var_id {
                update_taint(hir, *var_id, rhs_tainted, tainted);
            }
        }
        return;
    }

    let rhs_tainted = expr_value_is_division_or_tainted(expr, tainted);
    for var_id in vars.iter().flatten() {
        update_taint(hir, *var_id, rhs_tainted, tainted);
    }
}

fn update_assignment_taint(
    hir: &Hir<'_>,
    lhs: &Expr<'_>,
    rhs: &Expr<'_>,
    tainted: &mut HashSet<VariableId>,
) {
    if let (ExprKind::Tuple(lhs_exprs), ExprKind::Tuple(rhs_exprs)) =
        (&lhs.peel_parens().kind, &rhs.peel_parens().kind)
        && lhs_exprs.len() == rhs_exprs.len()
    {
        let rhs_taints: Vec<_> = rhs_exprs
            .iter()
            .map(|rhs| rhs.is_some_and(|rhs| expr_value_is_division_or_tainted(rhs, tainted)))
            .collect();
        for (lhs, rhs_tainted) in lhs_exprs.iter().zip(rhs_taints) {
            if let Some(lhs) = lhs {
                update_lhs_taint(hir, lhs, rhs_tainted, tainted);
            }
        }
        return;
    }

    update_lhs_taint(hir, lhs, expr_value_is_division_or_tainted(rhs, tainted), tainted);
}

fn union_taints(left: &HashSet<VariableId>, right: &HashSet<VariableId>) -> HashSet<VariableId> {
    left.union(right).copied().collect()
}

fn update_lhs_taint(
    hir: &Hir<'_>,
    lhs: &Expr<'_>,
    is_tainted: bool,
    tainted: &mut HashSet<VariableId>,
) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    update_taint(hir, *var_id, is_tainted, tainted);
                }
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                update_lhs_taint(hir, expr, is_tainted, tainted);
            }
        }
        _ => {}
    }
}

fn update_taint(
    hir: &Hir<'_>,
    var_id: VariableId,
    is_tainted: bool,
    tainted: &mut HashSet<VariableId>,
) {
    if !hir.variable(var_id).is_local_or_return() {
        return;
    }
    if is_tainted {
        tainted.insert(var_id);
    } else {
        tainted.remove(&var_id);
    }
}

fn expr_value_is_division_or_tainted(expr: &Expr<'_>, tainted: &HashSet<VariableId>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Binary(_, op, _) => op.kind == BinOpKind::Div,
        ExprKind::Ident(resolutions) => resolutions.iter().any(
            |res| matches!(res, Res::Item(ItemId::Variable(var_id)) if tainted.contains(var_id)),
        ),
        ExprKind::Call(..) => is_yul_division_call(expr),
        ExprKind::Tuple([Some(inner)]) => expr_value_is_division_or_tainted(inner, tainted),
        ExprKind::YulMember(inner, _) => expr_value_is_division_or_tainted(inner, tainted),
        ExprKind::Array(_)
        | ExprKind::Assign(..)
        | ExprKind::Delete(_)
        | ExprKind::Index(..)
        | ExprKind::Lit(_)
        | ExprKind::Member(_, _)
        | ExprKind::New(_)
        | ExprKind::Payable(_)
        | ExprKind::Slice(..)
        | ExprKind::Ternary(..)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Unary(_, _)
        | ExprKind::Tuple(_) => false,
        ExprKind::Err(_) => false,
    }
}

fn expr_is_division_result_or_tainted(expr: &Expr<'_>, tainted: &HashSet<VariableId>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Binary(_, op, _) => op.kind == BinOpKind::Div,
        ExprKind::Call(..) => is_yul_division_call(expr),
        ExprKind::Ident(resolutions) => resolutions.iter().any(
            |res| matches!(res, Res::Item(ItemId::Variable(var_id)) if tainted.contains(var_id)),
        ),
        ExprKind::Tuple([Some(inner)]) => expr_is_division_result_or_tainted(inner, tainted),
        _ => false,
    }
}

fn is_yul_division_call(expr: &Expr<'_>) -> bool {
    is_yul_builtin_call(expr, |builtin| matches!(builtin, Builtin::YulDiv | Builtin::YulSdiv))
}

fn is_yul_multiplication_call(expr: &Expr<'_>) -> bool {
    is_yul_builtin_call(expr, |builtin| matches!(builtin, Builtin::YulMul))
}

fn is_yul_builtin_call(expr: &Expr<'_>, predicate: impl Fn(Builtin) -> bool) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    if args.len() != 2 {
        return false;
    }
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return false };
    resolutions.iter().any(|res| matches!(res, Res::Builtin(builtin) if predicate(*builtin)))
}
