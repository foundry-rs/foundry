use super::UnsafeTypecast;
use crate::{
    linter::{LateLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, LitKind, StrKind},
    sema::{
        Gcx,
        eval::ConstantEvaluator,
        hir::{self, ElementaryType, ExprKind, Res, StmtKind, TypeKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    UNSAFE_TYPECAST,
    Severity::Med,
    "unsafe-typecast",
    "typecasts that can truncate values should be checked"
);

impl<'hir> LateLintPass<'hir> for UnsafeTypecast {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // Check for type cast expressions: Type(value)
        if let ExprKind::Call(call, args, _) = &expr.kind
            && let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call.kind
            && args.len() == 1
            && let Some(call_arg) = args.exprs().next()
            && is_unsafe_typecast_hir(gcx, call_arg, ty)
            && !is_bounded_by_dominating_check(gcx, _hir, expr, call_arg, ty)
        {
            ctx.emit_with_suggestion(
                &UNSAFE_TYPECAST,
                expr.span,
                Suggestion::example(
                    format!(
                        "// casting to '{abi_ty}' is safe because [explain why]\n// forge-lint: disable-next-line(unsafe-typecast)",
                        abi_ty = ty.to_abi_str()
            )).with_desc("consider disabling this lint if you're certain the cast is safe"));
        }
    }
}

/// Returns whether every path to the cast is constrained to the target unsigned integer range.
///
/// This is a small forward dataflow analysis over Solar's structured HIR. A conditional whose
/// out-of-range edge terminates contributes an upper-bound fact to the surviving edge. Assigning
/// to the tracked variable invalidates that fact.
fn is_bounded_by_dominating_check<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    cast: &'hir hir::Expr<'hir>,
    source: &'hir hir::Expr<'hir>,
    target: &ElementaryType,
) -> bool {
    let ElementaryType::UInt(size) = target else { return false };
    let Some(var) = resolved_variable(source) else { return false };
    let max = if size.bits() == 256 {
        alloy_primitives::U256::MAX
    } else {
        (alloy_primitives::U256::from(1) << size.bits()) - alloy_primitives::U256::from(1)
    };

    hir.all_functions().any(|id| {
        hir.function(id).body.is_some_and(|body| {
            body.span.contains(cast.span) && block_proves_bound(gcx, body, cast, var, max, false)
        })
    })
}

fn block_proves_bound<'hir>(
    gcx: Gcx<'hir>,
    block: hir::Block<'hir>,
    cast: &'hir hir::Expr<'hir>,
    var: hir::VariableId,
    max: alloy_primitives::U256,
    mut bounded: bool,
) -> bool {
    for stmt in block.stmts {
        if stmt.span.contains(cast.span) {
            return stmt_proves_bound(gcx, stmt, cast, var, max, bounded);
        }

        if let StmtKind::If(cond, then_stmt, else_stmt) = stmt.kind {
            let then_terminates = stmt_always_terminates(then_stmt);
            let else_terminates = else_stmt.is_some_and(stmt_always_terminates);
            if then_terminates && else_stmt.is_none() && false_edge_bounds(gcx, cond, var, max) {
                bounded = true;
            } else if else_terminates && true_edge_bounds(gcx, cond, var, max) {
                bounded = true;
            }
        }

        if stmt_assigns_var(stmt, var) {
            bounded = false;
        }
    }
    false
}

fn stmt_proves_bound<'hir>(
    gcx: Gcx<'hir>,
    stmt: &'hir hir::Stmt<'hir>,
    cast: &'hir hir::Expr<'hir>,
    var: hir::VariableId,
    max: alloy_primitives::U256,
    bounded: bool,
) -> bool {
    match stmt.kind {
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block_proves_bound(gcx, block, cast, var, max, bounded)
        }
        StmtKind::If(cond, then_stmt, else_stmt) => {
            if then_stmt.span.contains(cast.span) {
                stmt_proves_bound(
                    gcx,
                    then_stmt,
                    cast,
                    var,
                    max,
                    bounded || true_edge_bounds(gcx, cond, var, max),
                )
            } else if let Some(else_stmt) = else_stmt
                && else_stmt.span.contains(cast.span)
            {
                stmt_proves_bound(
                    gcx,
                    else_stmt,
                    cast,
                    var,
                    max,
                    bounded || false_edge_bounds(gcx, cond, var, max),
                )
            } else {
                false
            }
        }
        StmtKind::Loop(..)
        | StmtKind::AssemblyBlock(..)
        | StmtKind::Switch(..)
        | StmtKind::Try(..) => false,
        _ => bounded,
    }
}

fn true_edge_bounds(
    gcx: Gcx<'_>,
    cond: &hir::Expr<'_>,
    var: hir::VariableId,
    max: alloy_primitives::U256,
) -> bool {
    comparison_bounds(gcx, cond, var, max, true)
}

fn false_edge_bounds(
    gcx: Gcx<'_>,
    cond: &hir::Expr<'_>,
    var: hir::VariableId,
    max: alloy_primitives::U256,
) -> bool {
    comparison_bounds(gcx, cond, var, max, false)
}

fn comparison_bounds(
    gcx: Gcx<'_>,
    cond: &hir::Expr<'_>,
    var: hir::VariableId,
    max: alloy_primitives::U256,
    truthy: bool,
) -> bool {
    let ExprKind::Binary(lhs, op, rhs) = &cond.peel_parens().kind else { return false };

    let (value, limit, op) = if resolved_variable(lhs) == Some(var) {
        (*lhs, *rhs, op.kind)
    } else if resolved_variable(rhs) == Some(var) {
        let reversed = match op.kind {
            BinOpKind::Lt => BinOpKind::Gt,
            BinOpKind::Le => BinOpKind::Ge,
            BinOpKind::Gt => BinOpKind::Lt,
            BinOpKind::Ge => BinOpKind::Le,
            _ => return false,
        };
        (*rhs, *lhs, reversed)
    } else {
        return false;
    };
    debug_assert_eq!(resolved_variable(value), Some(var));

    let Ok(limit) = ConstantEvaluator::new(gcx).try_eval(limit) else { return false };
    let Some(limit) = limit.as_u256() else { return false };
    match (op, truthy) {
        (BinOpKind::Le, true) | (BinOpKind::Gt, false) => limit <= max,
        (BinOpKind::Lt, true) | (BinOpKind::Ge, false) => {
            limit.checked_sub(alloy_primitives::U256::from(1)).is_some_and(|limit| limit <= max)
        }
        _ => false,
    }
}

fn resolved_variable(expr: &hir::Expr<'_>) -> Option<hir::VariableId> {
    let ExprKind::Ident([Res::Item(hir::ItemId::Variable(id))]) = expr.peel_parens().kind else {
        return None;
    };
    Some(id)
}

fn stmt_always_terminates(stmt: &hir::Stmt<'_>) -> bool {
    match stmt.kind {
        StmtKind::Revert(_) | StmtKind::Return(_) => true,
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.last().is_some_and(stmt_always_terminates)
        }
        StmtKind::If(_, then_stmt, Some(else_stmt)) => {
            stmt_always_terminates(then_stmt) && stmt_always_terminates(else_stmt)
        }
        _ => false,
    }
}

fn stmt_assigns_var(stmt: &hir::Stmt<'_>, var: hir::VariableId) -> bool {
    match stmt.kind {
        StmtKind::Expr(expr) => expr_assigns_var(expr, var),
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            block.stmts.iter().any(|stmt| stmt_assigns_var(stmt, var))
        }
        StmtKind::Loop(block, _) => block.stmts.iter().any(|stmt| stmt_assigns_var(stmt, var)),
        // Inline assembly and try/switch control flow are conservatively treated as clobbers.
        StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Try(_) => true,
        StmtKind::If(_, then_stmt, else_stmt) => {
            stmt_assigns_var(then_stmt, var)
                || else_stmt.is_some_and(|stmt| stmt_assigns_var(stmt, var))
        }
        _ => false,
    }
}

fn expr_assigns_var(expr: &hir::Expr<'_>, var: hir::VariableId) -> bool {
    matches!(
        expr.peel_parens().kind,
        ExprKind::Assign(lhs, _, _) if resolved_variable(lhs) == Some(var)
    )
}

/// Determines if a typecast is potentially unsafe (could lose data or precision).
fn is_unsafe_typecast_hir<'hir>(
    gcx: Gcx<'hir>,
    source_expr: &hir::Expr<'hir>,
    target_type: &hir::ElementaryType,
) -> bool {
    if is_bounded_by_mask(source_expr, target_type) {
        return false;
    }

    let mut source_types = Vec::<ElementaryType>::new();
    infer_source_types(Some(&mut source_types), gcx, source_expr);

    if source_types.is_empty() {
        return false;
    };

    source_types.iter().any(|source_ty| is_unsafe_elementary_typecast(source_ty, target_type))
}

/// Returns whether a bitmask bounds an unsigned integer expression to the target type's range.
fn is_bounded_by_mask(source_expr: &hir::Expr<'_>, target_type: &ElementaryType) -> bool {
    let ElementaryType::UInt(target_size) = target_type else { return false };
    let ExprKind::Binary(lhs, op, rhs) = &source_expr.peel_parens().kind else { return false };
    if op.kind != BinOpKind::BitAnd {
        return false;
    }

    [lhs, rhs].into_iter().any(|expr| {
        matches!(
            expr.peel_parens().kind,
            ExprKind::Lit(hir::Lit { kind: LitKind::Number(mask), .. })
                if mask.bit_len() <= target_size.bits() as usize
        )
    })
}

/// Infers the elementary source type(s) of an expression.
///
/// This function traverses an expression tree to find the original "source" types.
/// For cast chains, it returns the ultimate source type, not intermediate cast results.
/// For binary operations, it collects types from both sides into the `output` vector.
///
/// # Returns
/// An `Option<ElementaryType>` containing the inferred type of the expression if it can be
/// resolved to a single source (like variables, literals, or unary expressions).
/// Returns `None` for expressions complex expressions (like binary operations).
fn infer_source_types<'hir>(
    mut output: Option<&mut Vec<ElementaryType>>,
    gcx: Gcx<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<ElementaryType> {
    let mut track = |ty: ElementaryType| -> Option<ElementaryType> {
        if let Some(output) = output.as_mut() {
            output.push(ty);
        }
        Some(ty)
    };

    match &expr.kind {
        // A type cast call: `Type(val)`
        ExprKind::Call(call_expr, args, ..) => {
            // Check if the called expression is a type, which indicates a cast.
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(..), .. }) =
                &call_expr.kind
                && let Some(inner) = args.exprs().next()
            {
                // Recurse to find the original (inner-most) source type.
                return infer_source_types(output, gcx, inner);
            }
            expr_elementary_type(gcx, expr).and_then(track)
        }

        // Handle string literals explicitly; Solar records them as literal types rather than
        // elementary `string`/`bytes`.
        ExprKind::Lit(hir::Lit { kind, .. }) => match kind {
            LitKind::Str(StrKind::Hex, ..) => track(ElementaryType::Bytes),
            LitKind::Str(..) => track(ElementaryType::String),
            _ => expr_elementary_type(gcx, expr).and_then(track),
        },

        // Identifiers and other simple typed expressions.
        ExprKind::Ident(_) => expr_elementary_type(gcx, expr).and_then(track),

        // Unary operations: Recurse to find the source type of the inner expression.
        ExprKind::Unary(_, inner_expr) => infer_source_types(output, gcx, inner_expr),

        // Binary operations
        ExprKind::Binary(lhs, _, rhs) => {
            if let Some(mut output) = output {
                // Recurse on both sides to find and collect all source types.
                infer_source_types(Some(&mut output), gcx, lhs);
                infer_source_types(Some(&mut output), gcx, rhs);
            }
            None
        }

        _ => expr_elementary_type(gcx, expr).and_then(track),
    }
}

fn expr_elementary_type<'hir>(gcx: Gcx<'hir>, expr: &hir::Expr<'hir>) -> Option<ElementaryType> {
    match gcx.type_of_expr(expr.peel_parens().id)?.peel_refs().kind {
        TyKind::Elementary(ty) => Some(ty),
        TyKind::StringLiteral(true, _) => Some(ElementaryType::String),
        TyKind::StringLiteral(false, _) => Some(ElementaryType::Bytes),
        _ => None,
    }
}

/// Checks if a type cast from source_type to target_type is unsafe.
const fn is_unsafe_elementary_typecast(
    source_type: &ElementaryType,
    target_type: &ElementaryType,
) -> bool {
    match (source_type, target_type) {
        // Numeric downcasts (smaller target size)
        (ElementaryType::UInt(source_size), ElementaryType::UInt(target_size))
        | (ElementaryType::Int(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() > target_size.bits()
        }

        // Signed to unsigned conversion (potential loss of sign)
        (ElementaryType::Int(_), ElementaryType::UInt(_)) => true,

        // Unsigned to signed conversion with same or smaller size
        (ElementaryType::UInt(source_size), ElementaryType::Int(target_size)) => {
            source_size.bits() >= target_size.bits()
        }

        // Fixed bytes to smaller fixed bytes
        (ElementaryType::FixedBytes(source_size), ElementaryType::FixedBytes(target_size)) => {
            source_size.bytes() > target_size.bytes()
        }

        // Dynamic bytes to fixed bytes (potential truncation)
        (ElementaryType::Bytes | ElementaryType::String, ElementaryType::FixedBytes(_)) => true,

        // Address to smaller uint (truncation) - address is 160 bits
        (ElementaryType::Address(_), ElementaryType::UInt(target_size)) => target_size.bits() < 160,

        // Address to int (sign issues)
        (ElementaryType::Address(_), ElementaryType::Int(_)) => true,

        _ => false,
    }
}
