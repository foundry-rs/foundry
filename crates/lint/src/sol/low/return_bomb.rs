use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, calls::is_call_with_gas_limit},
};
use solar::{
    ast::{ElementaryType, LitKind, StrKind},
    interface::{Symbol, kw, sym},
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
        // Flag gas-limited calls that can force the caller to copy unbounded returndata.
        if low_level_call_with_gas_consumes_unbounded_return_data(hir, expr)
            || call_with_gas_returns_dynamic_data(hir, expr)
        {
            ctx.emit(&RETURN_BOMB, expr.span);
        }
    }
}

/// Returns true for gas-limited calls that return dynamic data.
fn call_with_gas_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    is_call_with_gas_limit(expr) && call_returns_dynamic_data(hir, expr)
}
/// Returns true if a call resolves to functions that return dynamic data.
fn call_returns_dynamic_data(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    matching_functions_for_callee(hir, callee, args)
        .is_some_and(|functions| functions_return_dynamic_data(hir, &functions))
}

/// Returns true for gas-limited low-level calls that copy unbounded returndata.
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
/// Returns the function overloads that can match a callee and its arguments.
fn matching_functions_for_callee<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &hir::Expr<'hir>,
    args: &CallArgs<'hir>,
) -> Option<FunctionCandidates<'hir>> {
    match &callee.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let candidates = reses.iter().filter_map(res_function_id);
            let functions = select_functions_for_args(hir, candidates, args);
            (!functions.is_empty()).then_some(functions).or_else(|| {
                expr_type(hir, callee)
                    .and_then(|ty| function_type_returns_for_args(hir, ty, args))
                    .map(FunctionCandidates::Pointer)
            })
        }
        ExprKind::Member(base, member) => {
            let contract = expr_contract_id(hir, base)?;
            let candidates = function_candidates_for_member(hir, contract, member.name);
            Some(select_functions_for_args(hir, candidates, args))
        }
        _ => None,
    }
}

/// Returns function candidates with the given member name on a contract.
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

/// Selects the candidate functions that best match the call arguments.
fn select_functions_for_args<'hir>(
    hir: &'hir hir::Hir<'hir>,
    candidates: impl Iterator<Item = hir::FunctionId>,
    args: &CallArgs<'hir>,
) -> FunctionCandidates<'hir> {
    let mut exact = Vec::new();
    let mut maybe = Vec::new();

    for id in candidates {
        match function_arg_match(hir, id, args) {
            ArgMatch::Exact => exact.push(id),
            ArgMatch::Maybe => maybe.push(id),
            ArgMatch::No => {}
        }
    }

    if exact.is_empty() {
        FunctionCandidates::Maybe(maybe)
    } else {
        FunctionCandidates::Exact(exact)
    }
}
/// Returns true if a gas-limited call can return dynamic data.
fn functions_return_dynamic_data(hir: &hir::Hir<'_>, functions: &FunctionCandidates<'_>) -> bool {
    match functions {
        FunctionCandidates::Exact(functions) => match functions.as_slice() {
            [] => false,
            [function] => function_returns_dynamic_data(hir, *function),
            [first, rest @ ..] => {
                let first = function_returns_dynamic_data(hir, *first);
                rest.iter().all(|&function| function_returns_dynamic_data(hir, function) == first)
                    && first
            }
        },
        FunctionCandidates::Maybe(functions) => {
            functions.iter().any(|&function| function_returns_dynamic_data(hir, function))
        }
        FunctionCandidates::Pointer(returns) => returns_dynamic_data(hir, returns),
    }
}

enum FunctionCandidates<'hir> {
    Exact(Vec<hir::FunctionId>),
    Maybe(Vec<hir::FunctionId>),
    Pointer(&'hir [hir::VariableId]),
}

impl FunctionCandidates<'_> {
    const fn is_empty(&self) -> bool {
        match self {
            Self::Exact(functions) | Self::Maybe(functions) => functions.is_empty(),
            Self::Pointer(returns) => returns.is_empty(),
        }
    }
}

/// Returns true if any return variable has a dynamically encoded return type.
fn returns_dynamic_data(hir: &hir::Hir<'_>, returns: &[hir::VariableId]) -> bool {
    returns.iter().any(|&var| is_dynamic_type(hir, &hir.variable(var).ty))
}

const fn res_function_id(res: &hir::Res) -> Option<hir::FunctionId> {
    match res {
        hir::Res::Item(ItemId::Function(id)) => Some(*id),
        _ => None,
    }
}

/// Returns true if a function has any dynamically encoded return value.
fn function_returns_dynamic_data(hir: &hir::Hir<'_>, function: hir::FunctionId) -> bool {
    returns_dynamic_data(hir, hir.function(function).returns)
}

enum ArgMatch {
    Exact,
    Maybe,
    No,
}
/// Returns how well call arguments match a candidate function signature.
fn function_arg_match(hir: &hir::Hir<'_>, id: hir::FunctionId, args: &CallArgs<'_>) -> ArgMatch {
    let function = hir.function(id);
    params_arg_match(hir, function.parameters, args)
}

/// Returns how well call arguments match the expected parameter types.
fn params_arg_match(
    hir: &hir::Hir<'_>,
    parameters: &[hir::VariableId],
    args: &CallArgs<'_>,
) -> ArgMatch {
    if args.len() != parameters.len() {
        return ArgMatch::No;
    }

    match &args.kind {
        CallArgsKind::Unnamed(exprs) => {
            params_match_args(hir, parameters.iter().copied().zip(exprs.iter()))
        }
        CallArgsKind::Named(named_args) => {
            let mut maybe = false;
            for named_arg in *named_args {
                let Some(param) = parameters.iter().copied().find(|&param| {
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
/// Returns how well positional arguments match their expected parameter types.
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
/// Returns the contract id for expressions known to be contract-typed.
fn expr_contract_id(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) if is_this(reses) => enclosing_contract(hir, expr),
        ExprKind::Call(callee, _, _) => {
            expr_type(hir, expr).and_then(type_contract_id).or_else(|| {
                match &callee.peel_parens().kind {
                    ExprKind::Ident(reses) => reses.iter().find_map(|res| match res {
                        hir::Res::Item(ItemId::Contract(id)) => Some(*id),
                        _ => None,
                    }),
                    ExprKind::Type(hir::Type {
                        kind: TypeKind::Custom(ItemId::Contract(id)),
                        ..
                    }) => Some(*id),
                    _ => None,
                }
            })
        }
        _ => expr_type(hir, expr).and_then(type_contract_id),
    }
}

/// Returns the contract containing an expression span.
fn enclosing_contract(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> Option<hir::ContractId> {
    hir.function_ids().find_map(|id| {
        let function = hir.function(id);
        (function.body.is_some() && function.body_span.contains(expr.span))
            .then_some(function.contract?)
    })
}

fn is_this(reses: &[hir::Res]) -> bool {
    reses.iter().any(|res| matches!(res, hir::Res::Builtin(builtin) if builtin.name() == sym::this))
}

/// Returns the contract id for contract-typed values.
const fn type_contract_id(ty: &hir::Type<'_>) -> Option<hir::ContractId> {
    let TypeKind::Custom(ItemId::Contract(id)) = ty.kind else { return None };
    Some(id)
}
/// Returns true if an expression is known to have an address type.
fn expr_is_address(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => matches!(lit.kind, LitKind::Address(_)),
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) => {
            matches!(
                &callee.peel_parens().kind,
                ExprKind::Type(hir::Type {
                    kind: TypeKind::Elementary(ElementaryType::Address(_)),
                    ..
                })
            ) || expr_type(hir, expr).is_some_and(is_address_type)
        }
        ExprKind::Member(base, member) if member_is_builtin_address(base, member.name) => true,
        _ => expr_type(hir, expr).is_some_and(is_address_type),
    }
}

fn member_is_builtin_address(base: &hir::Expr<'_>, member: Symbol) -> bool {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let hir::Res::Builtin(builtin) = res else { return false };
        matches!(
            (builtin.name(), member),
            (sym::msg, sym::sender) | (sym::block, kw::Coinbase) | (sym::tx, kw::Origin)
        )
    })
}

/// Returns whether an expression can match the expected type, if known.
fn expr_matches_type(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>, ty: &hir::Type<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => return Some(lit_matches_type(lit, ty)),
        ExprKind::Payable(_) => return Some(is_address_type(ty)),
        _ => {}
    }

    expr_type(hir, expr).map(|expr_ty| types_match(hir, expr_ty, ty))
}
/// Returns the type of simple expressions needed by this lint.
fn expr_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &hir::Expr<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let var = reses.iter().filter_map(hir::Res::as_variable).next()?;
            Some(&hir.variable(var).ty)
        }
        ExprKind::Index(base, _) => match &expr_type(hir, base)?.kind {
            TypeKind::Array(array) => Some(&array.element),
            TypeKind::Mapping(mapping) => Some(&mapping.value),
            _ => None,
        },
        ExprKind::Member(base, member) => {
            let TypeKind::Custom(ItemId::Struct(id)) = expr_type(hir, base)?.kind else {
                return None;
            };
            hir.strukt(id).fields.iter().find_map(|&field| {
                (hir.variable(field).name?.name == member.name).then_some(&hir.variable(field).ty)
            })
        }
        ExprKind::Call(callee, args, _) => match &callee.peel_parens().kind {
            ExprKind::Type(ty) => Some(ty),
            _ => call_return_type(hir, callee, args),
        },
        _ => None,
    }
}

/// Returns the single return type for a statically resolved call expression.
fn call_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    callee: &hir::Expr<'hir>,
    args: &CallArgs<'hir>,
) -> Option<&'hir hir::Type<'hir>> {
    let functions = match matching_functions_for_callee(hir, callee, args)? {
        FunctionCandidates::Exact(functions) | FunctionCandidates::Maybe(functions) => functions,
        FunctionCandidates::Pointer(_) => return None,
    };
    let function = single_function(functions.into_iter())?;
    single_return_type(hir, hir.function(function).returns)
}

/// Returns exactly one function id from an iterator.
fn single_function(
    mut functions: impl Iterator<Item = hir::FunctionId>,
) -> Option<hir::FunctionId> {
    let function = functions.next()?;
    functions.next().is_none().then_some(function)
}

/// Returns exactly one return type.
fn single_return_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    returns: &[hir::VariableId],
) -> Option<&'hir hir::Type<'hir>> {
    let &[ret] = returns else { return None };
    Some(&hir.variable(ret).ty)
}

/// Returns external function-type return variables when the arguments can match.
fn function_type_returns_for_args<'hir>(
    hir: &hir::Hir<'_>,
    ty: &'hir hir::Type<'hir>,
    args: &CallArgs<'_>,
) -> Option<&'hir [hir::VariableId]> {
    let TypeKind::Function(function) = ty.kind else { return None };
    if function.visibility != hir::Visibility::External {
        return None;
    }
    if matches!(params_arg_match(hir, function.parameters, args), ArgMatch::No) {
        return None;
    }
    Some(function.returns)
}

/// Returns true if the type is an address type
const fn is_address_type(ty: &hir::Type<'_>) -> bool {
    matches!(ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
}
/// Returns true if a literal can be used for a value of the given type.
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

/// Returns true if two types are equivalent for overload resolution.
fn types_match(hir: &hir::Hir<'_>, a: &hir::Type<'_>, b: &hir::Type<'_>) -> bool {
    match (&a.kind, &b.kind) {
        (
            TypeKind::Elementary(ElementaryType::Address(_)),
            TypeKind::Elementary(ElementaryType::Address(_)),
        ) => true,
        (TypeKind::Elementary(a), TypeKind::Elementary(b)) => elementary_type_matches(*a, *b),
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

/// Returns true if an elementary value can be used for an expected elementary type.
fn elementary_type_matches(actual: ElementaryType, expected: ElementaryType) -> bool {
    use ElementaryType::{Address, Bool, Bytes, Fixed, FixedBytes, Int, String, UFixed, UInt};

    match (actual, expected) {
        (Address(_), Address(false)) => true,
        (Address(payable), Address(true)) => payable,
        (UInt(actual), UInt(expected))
        | (Int(actual), Int(expected))
        | (FixedBytes(actual), FixedBytes(expected)) => actual.bits() <= expected.bits(),
        (Bool, Bool) | (String, String) | (Bytes, Bytes) => true,
        (Fixed(actual_size, actual_scale), Fixed(expected_size, expected_scale))
        | (UFixed(actual_size, actual_scale), UFixed(expected_size, expected_scale)) => {
            actual_size == expected_size && actual_scale == expected_scale
        }
        _ => false,
    }
}

/// Returns true if the type contains dynamically encoded ABI data.
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
