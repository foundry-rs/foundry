use super::UnusedReturn;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Hir,
    hir::{Expr, ExprKind, Function, ItemId, Res, Stmt, StmtKind, TypeKind, VariableId},
};

declare_forge_lint!(
    UNUSED_RETURN,
    Severity::Med,
    "unused-return",
    "Return value of an external call is not used"
);

impl<'hir> LateLintPass<'hir> for UnusedReturn {
    fn check_stmt(&mut self, ctx: &LintContext, hir: &'hir Hir<'hir>, stmt: &'hir Stmt<'hir>) {
        if let StmtKind::Expr(expr) = &stmt.kind
            && is_unused_return_call(hir, expr)
        {
            ctx.emit(&UNUSED_RETURN, expr.span);
        }
    }
}

/// Returns true if `expr` is a member call on a contract whose resolved function has return
/// values, excluding ERC20 `transfer`/`transferFrom` (covered by `erc20-unchecked-transfer`).
fn is_unused_return_call(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let is_type = |var_id: VariableId, type_str: &str| {
        matches!(
            &hir.variable(var_id).ty.kind,
            TypeKind::Elementary(ty) if ty.to_abi_str() == type_str
        )
    };

    let ExprKind::Call(callee, call_args, ..) = &expr.kind else { return false };
    let ExprKind::Member(contract_expr, func_ident) = &callee.kind else { return false };

    // Arity from either positional or named args.
    let arity = call_args.kind.len();

    let Some(cid) = (match &contract_expr.kind {
        // Pre-instantiated contract variable: `oracle.f()`
        ExprKind::Ident([Res::Item(ItemId::Variable(id)), ..]) => {
            if let TypeKind::Custom(ItemId::Contract(cid)) = hir.variable(*id).ty.kind {
                Some(cid)
            } else {
                None
            }
        }
        // Explicit interface cast: `IOracle(addr).f()`
        ExprKind::Call(
            Expr { kind: ExprKind::Ident([Res::Item(ItemId::Contract(cid))]), .. },
            ..,
        ) => Some(*cid),
        _ => None,
    }) else {
        return false;
    };

    // Collect all functions in the contract matching this name and arity.
    let candidates: Vec<&Function<'_>> = hir
        .contract_item_ids(cid)
        .filter_map(|item| {
            let fid = item.as_function()?;
            let func = hir.function(fid);
            (func.name.is_some_and(|n| n.as_str() == func_ident.as_str())
                && func.kind.is_function()
                && func.parameters.len() == arity)
                .then_some(func)
        })
        .collect();

    // No matching candidate found, nothing to lint.
    if candidates.is_empty() {
        return false;
    }

    // If any candidate returns nothing, we can't tell which overload is being called,
    // skip to avoid a false positive.
    if candidates.iter().any(|f| f.returns.is_empty()) {
        return false;
    }

    // If any candidate is an ERC20 transfer/transferFrom, defer to erc20-unchecked-transfer.
    if candidates.iter().any(|f| is_erc20_transfer_sig(f, func_ident.as_str(), &is_type)) {
        return false;
    }

    true
}

/// Returns true if `func` matches the ERC20 `transfer` or `transferFrom` signature exactly.
/// These are handled by `erc20-unchecked-transfer` and must not be double-reported.
fn is_erc20_transfer_sig(
    func: &Function<'_>,
    name: &str,
    is_type: &impl Fn(VariableId, &str) -> bool,
) -> bool {
    match name {
        "transfer" if func.parameters.len() == 2 && func.returns.len() == 1 => {
            is_type(func.parameters[0], "address")
                && is_type(func.parameters[1], "uint256")
                && is_type(func.returns[0], "bool")
        }
        "transferFrom" if func.parameters.len() == 3 && func.returns.len() == 1 => {
            is_type(func.parameters[0], "address")
                && is_type(func.parameters[1], "address")
                && is_type(func.parameters[2], "uint256")
                && is_type(func.returns[0], "bool")
        }
        _ => false,
    }
}
