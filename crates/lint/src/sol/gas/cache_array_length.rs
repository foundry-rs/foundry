use super::CacheArrayLength;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::{Symbol, sym},
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

impl<'hir> LateLintPass<'hir> for CacheArrayLength {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let StmtKind::Loop(block, LoopSource::For) = &stmt.kind else { return };
        let Some(condition) = for_loop_condition(*block) else { return };

        emit_condition_length_reads(ctx, hir, condition);
    }
}

fn for_loop_condition<'hir>(block: hir::Block<'hir>) -> Option<&'hir hir::Expr<'hir>> {
    let first = block.stmts.first()?;
    match &first.kind {
        StmtKind::If(condition, _, Some(else_stmt)) => {
            matches!(&else_stmt.kind, StmtKind::Break).then_some(*condition)
        }
        _ => None,
    }
}

fn emit_condition_length_reads<'hir>(
    ctx: &LintContext,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) {
    match &expr.peel_parens().kind {
        ExprKind::Binary(lhs, op, rhs) if is_comparison(op.kind) => {
            emit_length_reads(ctx, hir, lhs);
            emit_length_reads(ctx, hir, rhs);
        }
        ExprKind::Binary(lhs, op, rhs) if matches!(op.kind, BinOpKind::And | BinOpKind::Or) => {
            emit_condition_length_reads(ctx, hir, lhs);
            emit_condition_length_reads(ctx, hir, rhs);
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Not => {
            emit_condition_length_reads(ctx, hir, inner);
        }
        _ => {}
    }
}

fn emit_length_reads<'hir>(
    ctx: &LintContext,
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) {
    let expr = expr.peel_parens();
    if is_array_length_member(hir, expr) {
        ctx.emit(&CACHE_ARRAY_LENGTH, expr.span);
        return;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            for expr in *exprs {
                emit_length_reads(ctx, hir, expr);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            emit_length_reads(ctx, hir, lhs);
            emit_length_reads(ctx, hir, rhs);
        }
        ExprKind::Call(callee, args, named_args) => {
            emit_length_reads(ctx, hir, callee);
            for arg in args.exprs() {
                emit_length_reads(ctx, hir, arg);
            }
            if let Some(named_args) = named_args {
                for arg in *named_args {
                    emit_length_reads(ctx, hir, &arg.value);
                }
            }
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            emit_length_reads(ctx, hir, inner);
        }
        ExprKind::Index(base, index) => {
            emit_length_reads(ctx, hir, base);
            if let Some(index) = index {
                emit_length_reads(ctx, hir, index);
            }
        }
        ExprKind::Slice(base, start, end) => {
            emit_length_reads(ctx, hir, base);
            if let Some(start) = start {
                emit_length_reads(ctx, hir, start);
            }
            if let Some(end) = end {
                emit_length_reads(ctx, hir, end);
            }
        }
        ExprKind::Member(base, _) => emit_length_reads(ctx, hir, base),
        ExprKind::Ternary(condition, then_expr, else_expr) => {
            emit_length_reads(ctx, hir, condition);
            emit_length_reads(ctx, hir, then_expr);
            emit_length_reads(ctx, hir, else_expr);
        }
        ExprKind::Tuple(exprs) => {
            for expr in exprs.iter().flatten() {
                emit_length_reads(ctx, hir, expr);
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

fn is_array_length_member(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Member(base, member) = &expr.kind else { return false };
    member.name == sym::length && is_array_like(hir, base)
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
        _ => None,
    }
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
