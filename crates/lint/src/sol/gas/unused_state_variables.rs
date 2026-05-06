use super::UnusedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ContractKind,
    interface::data_structures::Never,
    sema::hir::{self, Visit as _},
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    UNUSED_STATE_VARIABLES,
    Severity::Gas,
    "unused-state-variables",
    "state variable is never used"
);

impl<'hir> LateLintPass<'hir> for UnusedStateVariables {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        hir: &'hir hir::Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        // Skip interfaces, they cannot have mutable state variables.
        if contract.kind == ContractKind::Interface {
            return;
        }

        // Collect state variable IDs, skipping constants and immutables
        // (those are handled by the compiler and don't occupy storage slots).
        let state_vars: Vec<hir::VariableId> = contract
            .variables()
            .filter(|&var_id| {
                let var = hir.variable(var_id);
                !var.is_constant() && !var.is_immutable()
            })
            .collect();

        if state_vars.is_empty() {
            return;
        }

        // Walk the full contract — functions (including modifier call args, parameters, returns,
        // and bodies) and state variable initializers — to collect every variable referenced
        // anywhere in this contract.
        let mut collector = UsedVarCollector { hir, used: HashSet::new() };
        for func_id in contract.all_functions() {
            let _ = collector.visit_nested_function(func_id);
        }
        // State variables can reference other state variables in their initializers.
        for var_id in contract.variables() {
            let _ = collector.visit_nested_var(var_id);
        }

        // Report any state variable that was never referenced.
        for var_id in state_vars {
            if !collector.used.contains(&var_id) {
                let var = hir.variable(var_id);
                ctx.emit(&UNUSED_STATE_VARIABLES, var.span);
            }
        }
    }
}

struct UsedVarCollector<'hir> {
    hir: &'hir hir::Hir<'hir>,
    used: HashSet<hir::VariableId>,
}

impl<'hir> hir::Visit<'hir> for UsedVarCollector<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::ExprKind::Ident(resolutions) = &expr.kind {
            for res in *resolutions {
                if let hir::Res::Item(hir::ItemId::Variable(var_id)) = res {
                    self.used.insert(*var_id);
                }
            }
        }
        self.walk_expr(expr)
    }
}
