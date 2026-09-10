use super::FunctionInitState;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::StateMutability,
    sema::{
        Gcx,
        hir::{ContractId, Expr, FunctionId, Hir, VariableId, Visit},
    },
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    FUNCTION_INIT_STATE,
    Severity::Info,
    "function-init-state",
    "state variable initializer depends on a non-pure function or another state variable"
);

impl<'gcx> LateLintPass<'gcx> for FunctionInitState {
    fn check_nested_contract(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, id: ContractId) {
        // State variable initializers run at construction, before the constructor body, in
        // base-to-derived order: reading another non-constant state variable or calling a
        // non-pure function there observes that partial state. Constants are fixed at compile
        // time, so both constant declarations and references to constants are fine.
        let contract = gcx.hir.contract(id);
        for item_id in contract.items {
            let Some(variable) = item_id.as_variable().map(|v| gcx.hir.variable(v)) else {
                continue;
            };
            if variable.is_state_variable()
                && !variable.is_constant()
                && let Some(initializer) = variable.initializer
            {
                let mut finder = ImpureRefFinder { gcx, found: false };
                let _ = finder.visit_expr(initializer);
                if finder.found {
                    ctx.emit(&FUNCTION_INIT_STATE, variable.span);
                }
            }
        }
    }
}

/// Looks for a reference to a non-constant state variable or to a non-pure function anywhere in
/// an initializer expression, arguments of nested calls included.
struct ImpureRefFinder<'gcx> {
    gcx: Gcx<'gcx>,
    found: bool,
}

impl<'gcx> Visit<'gcx> for ImpureRefFinder<'gcx> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let Some(variable_id) = self.gcx.resolved_variable(expr) {
            self.judge_variable(variable_id);
        } else if let Some(function_id) = self.gcx.resolved_function(expr) {
            self.judge_function(function_id);
        }
        self.walk_expr(expr)
    }
}

impl ImpureRefFinder<'_> {
    /// A read of another state variable: its initializer may not have run yet.
    fn judge_variable(&mut self, variable_id: VariableId) {
        let variable = self.gcx.hir.variable(variable_id);
        self.found |= variable.is_state_variable() && !variable.is_constant();
    }

    /// A non-pure function observes the same partial state. A variable referenced through its
    /// synthesized getter is judged as a read of the variable itself, so a public constant
    /// stays fine.
    fn judge_function(&mut self, function_id: FunctionId) {
        let function = self.gcx.hir.function(function_id);
        match function.gettee {
            Some(variable_id) => self.judge_variable(variable_id),
            None => self.found |= function.state_mutability != StateMutability::Pure,
        }
    }
}
