use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, calls::is_call_with_gas_limit},
};
use solar::{
    ast::{ElementaryType, LitKind, StrKind},
    interface::{Symbol, kw},
    sema::hir::{self, CallArgs, CallArgsKind, ExprKind, ItemId, TypeKind},
};

declare_forge_lint!(
    RETURN_BOMB,
    Severity::Low,
    "return-bomb",
    "external calls with a gas limit should not consume unbounded return data"
);

impl<'hir> LateLintPass<'hir> for ReturnBomb {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        if low_level_call_with_gas_consumes_unbounded_return_data(hir, expr)
            || call_with_gas_returns_dynamic_data(hir, expr)
        {
            ctx.emit(&RETURN_BOMB, expr.span);
        }
    }
}

fn call_with_gas_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    is_call_with_gas_limit(expr) && call_returns_dynamic_data(hir, expr)
}

fn call_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    matching_functions_for_callee(hir, callee, args)
        .is_some_and(|functions| functions_return_dynamic_data(hir, &functions))
}

fn low_level_call_with_gas_consumes_unbounded_return_data(
    hir: &hir::Hir<'_>,
    expr: &hir::Expr<'_>,
) -> bool {
    if !is_call_with_gas_limit(expr) {
        return false;
    }

    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return false };
    matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
        && expr_is_address(hir, receiver)
}

fn matching_functions_for_callee<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &hir::Expr<'hir>,
    args: &CallArgs<'hir>,
) -> Option<Vec<hir::FunctionId>> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let candidates = reses.iter().filter_map(|res| match res {
                hir::Res::Item(ItemId::Function(id)) => Some(*id),
                _ => None,
            });
            Some(select_functions_for_args(hir, candidates, args))
        }
        ExprKind::Member(base, member) => {
            let contract = expr_contract_id(hir, base)?;
            let candidates = function_candidates_for_member(hir, contract, member.name);
            Some(select_functions_for_args(hir, candidates, args))
        }
        _ => None,
    }
}

fn function_candidates_for_member<'hir>(
    hir: &'hir hir::Hir<'hir>,
    contract: hir::ContractId,
    member: Symbol,
) -> impl Iterator<Item = hir::FunctionId> + Clone + 'hir {
    hir.contract_item_ids(contract).filter_map(move |item| {
        let ItemId::Function(id) = item else { return None };
        let function = hir.function(id);
        (function.name?.name == member).then_some(id)
    })
}

fn select_functions_for_args<'hir>(
    hir: &'hir hir::Hir<'hir>,
    candidates: impl Iterator<Item = hir::FunctionId>,
    args: &CallArgs<'hir>,
) -> Vec<hir::FunctionId> {
    let mut exact = Vec::new();
    let mut maybe = Vec::new();

    for id in candidates {
        match function_arg_match(hir, id, args) {
            ArgMatch::Exact => exact.push(id),
            ArgMatch::Maybe => maybe.push(id),
            ArgMatch::No => {}
        }
    }

    if exact.is_empty() { maybe } else { exact }
}

fn functions_return_dynamic_data(hir: &hir::Hir<'_>, functions: &[hir::FunctionId]) -> bool {
    match functions {
        [] => false,
        &[function] => function_returns_dynamic_data(hir, function),
        [first, rest @ ..] => {
            let first = function_returns_dynamic_data(hir, *first);
            rest.iter().all(|&function| function_returns_dynamic_data(hir, function) == first)
                && first
        }
    }
}

fn function_returns_dynamic_data(hir: &hir::Hir<'_>, function: hir::FunctionId) -> bool {
    hir.function(function).returns.iter().any(|&var| is_dynamic_type(hir, &hir.variable(var).ty))
}

enum ArgMatch {
    Exact,
    Maybe,
    No,
}

fn function_arg_match(hir: &hir::Hir<'_>, id: hir::FunctionId, args: &CallArgs<'_>) -> ArgMatch {
    let function = hir.function(id);
    if args.len() != function.parameters.len() {
        return ArgMatch::No;
    }

    match &args.kind {
        CallArgsKind::Unnamed(exprs) => {
            params_match_args(hir, function.parameters.iter().copied().zip(exprs.iter()))
        }
        CallArgsKind::Named(named_args) => {
            let mut maybe = false;
            for named_arg in *named_args {
                let Some(param) = function.parameters.iter().copied().find(|&param| {
                    hir.variable(param).name.is_some_and(|name| name.name == named_arg.name.name)
                }) else {
                    return ArgMatch::No;
                };
                match expr_matches_type(hir, &named_arg.value, &hir.variable(param).ty) {
                    Some(true) => {}
                    Some(false) => return ArgMatch::No,
                    None => maybe = true,
                }
            }
            if maybe { ArgMatch::Maybe } else { ArgMatch::Exact }
        }
    }
}

fn params_match_args<'hir>(
    hir: &hir::Hir<'hir>,
    params_and_args: impl Iterator<Item = (hir::VariableId, &'hir hir::Expr<'hir>)>,
) -> ArgMatch {
    let mut maybe = false;
    for (param, arg) in params_and_args {
        match expr_matches_type(hir, arg, &hir.variable(param).ty) {
            Some(true) => {}
            Some(false) => return ArgMatch::No,
            None => maybe = true,
        }
    }
    if maybe { ArgMatch::Maybe } else { ArgMatch::Exact }
}

fn expr_contract_id(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => reses
            .iter()
            .find_map(hir::Res::as_variable)
            .and_then(|var| type_contract_id(&hir.variable(var).ty)),
        ExprKind::Call(callee, _, _) => match &callee.peel_parens().kind {
            ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
                hir::Res::Item(ItemId::Contract(id)) => Some(*id),
                _ => None,
            }),
            ExprKind::Type(hir::Type { kind: TypeKind::Custom(ItemId::Contract(id)), .. }) => {
                Some(*id)
            }
            _ => None,
        },
        _ => None,
    }
}

const fn type_contract_id(ty: &hir::Type<'_>) -> Option<hir::ContractId> {
    let TypeKind::Custom(ItemId::Contract(id)) = ty.kind else { return None };
    Some(id)
}

fn expr_is_address(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => matches!(lit.kind, LitKind::Address(_)),
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) => matches!(
            &callee.peel_parens().kind,
            ExprKind::Type(hir::Type {
                kind: TypeKind::Elementary(ElementaryType::Address(_)),
                ..
            })
        ),
        _ => expr_type(hir, expr).is_some_and(is_address_type),
    }
}

fn expr_matches_type(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>, ty: &hir::Type<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => return Some(lit_matches_type(lit, ty)),
        ExprKind::Payable(_) => return Some(is_address_type(ty)),
        _ => {}
    }

    expr_type(hir, expr).map(|expr_ty| types_match(hir, expr_ty, ty))
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
        ExprKind::Index(base, _) => {
            let TypeKind::Array(array) = &expr_type(hir, base)?.kind else { return None };
            Some(&array.element)
        }
        ExprKind::Member(base, member) => {
            let TypeKind::Custom(ItemId::Struct(id)) = expr_type(hir, base)?.kind else {
                return None;
            };
            hir.strukt(id).fields.iter().find_map(|&field| {
                (hir.variable(field).name?.name == member.name).then_some(&hir.variable(field).ty)
            })
        }
        ExprKind::Call(callee, _, _) => match &callee.peel_parens().kind {
            ExprKind::Type(ty) => Some(ty),
            _ => None,
        },
        _ => None,
    }
}

const fn is_address_type(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}

const fn lit_matches_type(lit: &solar::ast::Lit<'_>, ty: &hir::Type<'_>) -> bool {
    matches!(
        (&lit.kind, &ty.kind),
        (LitKind::Address(_), TypeKind::Elementary(ElementaryType::Address(_)))
            | (LitKind::Bool(_), TypeKind::Elementary(ElementaryType::Bool))
            | (
                LitKind::Number(_) | LitKind::Rational(_),
                TypeKind::Elementary(
                    ElementaryType::Int(_)
                        | ElementaryType::UInt(_)
                        | ElementaryType::Fixed(_, _)
                        | ElementaryType::UFixed(_, _),
                ),
            )
            | (
                LitKind::Str(StrKind::Str | StrKind::Unicode, ..),
                TypeKind::Elementary(ElementaryType::String),
            )
            | (
                LitKind::Str(StrKind::Hex, ..),
                TypeKind::Elementary(ElementaryType::Bytes | ElementaryType::FixedBytes(_)),
            )
    )
}

fn types_match(hir: &hir::Hir<'_>, a: &hir::Type<'_>, b: &hir::Type<'_>) -> bool {
    match (&a.kind, &b.kind) {
        (
            TypeKind::Elementary(ElementaryType::Address(_)),
            TypeKind::Elementary(ElementaryType::Address(_)),
        ) => true,
        (TypeKind::Elementary(a), TypeKind::Elementary(b)) => a == b,
        (TypeKind::Array(a), TypeKind::Array(b)) => {
            a.size.is_some() == b.size.is_some() && types_match(hir, &a.element, &b.element)
        }
        (TypeKind::Custom(a), TypeKind::Custom(b)) => a == b,
        (TypeKind::Function(a), TypeKind::Function(b)) => {
            a.parameters.len() == b.parameters.len()
                && a.returns.len() == b.returns.len()
                && a.parameters
                    .iter()
                    .zip(b.parameters)
                    .all(|(&a, &b)| types_match(hir, &hir.variable(a).ty, &hir.variable(b).ty))
                && a.returns
                    .iter()
                    .zip(b.returns)
                    .all(|(&a, &b)| types_match(hir, &hir.variable(a).ty, &hir.variable(b).ty))
        }
        _ => false,
    }
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
