use super::BlockTimestamp;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast,
    interface::{kw, sym},
    sema::{
        Gcx, Hir,
        builtins::Builtin,
        hir::{
            BinOpKind, Block, ContractId, Expr, ExprKind, Function, FunctionId, ItemId, Res, Stmt,
            StmtKind, VariableId,
        },
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    BLOCK_TIMESTAMP,
    Severity::Low,
    "block-timestamp",
    "usage of `block.timestamp` in a comparison may be manipulated by validators"
);

impl<'hir> LateLintPass<'hir> for BlockTimestamp {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        if let Some(body) = func.body {
            let helpers = timestamp_helpers(hir, func.contract);
            let mut aliases = HashSet::new();
            check_block(ctx, hir, &helpers, body, &mut aliases);
        }
    }
}

fn check_block<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    helpers: &HashSet<FunctionId>,
    block: Block<'hir>,
    aliases: &mut HashSet<VariableId>,
) -> bool {
    for stmt in block.stmts {
        if !check_stmt(ctx, hir, helpers, stmt, aliases) {
            return false;
        }
    }
    true
}

fn check_stmt<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    helpers: &HashSet<FunctionId>,
    stmt: &'hir Stmt<'hir>,
    aliases: &mut HashSet<VariableId>,
) -> bool {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => {
            if let Some(init) = hir.variable(*var_id).initializer {
                check_expr(ctx, hir, helpers, init, aliases);
                update_alias(
                    hir,
                    *var_id,
                    expr_value_is_timestamp_source(helpers, init, aliases),
                    aliases,
                );
            }
            true
        }
        StmtKind::DeclMulti(vars, expr) => {
            check_expr(ctx, hir, helpers, expr, aliases);
            update_multi_aliases(hir, helpers, vars, expr, aliases);
            true
        }
        StmtKind::Expr(expr) => {
            check_expr(ctx, hir, helpers, expr, aliases);
            !is_revert_call(expr)
        }
        StmtKind::Emit(expr) => {
            check_expr(ctx, hir, helpers, expr, aliases);
            true
        }
        StmtKind::Revert(expr) | StmtKind::Return(Some(expr)) => {
            check_expr(ctx, hir, helpers, expr, aliases);
            false
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            check_expr(ctx, hir, helpers, cond, aliases);

            let baseline = aliases.clone();
            let mut merged = HashSet::new();
            let mut falls_through = false;

            let mut then_aliases = baseline.clone();
            if check_stmt(ctx, hir, helpers, then_stmt, &mut then_aliases) {
                merged.extend(then_aliases);
                falls_through = true;
            }

            if let Some(else_stmt) = else_stmt {
                let mut else_aliases = baseline;
                if check_stmt(ctx, hir, helpers, else_stmt, &mut else_aliases) {
                    merged.extend(else_aliases);
                    falls_through = true;
                }
            } else {
                merged.extend(baseline);
                falls_through = true;
            }

            if falls_through {
                *aliases = merged;
            }
            falls_through
        }
        StmtKind::Loop(block, _) => {
            let baseline = aliases.clone();
            let mut loop_aliases = baseline.clone();
            *aliases = if check_block(ctx, hir, helpers, *block, &mut loop_aliases) {
                baseline.union(&loop_aliases).copied().collect()
            } else {
                baseline
            };
            true
        }
        StmtKind::Try(try_stmt) => {
            check_expr(ctx, hir, helpers, &try_stmt.expr, aliases);

            let baseline = aliases.clone();
            let mut merged = baseline.clone();
            for clause in try_stmt.clauses {
                let mut clause_aliases = baseline.clone();
                if check_block(ctx, hir, helpers, clause.block, &mut clause_aliases) {
                    merged.extend(clause_aliases);
                }
            }
            *aliases = merged;
            true
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            check_block(ctx, hir, helpers, *block, aliases)
        }
        StmtKind::AssemblyBlock(block) => check_block(ctx, hir, helpers, *block, aliases),
        StmtKind::Switch(switch) => {
            check_expr(ctx, hir, helpers, switch.selector, aliases);

            let baseline = aliases.clone();
            let mut merged = baseline.clone();
            for case in switch.cases {
                let mut case_aliases = baseline.clone();
                if check_block(ctx, hir, helpers, case.body, &mut case_aliases) {
                    merged.extend(case_aliases);
                }
            }
            *aliases = merged;
            true
        }
        StmtKind::Return(None) => false,
        StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => true,
    }
}

fn check_expr<'hir>(
    ctx: &LintContext,
    hir: &'hir Hir<'hir>,
    helpers: &HashSet<FunctionId>,
    expr: &'hir Expr<'hir>,
    aliases: &mut HashSet<VariableId>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, op, rhs) => {
            check_expr(ctx, hir, helpers, rhs, aliases);
            let rhs_is_timestamp = expr_value_is_timestamp_source(helpers, rhs, aliases);

            if op.is_some() {
                check_expr(ctx, hir, helpers, lhs, aliases);
                update_lhs_aliases(
                    hir,
                    lhs,
                    rhs_is_timestamp || expr_value_is_timestamp_source(helpers, lhs, aliases),
                    aliases,
                );
            } else {
                update_assignment_aliases(hir, helpers, lhs, rhs, aliases);
            }
        }
        ExprKind::Binary(lhs, op, rhs) => {
            if is_cmp(op.kind)
                && (expr_contains_timestamp_source(helpers, lhs, aliases)
                    || expr_contains_timestamp_source(helpers, rhs, aliases))
            {
                ctx.emit(&BLOCK_TIMESTAMP, expr.span);
            }

            check_expr(ctx, hir, helpers, lhs, aliases);
            check_expr(ctx, hir, helpers, rhs, aliases);
        }
        ExprKind::Call(callee, args, options) => {
            check_expr(ctx, hir, helpers, callee, aliases);
            if let Some(options) = options {
                for arg in options.args {
                    check_expr(ctx, hir, helpers, &arg.value, aliases);
                }
            }
            for arg in args.exprs() {
                check_expr(ctx, hir, helpers, arg, aliases);
            }
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner)
        | ExprKind::YulMember(inner, _) => check_expr(ctx, hir, helpers, inner, aliases),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            check_expr(ctx, hir, helpers, cond, aliases);

            let baseline = aliases.clone();
            let mut then_aliases = baseline.clone();
            check_expr(ctx, hir, helpers, then_expr, &mut then_aliases);
            let mut else_aliases = baseline;
            check_expr(ctx, hir, helpers, else_expr, &mut else_aliases);
            *aliases = then_aliases.union(&else_aliases).copied().collect();
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                check_expr(ctx, hir, helpers, expr, aliases);
            }
        }
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                check_expr(ctx, hir, helpers, expr, aliases);
            }
        }
        ExprKind::Index(base, index) => {
            check_expr(ctx, hir, helpers, base, aliases);
            if let Some(index) = index {
                check_expr(ctx, hir, helpers, index, aliases);
            }
        }
        ExprKind::Slice(base, start, end) => {
            check_expr(ctx, hir, helpers, base, aliases);
            if let Some(start) = start {
                check_expr(ctx, hir, helpers, start, aliases);
            }
            if let Some(end) = end {
                check_expr(ctx, hir, helpers, end, aliases);
            }
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => {}
    }
}

const fn is_cmp(kind: BinOpKind) -> bool {
    matches!(
        kind,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    )
}

fn update_multi_aliases(
    hir: &Hir<'_>,
    helpers: &HashSet<FunctionId>,
    vars: &[Option<VariableId>],
    expr: &Expr<'_>,
    aliases: &mut HashSet<VariableId>,
) {
    if let ExprKind::Tuple(exprs) = &expr.peel_parens().kind
        && exprs.len() == vars.len()
    {
        let rhs_aliases: Vec<_> = exprs
            .iter()
            .map(|expr| {
                expr.is_some_and(|expr| expr_value_is_timestamp_source(helpers, expr, aliases))
            })
            .collect();
        for (var_id, rhs_is_timestamp) in vars.iter().zip(rhs_aliases) {
            if let Some(var_id) = var_id {
                update_alias(hir, *var_id, rhs_is_timestamp, aliases);
            }
        }
        return;
    }

    let rhs_is_timestamp = expr_value_is_timestamp_source(helpers, expr, aliases);
    for var_id in vars.iter().flatten() {
        update_alias(hir, *var_id, rhs_is_timestamp, aliases);
    }
}

fn update_assignment_aliases(
    hir: &Hir<'_>,
    helpers: &HashSet<FunctionId>,
    lhs: &Expr<'_>,
    rhs: &Expr<'_>,
    aliases: &mut HashSet<VariableId>,
) {
    if let (ExprKind::Tuple(lhs_exprs), ExprKind::Tuple(rhs_exprs)) =
        (&lhs.peel_parens().kind, &rhs.peel_parens().kind)
        && lhs_exprs.len() == rhs_exprs.len()
    {
        let rhs_aliases: Vec<_> = rhs_exprs
            .iter()
            .map(|rhs| rhs.is_some_and(|rhs| expr_value_is_timestamp_source(helpers, rhs, aliases)))
            .collect();
        for (lhs, rhs_is_timestamp) in lhs_exprs.iter().zip(rhs_aliases) {
            if let Some(lhs) = lhs {
                update_lhs_aliases(hir, lhs, rhs_is_timestamp, aliases);
            }
        }
        return;
    }

    let rhs_is_timestamp = expr_value_is_timestamp_source(helpers, rhs, aliases);
    update_lhs_aliases(hir, lhs, rhs_is_timestamp, aliases);
}

fn update_lhs_aliases(
    hir: &Hir<'_>,
    lhs: &Expr<'_>,
    is_timestamp: bool,
    aliases: &mut HashSet<VariableId>,
) {
    match &lhs.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            for res in *resolutions {
                if let Res::Item(ItemId::Variable(var_id)) = res {
                    update_alias(hir, *var_id, is_timestamp, aliases);
                }
            }
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                update_lhs_aliases(hir, expr, is_timestamp, aliases);
            }
        }
        _ => {}
    }
}

fn update_alias(
    hir: &Hir<'_>,
    var_id: VariableId,
    is_timestamp: bool,
    aliases: &mut HashSet<VariableId>,
) {
    if !hir.variable(var_id).is_local_or_return() {
        return;
    }
    if is_timestamp {
        aliases.insert(var_id);
    } else {
        aliases.remove(&var_id);
    }
}

fn expr_contains_timestamp_source(
    helpers: &HashSet<FunctionId>,
    expr: &Expr<'_>,
    aliases: &HashSet<VariableId>,
) -> bool {
    if expr_value_is_timestamp_source(helpers, expr, aliases) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_contains_timestamp_source(helpers, lhs, aliases)
                || expr_contains_timestamp_source(helpers, rhs, aliases)
        }
        ExprKind::Call(callee, args, options) => {
            expr_contains_timestamp_source(helpers, callee, aliases)
                || options.is_some_and(|options| {
                    options
                        .args
                        .iter()
                        .any(|arg| expr_contains_timestamp_source(helpers, &arg.value, aliases))
                })
                || args.exprs().any(|arg| expr_contains_timestamp_source(helpers, arg, aliases))
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner)
        | ExprKind::YulMember(inner, _) => expr_contains_timestamp_source(helpers, inner, aliases),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            expr_contains_timestamp_source(helpers, cond, aliases)
                || expr_contains_timestamp_source(helpers, then_expr, aliases)
                || expr_contains_timestamp_source(helpers, else_expr, aliases)
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .flatten()
            .any(|expr| expr_contains_timestamp_source(helpers, expr, aliases)),
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_contains_timestamp_source(helpers, expr, aliases))
        }
        ExprKind::Index(base, index) => {
            expr_contains_timestamp_source(helpers, base, aliases)
                || index
                    .is_some_and(|index| expr_contains_timestamp_source(helpers, index, aliases))
        }
        ExprKind::Slice(base, start, end) => {
            expr_contains_timestamp_source(helpers, base, aliases)
                || start
                    .is_some_and(|start| expr_contains_timestamp_source(helpers, start, aliases))
                || end.is_some_and(|end| expr_contains_timestamp_source(helpers, end, aliases))
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}

fn expr_value_is_timestamp_source(
    helpers: &HashSet<FunctionId>,
    expr: &Expr<'_>,
    aliases: &HashSet<VariableId>,
) -> bool {
    if is_block_timestamp(expr) || is_timestamp_helper_call(helpers, expr) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => resolutions.iter().any(
            |res| matches!(res, Res::Item(ItemId::Variable(var_id)) if aliases.contains(var_id)),
        ),
        ExprKind::Binary(lhs, op, rhs) if !is_cmp(op.kind) => {
            expr_value_is_timestamp_source(helpers, lhs, aliases)
                || expr_value_is_timestamp_source(helpers, rhs, aliases)
        }
        ExprKind::Unary(_, inner) | ExprKind::Payable(inner) | ExprKind::YulMember(inner, _) => {
            expr_value_is_timestamp_source(helpers, inner, aliases)
        }
        ExprKind::Ternary(_, then_expr, else_expr) => {
            expr_value_is_timestamp_source(helpers, then_expr, aliases)
                || expr_value_is_timestamp_source(helpers, else_expr, aliases)
        }
        ExprKind::Tuple([Some(inner)]) => expr_value_is_timestamp_source(helpers, inner, aliases),
        _ => false,
    }
}

fn is_block_timestamp(expr: &Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Member(base, member) if member.name == kw::Timestamp => {
            let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
            reses
                .iter()
                .any(|res| matches!(res, Res::Builtin(builtin) if builtin.name() == sym::block))
        }
        ExprKind::Ident(reses) => {
            reses.iter().any(|res| matches!(res, Res::Builtin(Builtin::BlockTimestamp)))
        }
        _ => false,
    }
}

fn timestamp_helpers(hir: &Hir<'_>, contract: Option<ContractId>) -> HashSet<FunctionId> {
    let Some(contract) = contract else { return HashSet::new() };
    hir.contract_item_ids(contract)
        .filter_map(|item| item.as_function())
        .filter(|fid| {
            let helper = hir.function(*fid);
            helper.contract == Some(contract)
                && matches!(helper.visibility, ast::Visibility::Internal | ast::Visibility::Private)
                && helper.body.is_some_and(block_directly_returns_timestamp)
        })
        .collect()
}

fn is_timestamp_helper_call(helpers: &HashSet<FunctionId>, expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    helper_function_ids(callee).into_iter().any(|fid| helpers.contains(&fid))
}

fn is_revert_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return false };
    resolutions.iter().any(|res| matches!(res, Res::Builtin(Builtin::Revert | Builtin::RevertMsg)))
}

fn helper_function_ids(callee: &Expr<'_>) -> Vec<FunctionId> {
    let ExprKind::Ident(resolutions) = &callee.peel_parens().kind else { return Vec::new() };
    resolutions
        .iter()
        .filter_map(
            |res| {
                if let Res::Item(ItemId::Function(fid)) = res { Some(*fid) } else { None }
            },
        )
        .collect()
}

fn block_directly_returns_timestamp(block: Block<'_>) -> bool {
    block.stmts.iter().any(stmt_directly_returns_timestamp)
}

fn stmt_directly_returns_timestamp(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Return(Some(expr)) => expr_contains_direct_block_timestamp(expr),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block_directly_returns_timestamp(*block)
        }
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_directly_returns_timestamp(then_stmt)
                || else_stmt.is_some_and(stmt_directly_returns_timestamp)
        }
        _ => false,
    }
}

fn expr_contains_direct_block_timestamp(expr: &Expr<'_>) -> bool {
    if is_block_timestamp(expr) {
        return true;
    }

    match &expr.peel_parens().kind {
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_contains_direct_block_timestamp(lhs) || expr_contains_direct_block_timestamp(rhs)
        }
        ExprKind::Call(callee, args, options) => {
            expr_contains_direct_block_timestamp(callee)
                || options.is_some_and(|options| {
                    options.args.iter().any(|arg| expr_contains_direct_block_timestamp(&arg.value))
                })
                || args.exprs().any(expr_contains_direct_block_timestamp)
        }
        ExprKind::Unary(_, inner)
        | ExprKind::Delete(inner)
        | ExprKind::Member(inner, _)
        | ExprKind::Payable(inner)
        | ExprKind::YulMember(inner, _) => expr_contains_direct_block_timestamp(inner),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            expr_contains_direct_block_timestamp(cond)
                || expr_contains_direct_block_timestamp(then_expr)
                || expr_contains_direct_block_timestamp(else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_contains_direct_block_timestamp(expr))
        }
        ExprKind::Array(exprs) => exprs.iter().any(expr_contains_direct_block_timestamp),
        ExprKind::Index(base, index) => {
            expr_contains_direct_block_timestamp(base)
                || index.is_some_and(expr_contains_direct_block_timestamp)
        }
        ExprKind::Slice(base, start, end) => {
            expr_contains_direct_block_timestamp(base)
                || start.is_some_and(expr_contains_direct_block_timestamp)
                || end.is_some_and(expr_contains_direct_block_timestamp)
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}
