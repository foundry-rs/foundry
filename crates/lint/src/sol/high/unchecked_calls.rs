use super::{UncheckedCall, UncheckedTransferERC20};
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{Expr, ExprKind, ItemFunction, Stmt, StmtKind, visit::Visit};
use solar_interface::kw;
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

/// WARN: can issue false positives. It does not check that the contract being called is an ERC20.
impl<'ast> EarlyLintPass<'ast> for UncheckedTransferERC20 {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = UncheckedTransferERC20Checker { ctx };
            let _ = checker.visit_block(body);
        }
    }
}

/// Visitor that detects unchecked ERC20 transfer calls within function bodies.
///
/// Unchecked transfers appear as standalone expression statements.
/// When a transfer's return value is used (in require, assignment, etc.), it's part
/// of a larger expression and won't be flagged.
struct UncheckedTransferERC20Checker<'a, 's> {
    ctx: &'a LintContext<'s>,
}

impl<'ast> Visit<'ast> for UncheckedTransferERC20Checker<'_, '_> {
    type BreakValue = ();

    fn visit_stmt(&mut self, stmt: &'ast Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        // Only expression statements can contain unchecked transfers.
        if let StmtKind::Expr(expr) = &stmt.kind
            && is_erc20_transfer_call(expr)
        {
            self.ctx.emit(&ERC20_UNCHECKED_TRANSFER, expr.span);
        }
        self.walk_stmt(stmt)
    }
}

/// Checks if an expression is an ERC20 `transfer` or `transferFrom` call.
/// `function ERC20.transfer(to, amount)`
/// `function ERC20.transferFrom(from, to, amount)`
///
/// Validates both the method name and argument count to avoid false positives
/// from other functions that happen to be named "transfer".
fn is_erc20_transfer_call(expr: &Expr<'_>) -> bool {
    if let ExprKind::Call(call_expr, args) = &expr.kind {
        // Must be a member access pattern: `token.transfer(...)`
        if let ExprKind::Member(_, member) = &call_expr.kind {
            return (args.len() == 2 && member.as_str() == "transfer")
                || (args.len() == 3 && member.as_str() == "transferFrom");
        }
    }
    false
}

// -- UNCKECKED LOW-LEVEL CALLS -------------------------------------------------------------------

impl<'ast> EarlyLintPass<'ast> for UncheckedCall {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
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
    ctx: &'a LintContext<'s>,
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
