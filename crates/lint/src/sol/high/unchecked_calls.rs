use super::{UncheckedCall, UncheckedTransferERC20};
use crate::{
    linter::{EarlyLintPass, LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Expr, ExprKind, ItemFunction, Stmt, StmtKind, visit::Visit},
    interface::kw,
    sema::hir::{self},
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNCHECKED_CALL,
    Severity::High,
    "unchecked-call",
    "Low-level calls should check the success return value"
);

declare_forge_lint!(
    ERC20_UNCHECKED_TRANSFER,
    Severity::High,
    "erc20-unchecked-transfer",
    "ERC20 'transfer' and 'transferFrom' calls should check the return value"
);

// -- ERC20 UNCKECKED TRANSFERS -------------------------------------------------------------------

/// Checks that calls to functions with the same signature as the ERC20 transfer methods, and which
/// return a boolean are not ignored.
///
/// WARN: can issue false positives, as it doesn't check that the contract being called sticks to
/// the full ERC20 specification.
impl<'hir> LateLintPass<'hir> for UncheckedTransferERC20 {
    fn check_stmt(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        stmt: &'hir hir::Stmt<'hir>,
    ) {
        // Only expression statements can contain unchecked transfers.
        if let hir::StmtKind::Expr(expr) = &stmt.kind
            && is_erc20_transfer_call(hir, expr)
        {
            ctx.emit(&ERC20_UNCHECKED_TRANSFER, expr.span);
        }
    }
}

/// Checks if an expression is an ERC20 `transfer` or `transferFrom` call.
/// * `function transfer(address to, uint256 amount) external returns bool;`
/// * `function transferFrom(address from, address to, uint256 amount) external returns bool;`
///
/// Validates the method name, the params (count + types), and the returns (count + types).
fn is_erc20_transfer_call(hir: &hir::Hir<'_>, expr: &hir::Expr<'_>) -> bool {
    let is_type = |var_id: hir::VariableId, type_str: &str| {
        matches!(
            &hir.variable(var_id).ty.kind,
            hir::TypeKind::Elementary(ty) if ty.to_abi_str() == type_str
        )
    };

    // Ensure the expression is a call to a contract member function.
    let hir::ExprKind::Call(
        hir::Expr { kind: hir::ExprKind::Member(contract_expr, func_ident), .. },
        hir::CallArgs { kind: hir::CallArgsKind::Unnamed(args), .. },
        ..,
    ) = &expr.kind
    else {
        return false;
    };

    // Determine the expected ERC20 signature from the call
    let (expected_params, expected_returns): (&[&str], &[&str]) = match func_ident.as_str() {
        "transferFrom" if args.len() == 3 => (&["address", "address", "uint256"], &["bool"]),
        "transfer" if args.len() == 2 => (&["address", "uint256"], &["bool"]),
        _ => return false,
    };

    let Some(cid) = (match &contract_expr.kind {
        // Call to pre-instantiated contract variable
        hir::ExprKind::Ident([hir::Res::Item(hir::ItemId::Variable(id)), ..]) => {
            if let hir::TypeKind::Custom(hir::ItemId::Contract(cid)) = hir.variable(*id).ty.kind {
                Some(cid)
            } else {
                None
            }
        }
        // Call to address wrapped by the contract interface
        hir::ExprKind::Call(
            hir::Expr {
                kind: hir::ExprKind::Ident([hir::Res::Item(hir::ItemId::Contract(cid))]),
                ..
            },
            ..,
        ) => Some(*cid),
        _ => None,
    }) else {
        return false;
    };

    // Try to find a function in the contract that matches the expected signature.
    hir.contract_item_ids(cid).any(|item| {
        let Some(fid) = item.as_function() else { return false };
        let func = hir.function(fid);
        func.name.is_some_and(|name| name.as_str() == func_ident.as_str())
            && func.kind.is_function()
            && func.mutates_state()
            && func.parameters.len() == expected_params.len()
            && func.returns.len() == expected_returns.len()
            && func.parameters.iter().zip(expected_params).all(|(id, &ty)| is_type(*id, ty))
            && func.returns.iter().zip(expected_returns).all(|(id, &ty)| is_type(*id, ty))
    })
}

// -- UNCKECKED LOW-LEVEL CALLS -------------------------------------------------------------------

impl<'ast> EarlyLintPass<'ast> for UncheckedCall {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = UncheckedCallChecker { ctx };
            let _ = checker.visit_block(body);
        }
    }
}

/// Visitor that detects unchecked low-level calls within function bodies.
///
/// Similar to unchecked transfers, unchecked calls appear as standalone expression
/// statements. When the success value is checked (in require, if, etc.), the call
/// is part of a larger expression and won't be flagged.
struct UncheckedCallChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for UncheckedCallChecker<'_, '_> {
    type BreakValue = ();

    fn visit_stmt(&mut self, stmt: &'ast Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        match &stmt.kind {
            // Check standalone expression statements: `target.call(data);`
            StmtKind::Expr(expr) => {
                if is_low_level_call(expr) {
                    self.ctx.emit(&UNCHECKED_CALL, expr.span);
                } else if let ExprKind::Assign(lhs, _, rhs) = &expr.kind {
                    // Check assignments to existing vars: `(, existingVar) = target.call(data);`
                    if is_low_level_call(rhs) && is_unchecked_tuple_assignment(lhs) {
                        self.ctx.emit(&UNCHECKED_CALL, expr.span);
                    }
                }
            }
            // Check multi-variable declarations: `(bool success, ) = target.call(data);`
            StmtKind::DeclMulti(vars, expr) => {
                if is_low_level_call(expr) && vars.first().is_none_or(|v| v.is_none()) {
                    self.ctx.emit(&UNCHECKED_CALL, stmt.span);
                }
            }
            _ => {}
        }
        self.walk_stmt(stmt)
    }
}

/// Checks if an expression is a low-level call that should be checked.
///
/// Detects patterns like:
/// - `target.call(...)`
/// - `target.delegatecall(...)`
/// - `target.staticcall(...)`
/// - `target.call{value: x}(...)`
fn is_low_level_call(expr: &Expr<'_>) -> bool {
    if let ExprKind::Call(call_expr, _args) = &expr.kind {
        // Check the callee expression
        let callee = match &call_expr.kind {
            // Handle call options like {value: x}
            ExprKind::CallOptions(inner_expr, _) => inner_expr,
            // Direct call without options
            _ => call_expr,
        };

        if let ExprKind::Member(_, member) = &callee.kind {
            // Check for low-level call methods
            return matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall);
        }
    }
    false
}

/// Checks if a tuple assignment doesn't properly check the success value.
///
/// Returns true if the first variable (success) is None: `(, bytes memory data) =
/// target.call(...)`
fn is_unchecked_tuple_assignment(expr: &Expr<'_>) -> bool {
    if let ExprKind::Tuple(elements) = &expr.kind {
        elements.first().is_none_or(|e| e.is_none())
    } else {
        false
    }
}
