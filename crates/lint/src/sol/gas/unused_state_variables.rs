use super::UnusedStateVariables;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::ContractKind,
    interface::data_structures::Never,
    sema::{
        Gcx,
        hir::{self, ExprKind, Res, Visit as _},
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(
    UNUSED_STATE_VARIABLES,
    Severity::Gas,
    "unused-state-variables",
    "state variable is never used"
);

impl<'gcx> LateLintPass<'gcx> for UnusedStateVariables {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'gcx>,
        contract: &'gcx hir::Contract<'gcx>,
    ) {
        if contract.kind == ContractKind::Interface {
            return;
        }

        // Functions (including modifier call args) and state variable initializers cover every
        // variable reference in the contract.
        let mut collector = UsedVarCollector { hir: &gcx.hir, used: HashSet::new() };
        for func_id in contract.all_functions() {
            let _ = collector.visit_nested_function(func_id);
        }
        for var_id in contract.variables() {
            let _ = collector.visit_nested_var(var_id);
        }

        // Constants and immutables do not occupy storage slots.
        for var_id in contract.variables() {
            let var = gcx.hir.variable(var_id);
            if !var.is_constant() && !var.is_immutable() && !collector.used.contains(&var_id) {
                ctx.emit(&UNUSED_STATE_VARIABLES, var.span);
            }
        }
    }
}

struct UsedVarCollector<'gcx> {
    hir: &'gcx hir::Hir<'gcx>,
    used: HashSet<hir::VariableId>,
}

impl<'gcx> hir::Visit<'gcx> for UsedVarCollector<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'gcx hir::Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Ident(reses) = &expr.kind {
            self.used.extend(reses.iter().filter_map(Res::as_variable));
        }
        self.walk_expr(expr)
    }
}
