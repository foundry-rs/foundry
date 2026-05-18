use super::CacheArrayLength;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{Symbol, kw, sym},
    sema::hir::{
        self, BinOpKind, ElementaryType, ExprKind, ItemId, LoopSource, Res, StmtKind, TypeKind,
        UnOpKind,
    },
};

declare_forge_lint!(
    CACHE_ARRAY_LENGTH,
    Severity::Gas,
    "cache-array-length",
    "array length read in loop condition should be cached outside the loop"
);

#[derive(Clone, Copy)]
struct LengthRead<'hir> {
    expr: &'hir hir::Expr<'hir>,
}

impl<'hir> LateLintPass<'hir> for CacheArrayLength {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let StmtKind::Loop(block, LoopSource::For) = &stmt.kind else { return };
        let Some((condition, body)) = for_loop_parts(*block) else { return };

        let mut reads = Vec::new();
        collect_condition_length_reads(hir, condition, &mut reads);
        if reads.is_empty() || stmt_mutates_array_length(hir, body) {
            return;
        }
        for read in reads {
            ctx.emit(&CACHE_ARRAY_LENGTH, read.expr.span);
        }
    }
}

fn for_loop_parts<'hir>(
    block: hir::Block<'hir>,
) -> Option<(&'hir hir::Expr<'hir>, &'hir hir::Stmt<'hir>)> {
    let first = block.stmts.first()?;
    match &first.kind {
        StmtKind::If(condition, _, Some(else_stmt)) => {
            matches!(&else_stmt.kind, StmtKind::Break).then_some((*condition, first))
        }
        _ => None,
    }
}

fn collect_condition_length_reads<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) if is_comparison(op.kind) => {
            collect_length_reads(hir, lhs, reads);
            collect_length_reads(hir, rhs, reads);
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            collect_condition_length_reads(hir, lhs, reads);
            collect_condition_length_reads(hir, rhs, reads);
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            collect_condition_length_reads(hir, inner, reads);
        }
        _ => {}
    }
}

fn collect_length_reads<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
    reads: &mut Vec<LengthRead<'hir>>,
) {
    let expr = expr.peel_parens();
    if let ExprKind::Member(base, member) = &expr.kind
        && member.name == sym::length
        && is_array_like(hir, base)
    {
        reads.push(LengthRead { expr });
        return;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                collect_length_reads(hir, expr, reads);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            collect_length_reads(hir, lhs, reads);
            collect_length_reads(hir, rhs, reads);
        }
        ExprKind::Call(callee, args, named_args) => {
            collect_length_reads(hir, callee, reads);
            for arg in args.exprs() {
                collect_length_reads(hir, arg, reads);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    collect_length_reads(hir, &arg.value, reads);
                }
            }
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            collect_length_reads(hir, inner, reads);
        }
        ExprKind::Index(base, index) => {
            collect_length_reads(hir, base, reads);
            if let Some(index) = index {
                collect_length_reads(hir, index, reads);
            }
        }
        ExprKind::Slice(base, start, end) => {
            collect_length_reads(hir, base, reads);
            if let Some(start) = start {
                collect_length_reads(hir, start, reads);
            }
            if let Some(end) = end {
                collect_length_reads(hir, end, reads);
            }
        }
        ExprKind::Member(base, _) => collect_length_reads(hir, base, reads),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            collect_length_reads(hir, condition, reads);
            collect_length_reads(hir, then_expr, reads);
            collect_length_reads(hir, else_expr, reads);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                collect_length_reads(hir, expr, reads);
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

fn stmt_mutates_array_length<'hir>(hir: &'hir hir::Hir<'hir>, stmt: &'hir hir::Stmt<'hir>) -> bool {
    match &stmt.kind {
        StmtKind::DeclSingle(var_id) => hir
            .variable(*var_id)
            .initializer
            .is_some_and(|expr| expr_mutates_array_length(hir, expr)),
        StmtKind::DeclMulti(_, expr)
        | StmtKind::Emit(expr)
        | StmtKind::Revert(expr)
        | StmtKind::Expr(expr) => expr_mutates_array_length(hir, expr),
        StmtKind::Return(expr) => expr.is_some_and(|expr| expr_mutates_array_length(hir, expr)),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) | StmtKind::Loop(block, _) => {
            block.stmts.iter().any(|stmt| stmt_mutates_array_length(hir, stmt))
        }
        StmtKind::If(condition, then_stmt, else_stmt) => {
            expr_mutates_array_length(hir, condition)
                || stmt_mutates_array_length(hir, then_stmt)
                || else_stmt.is_some_and(|stmt| stmt_mutates_array_length(hir, stmt))
        }
        StmtKind::Try(stmt_try) => {
            expr_mutates_array_length(hir, &stmt_try.expr)
                || stmt_try.clauses.iter().any(|clause| {
                    clause.block.stmts.iter().any(|stmt| stmt_mutates_array_length(hir, stmt))
                })
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Placeholder | StmtKind::Err(_) => false,
    }
}

fn expr_mutates_array_length<'hir>(hir: &'hir hir::Hir<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    let expr = expr.peel_parens();
    if array_length_mutated(hir, expr) {
        return true;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => exprs.iter().any(|expr| expr_mutates_array_length(hir, expr)),
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_mutates_array_length(hir, lhs) || expr_mutates_array_length(hir, rhs)
        }
        ExprKind::Call(callee, args, named_args) => {
            expr_mutates_array_length(hir, callee)
                || args.exprs().any(|arg| expr_mutates_array_length(hir, arg))
                || named_args.is_some_and(|named_args| {
                    named_args.iter().any(|arg| expr_mutates_array_length(hir, &arg.value))
                })
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            expr_mutates_array_length(hir, inner)
        }
        ExprKind::Index(base, index) => {
            expr_mutates_array_length(hir, base)
                || index.is_some_and(|index| expr_mutates_array_length(hir, index))
        }
        ExprKind::Slice(base, start, end) => {
            expr_mutates_array_length(hir, base)
                || start.is_some_and(|start| expr_mutates_array_length(hir, start))
                || end.is_some_and(|end| expr_mutates_array_length(hir, end))
        }
        ExprKind::Member(base, _) => expr_mutates_array_length(hir, base),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            expr_mutates_array_length(hir, condition)
                || expr_mutates_array_length(hir, then_expr)
                || expr_mutates_array_length(hir, else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_mutates_array_length(hir, expr))
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}

fn array_length_mutated<'hir>(hir: &'hir hir::Hir<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, _, _) | ExprKind::Delete(lhs) => is_array_like(hir, lhs),
        ExprKind::Call(callee, _, _) => {
            let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return false };
            matches!(member.name, sym::push | kw::Pop) && is_array_like(hir, base)
        }
        _ => false,
    }
}

const fn is_comparison(op: BinOpKind) -> bool {
    matches!(
        op,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    )
}

fn is_array_like(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let Some(ty) = expr_type(hir, expr) else { return false };
    match &ty.kind {
        TypeKind::Array(array) => array.size.is_none(),
        TypeKind::Elementary(ElementaryType::Bytes) => true,
        _ => false,
    }
}

fn expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            let var_id = resolutions.iter().find_map(|res| {
                if let Res::Item(ItemId::Variable(var_id)) = res { Some(*var_id) } else { None }
            })?;
            Some(&hir.variable(var_id).ty)
        }
        ExprKind::Index(base, _) => match &expr_type(hir, base)?.kind {
            TypeKind::Array(array) => Some(&array.element),
            TypeKind::Mapping(mapping) => Some(&mapping.value),
            _ => None,
        },
        ExprKind::Member(base, member) => {
            struct_field_type(hir, expr_type(hir, base)?, member.name)
        }
        ExprKind::Call(callee, _, _) => call_return_type(hir, callee),
        _ => None,
    }
}

fn call_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &'hir hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &callee.peel_parens().kind {
        ExprKind::Type(ty) => Some(ty),
        ExprKind::Ident(resolutions) => {
            let function_id = resolutions.iter().find_map(|res| {
                if let Res::Item(ItemId::Function(function_id)) = res {
                    Some(*function_id)
                } else {
                    None
                }
            })?;
            function_return_type(hir, function_id)
        }
        _ => match &expr_type(hir, callee)?.kind {
            TypeKind::Function(function) => {
                let [return_id] = function.returns else { return None };
                Some(&hir.variable(*return_id).ty)
            }
            _ => None,
        },
    }
}

fn function_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    function_id: hir::FunctionId,
) -> Option<&'hir hir::Type<'hir>> {
    let [return_id] = hir.function(function_id).returns else { return None };
    Some(&hir.variable(*return_id).ty)
}

fn struct_field_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    ty: &'hir hir::Type<'hir>,
    member: Symbol,
) -> Option<&'hir hir::Type<'hir>> {
    let TypeKind::Custom(ItemId::Struct(struct_id)) = &ty.kind else { return None };
    hir.strukt(*struct_id)
        .fields
        .iter()
        .map(|&field_id| hir.variable(field_id))
        .find(|field| field.name.is_some_and(|name| name.name == member))
        .map(|field| &field.ty)
}
