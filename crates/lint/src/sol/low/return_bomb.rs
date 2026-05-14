use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, calls::is_low_level_call_with_gas_limit},
};
use solar::{
    ast::ElementaryType,
    sema::hir::{self, ExprKind, StmtKind, TypeKind},
};

declare_forge_lint!(
    RETURN_BOMB,
    Severity::Low,
    "return-bomb",
    "external calls with a gas limit should not consume unbounded return data"
);

impl<'hir> LateLintPass<'hir> for ReturnBomb {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        let span = match stmt.kind {
            StmtKind::DeclMulti(vars, expr) => (is_low_level_call_with_gas_limit(expr)
                && captures_return_data(hir, vars))
            .then_some(stmt.span),
            StmtKind::Expr(expr) => match expr.kind {
                ExprKind::Assign(lhs, _, rhs)
                    if is_low_level_call_with_gas_limit(rhs)
                        && assigns_return_data_receiver(hir, lhs) =>
                {
                    Some(expr.span)
                }
                _ => None,
            },
            _ => None,
        };

        if let Some(span) = span {
            ctx.emit(&RETURN_BOMB, span);
        }
    }
}

fn captures_return_data(hir: &hir::Hir<'_>, vars: &[Option<hir::VariableId>]) -> bool {
    vars.get(1)
        .and_then(|var| *var)
        .is_some_and(|var| is_dynamic_bytes_type(&hir.variable(var).ty.kind))
}

fn assigns_return_data_receiver(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> bool {
    let ExprKind::Tuple(elements) = &lhs.peel_parens().kind else { return false };
    elements.get(1).and_then(|element| *element).is_some_and(|expr| is_bytes_variable(hir, expr))
}

fn is_bytes_variable(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &expr.peel_parens().kind else { return false };
    reses
        .iter()
        .filter_map(hir::Res::as_variable)
        .any(|var| is_dynamic_bytes_type(&hir.variable(var).ty.kind))
}

const fn is_dynamic_bytes_type(ty: &TypeKind<'_>) -> bool {
    matches!(ty, TypeKind::Elementary(ElementaryType::Bytes))
}
