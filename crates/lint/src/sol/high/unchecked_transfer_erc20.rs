use super::UncheckedTransferERC20;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{visit::Visit, Expr, ExprKind, ItemFunction, Stmt, StmtKind};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNCHECKED_TRANSFER_ERC20,
    Severity::High,
    "erc20-unchecked-transfer",
    "ERC20 'transfer' and 'transferFrom' calls should check the return value"
);

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
        if let StmtKind::Expr(expr) = &stmt.kind {
            if is_transfer_call(expr) {
                self.ctx.emit(&UNCHECKED_TRANSFER_ERC20, expr.span);
            }
        }
        self.walk_stmt(stmt)
    }
}

/// Checks if an expression is an ERC20 `transfer` or `transferFrom` call.
///
/// Validates both the method name and argument count to avoid false positives
/// from other functions that happen to be named "transfer".
fn is_transfer_call(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Call(call_expr, args) => {
            // Must be a member access pattern: `token.transfer(...)`
            if let ExprKind::Member(_, member) = &call_expr.kind {
                let method_name = member.as_str();
                // function ERC20.transfer(to, amount)
                // function ERC20.transferFrom(from, to, amount)
                (args.len() == 2 && method_name == "transfer") ||
                    (args.len() == 3 && method_name == "transferFrom")
            } else {
                false
            }
        }
        _ => false,
    }
}
