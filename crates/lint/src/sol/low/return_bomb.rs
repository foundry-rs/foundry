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
            StmtKind::DeclSingle(var) => {
                let var = hir.variable(var);
                var.initializer
                    .filter(|expr| {
                        call_with_gas_returns_dynamic_data(hir, expr)
                            && is_dynamic_type(hir, &var.ty)
                    })
                    .map(|_| stmt.span)
            }
            StmtKind::DeclMulti(vars, expr) => {
                if is_low_level_call_with_gas_limit(expr) && captures_return_data(hir, vars) {
                    Some(stmt.span)
                } else if call_with_gas_returns_dynamic_data(hir, expr)
                    && captures_dynamic_return_data(hir, vars)
                {
                    Some(stmt.span)
                } else {
                    None
                }
            }
            StmtKind::Expr(expr) => match expr.kind {
                ExprKind::Assign(lhs, _, rhs)
                    if is_low_level_call_with_gas_limit(rhs)
                        && assigns_return_data_receiver(hir, lhs) =>
                {
                    Some(expr.span)
                }
                ExprKind::Assign(lhs, _, rhs)
                    if call_with_gas_returns_dynamic_data(hir, rhs)
                        && assignment_captures_dynamic_return_data(hir, lhs) =>
                {
                    Some(expr.span)
                }
                _ => None,
            },
            StmtKind::Return(Some(expr)) if is_low_level_call_with_gas_limit(expr) => {
                Some(stmt.span)
            }
            StmtKind::Return(Some(expr)) if call_with_gas_returns_dynamic_data(hir, expr) => {
                Some(stmt.span)
            }
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
    elements.get(1).and_then(|element| *element).is_some_and(|expr| is_bytes_lvalue(hir, expr))
}

fn is_bytes_lvalue(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let expr = expr.peel_parens();
    match &expr.kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .filter_map(hir::Res::as_variable)
            .any(|var| is_dynamic_bytes_type(&hir.variable(var).ty.kind)),
        ExprKind::Member(base, member) => {
            field_type(hir, base, member.name).is_some_and(|ty| is_dynamic_bytes_type(&ty.kind))
        }
        ExprKind::Index(base, _) => {
            indexed_value_type(hir, base).is_some_and(|ty| is_dynamic_bytes_type(&ty.kind))
        }
        _ => false,
    }
}

const fn is_dynamic_bytes_type(ty: &TypeKind<'_>) -> bool {
    matches!(ty, TypeKind::Elementary(ElementaryType::Bytes))
}

fn field_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    base: &hir::Expr<'hir>,
    member: Symbol,
) -> Option<&'hir hir::Type<'hir>> {
    let base = base.peel_parens();
    let ExprKind::Ident(reses) = &base.kind else { return None };
    let var = reses.iter().filter_map(hir::Res::as_variable).next()?;
    let TypeKind::Custom(ItemId::Struct(struct_id)) = hir.variable(var).ty.kind else {
        return None;
    };
    hir.strukt(struct_id).fields.iter().find_map(|&field| {
        let field_var = hir.variable(field);
        (field_var.name?.name == member).then_some(&field_var.ty)
    })
}

fn indexed_value_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    base: &hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &base.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var = reses.iter().filter_map(hir::Res::as_variable).next()?;
            array_element_type(&hir.variable(var).ty)
        }
        ExprKind::Member(inner, member) => {
            let field = field_type(hir, inner, member.name)?;
            array_element_type(field)
        }
        ExprKind::Index(inner, _) => {
            let element = indexed_value_type(hir, inner)?;
            array_element_type(element)
        }
        _ => None,
    }
}

fn array_element_type<'hir>(ty: &'hir hir::Type<'hir>) -> Option<&'hir hir::Type<'hir>> {
    let TypeKind::Array(array) = &ty.kind else { return None };
    Some(&array.element)
}

fn captures_dynamic_return_data(hir: &hir::Hir<'_>, vars: &[Option<hir::VariableId>]) -> bool {
    vars.iter().flatten().any(|&var| is_dynamic_type(hir, &hir.variable(var).ty))
}

fn assignment_captures_dynamic_return_data(hir: &hir::Hir<'_>, lhs: &hir::Expr<'_>) -> bool {
    let ExprKind::Tuple(elements) = &lhs.peel_parens().kind else {
        return expr_has_dynamic_type(hir, lhs);
    };
    elements.iter().flatten().any(|expr| expr_has_dynamic_type(hir, expr))
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

fn expr_has_dynamic_type(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    expr_type(hir, expr).is_some_and(|ty| is_dynamic_type(hir, ty))
}

fn expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var = reses.iter().filter_map(hir::Res::as_variable).next()?;
            Some(&hir.variable(var).ty)
        }
        ExprKind::Member(base, member) => field_type(hir, base, member.name),
        ExprKind::Index(base, _) => indexed_value_type(hir, base),
        _ => None,
    }
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
