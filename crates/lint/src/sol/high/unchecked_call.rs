use super::UncheckedCall;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{visit::Visit, Expr, ExprKind, ItemFunction, Stmt, StmtKind, VariableDefinition};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNCHECKED_CALL,
    Severity::High,
    "unchecked-call",
    "Low-level calls should check the success return value"
);

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
                    // Check assignments to existing variables: `(, existingVar) =
                    // target.call(data);`
                    if is_low_level_call(rhs) && is_unchecked_tuple_assignment(lhs) {
                        self.ctx.emit(&UNCHECKED_CALL, expr.span);
                    }
                }
            }
            // Check multi-variable declarations: `(bool success, ) = target.call(data);`
            StmtKind::DeclMulti(vars, expr) => {
                if is_low_level_call(expr) && is_unchecked_multi_declaration(vars) {
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
    match &expr.kind {
        ExprKind::Call(call_expr, _args) => {
            // Check the callee expression
            let callee = match &call_expr.kind {
                // Handle call options like {value: x}
                ExprKind::CallOptions(inner_expr, _) => inner_expr,
                // Direct call without options
                _ => call_expr,
            };

            // Must be a member access pattern: `target.call(...)`
            if let ExprKind::Member(_, member) = &callee.kind {
                let method_name = member.as_str();
                // Check for low-level call methods
                matches!(method_name, "call" | "delegatecall" | "staticcall")
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Checks if a multi-variable declaration doesn't properly check the success value.
///
/// Returns true if the first variable (success) is None: `(, bytes memory data) = target.call(...)`
fn is_unchecked_multi_declaration(vars: &[Option<VariableDefinition<'_>>]) -> bool {
    vars.len() == 2 && vars.first().map_or(true, |v| v.is_none())
}

/// Checks if a tuple assignment doesn't properly check the success value.
///
/// Returns true if the expression is a tuple where:
/// - All elements are empty/underscore
/// - The first element (success position) is empty/underscore
fn is_unchecked_tuple_assignment(expr: &Expr<'_>) -> bool {
    if let ExprKind::Tuple(elements) = &expr.kind {
        // Check if the tuple is empty or the first element is None (underscore)
        elements.is_empty() || elements.first().map_or(true, |e| e.is_none())
    } else {
        false
    }
}
