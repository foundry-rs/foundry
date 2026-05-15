use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        calls::{is_call_with_gas_limit, is_low_level_call_with_gas_limit},
    },
};
use solar::{
    ast::ElementaryType,
    interface::Symbol,
    sema::hir::{self, ExprKind, ItemId, StmtKind, TypeKind},
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
            StmtKind::DeclSingle(var) => hir
                .variable(var)
                .initializer
                .filter(|expr| expr_consumes_unbounded_return_data(hir, expr))
                .map(|_| stmt.span),
            StmtKind::DeclMulti(_, expr) => {
                if expr_consumes_unbounded_return_data(hir, expr) {
                    Some(stmt.span)
                } else {
                    None
                }
            }
            StmtKind::Expr(expr) if expr_consumes_unbounded_return_data(hir, expr) => {
                Some(expr.span)
            }
            StmtKind::Return(Some(expr)) if expr_consumes_unbounded_return_data(hir, expr) => {
                Some(stmt.span)
            }
            _ => None,
        };

        if let Some(span) = span {
            ctx.emit(&RETURN_BOMB, span);
        }
    }
}

fn expr_consumes_unbounded_return_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let expr = expr.peel_parens();

    if is_low_level_call_with_gas_limit(expr) || call_with_gas_returns_dynamic_data(hir, expr) {
        return true;
    }

    match &expr.kind {
        ExprKind::Array(exprs) => {
            exprs.iter().any(|expr| expr_consumes_unbounded_return_data(hir, expr))
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            expr_consumes_unbounded_return_data(hir, lhs)
                || expr_consumes_unbounded_return_data(hir, rhs)
        }
        ExprKind::Call(callee, args, opts) => {
            expr_consumes_unbounded_return_data(hir, callee)
                || args.exprs().any(|expr| expr_consumes_unbounded_return_data(hir, expr))
                || opts.is_some_and(|opts| {
                    opts.iter().any(|opt| expr_consumes_unbounded_return_data(hir, &opt.value))
                })
        }
        ExprKind::Delete(inner) | ExprKind::Payable(inner) | ExprKind::Unary(_, inner) => {
            expr_consumes_unbounded_return_data(hir, inner)
        }
        ExprKind::Index(base, index) => {
            expr_consumes_unbounded_return_data(hir, base)
                || index.is_some_and(|index| expr_consumes_unbounded_return_data(hir, index))
        }
        ExprKind::Slice(base, start, end) => {
            expr_consumes_unbounded_return_data(hir, base)
                || start.is_some_and(|start| expr_consumes_unbounded_return_data(hir, start))
                || end.is_some_and(|end| expr_consumes_unbounded_return_data(hir, end))
        }
        ExprKind::Member(base, _) => expr_consumes_unbounded_return_data(hir, base),
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            expr_consumes_unbounded_return_data(hir, cond)
                || expr_consumes_unbounded_return_data(hir, then_expr)
                || expr_consumes_unbounded_return_data(hir, else_expr)
        }
        ExprKind::Tuple(exprs) => {
            exprs.iter().flatten().any(|expr| expr_consumes_unbounded_return_data(hir, expr))
        }
        ExprKind::Ident(_)
        | ExprKind::Lit(_)
        | ExprKind::New(_)
        | ExprKind::TypeCall(_)
        | ExprKind::Type(_)
        | ExprKind::Err(_) => false,
    }
}

fn call_with_gas_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    is_call_with_gas_limit(expr) && call_returns_dynamic_data(hir, expr)
}

fn call_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    callee_return_vars(hir, callee).is_some_and(|returns| {
        returns.iter().any(|&var| is_dynamic_type(hir, &hir.variable(var).ty))
    })
}

fn callee_return_vars<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &hir::Expr<'hir>,
) -> Option<&'hir [hir::VariableId]> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .filter_map(|res| match res {
                hir::Res::Item(ItemId::Function(id)) => Some(hir.function(*id).returns),
                _ => None,
            })
            .next(),
        ExprKind::Member(base, member) => {
            let contract = expr_contract_id(hir, base)?;
            function_returns_for_member(hir, contract, member.name)
        }
        _ => None,
    }
}

fn function_returns_for_member<'hir>(
    hir: &'hir hir::Hir<'hir>,
    contract: hir::ContractId,
    member: Symbol,
) -> Option<&'hir [hir::VariableId]> {
    hir.contract_item_ids(contract).find_map(|item| {
        let ItemId::Function(id) = item else { return None };
        let function = hir.function(id);
        (function.name?.name == member).then_some(function.returns)
    })
}

fn expr_contract_id(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var = reses.iter().filter_map(hir::Res::as_variable).next()?;
            type_contract_id(&hir.variable(var).ty)
        }
        ExprKind::Call(callee, _, _) => match &callee.peel_parens().kind {
            ExprKind::Type(hir::Type { kind: TypeKind::Custom(ItemId::Contract(id)), .. }) => {
                Some(*id)
            }
            _ => None,
        },
        _ => None,
    }
}

fn type_contract_id(ty: &hir::Type<'_>) -> Option<hir::ContractId> {
    let TypeKind::Custom(ItemId::Contract(id)) = ty.kind else { return None };
    Some(id)
}

fn is_dynamic_type(hir: &hir::Hir<'_>, ty: &hir::Type<'_>) -> bool {
    match &ty.kind {
        TypeKind::Elementary(ElementaryType::Bytes | ElementaryType::String) => true,
        TypeKind::Array(array) => array.size.is_none() || is_dynamic_type(hir, &array.element),
        TypeKind::Custom(ItemId::Struct(id)) => hir
            .strukt(*id)
            .fields
            .iter()
            .any(|&field| is_dynamic_type(hir, &hir.variable(field).ty)),
        _ => false,
    }
}
