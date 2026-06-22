use super::UnusedReturn;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::interface::receiver_contract_id},
};
use solar::sema::{
    Gcx, Hir,
    hir::{Expr, ExprKind, Function, Stmt, StmtKind, TypeKind, VariableId},
};

declare_forge_lint!(
    UNUSED_RETURN,
    Severity::Med,
    "unused-return",
    "Return value of an external call is not used"
);

impl<'hir> LateLintPass<'hir> for UnusedReturn {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        stmt: &'hir Stmt<'hir>,
    ) {
        match &stmt.kind {
            StmtKind::Expr(expr)
                if is_unused_return_call(hir, expr) || is_ignored_tuple_assignment(hir, expr) =>
            {
                ctx.emit(&UNUSED_RETURN, expr.span);
            }
            StmtKind::DeclMulti(vars, expr)
                if vars.iter().any(Option::is_none) && is_unused_return_call(hir, expr) =>
            {
                ctx.emit(&UNUSED_RETURN, expr.span);
            }
            _ => {}
        }
    }
}

fn is_ignored_tuple_assignment(hir: &Hir<'_>, expr: &Expr<'_>) -> bool {
    let ExprKind::Assign(lhs, None, rhs) = &expr.peel_parens().kind else { return false };
    matches!(&lhs.peel_parens().kind, ExprKind::Tuple(elems) if elems.iter().any(Option::is_none))
        && is_unused_return_call(hir, rhs)
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

    let ExprKind::Call(callee, call_args, ..) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(contract_expr, func_ident) = &callee.peel_parens().kind else {
        return false;
    };

    // Arity from either positional or named args.
    let arity = call_args.kind.len();

    let Some(cid) = receiver_contract_id(hir, contract_expr) else { return false };

    let mut has_candidate = false;
    for item in hir.contract_item_ids(cid) {
        let Some(fid) = item.as_function() else { continue };
        let func = hir.function(fid);
        if func.name.is_none_or(|n| n.as_str() != func_ident.as_str())
            || !func.kind.is_function()
            || func.parameters.len() != arity
        {
            continue;
        }

        has_candidate = true;

        // If any matching overload returns nothing, we can't tell which overload is being called,
        // skip to avoid a false positive.
        if func.returns.is_empty() {
            return false;
        }

        // If any candidate is an ERC20 transfer/transferFrom, defer to erc20-unchecked-transfer.
        if is_erc20_transfer_sig(func, func_ident.as_str(), &is_type) {
            return false;
        }
    }

    has_candidate
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
