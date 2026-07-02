use super::FunctionInitState;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::StateMutability,
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind, FunctionId, Hir, ItemId, Res, VariableId, Visit},
        ty::TyKind,
    },
};
use std::{convert::Infallible, ops::ControlFlow};

declare_forge_lint!(
    FUNCTION_INIT_STATE,
    Severity::Info,
    "function-init-state",
    "state variable initializer depends on a non-pure function or another state variable"
);

impl<'hir> LateLintPass<'hir> for FunctionInitState {
    fn check_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        contract: &'hir hir::Contract<'hir>,
    ) {
        // State variable initializers run at construction, before the constructor body, in
        // base-to-derived order: reading another non-constant state variable or calling a
        // non-pure function there observes that partial state. Constants are fixed at compile
        // time, so both constant declarations and references to constants are fine.
        for item_id in contract.items {
            if let ItemId::Variable(variable_id) = item_id {
                let variable = hir.variable(*variable_id);
                // A constant's initializer is restricted to compile-time constant expressions.
                if variable.is_state_variable()
                    && !variable.is_constant()
                    && let Some(initializer) = variable.initializer
                {
                    let mut finder = ImpureRefFinder { gcx, hir, found: false };
                    let _ = finder.visit_expr(initializer);
                    if finder.found {
                        ctx.emit(&FUNCTION_INIT_STATE, variable.span);
                    }
                }
            }
        }
    }
}

/// Looks for a reference to a non-constant state variable or to a non-pure function anywhere in
/// an initializer expression, arguments of nested calls included.
struct ImpureRefFinder<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    found: bool,
}

impl<'hir> Visit<'hir> for ImpureRefFinder<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            ExprKind::Ident(resolutions) => {
                for res in *resolutions {
                    match res {
                        Res::Item(ItemId::Variable(variable_id)) => {
                            self.judge_variable(*variable_id);
                        }
                        Res::Item(ItemId::Function(function_id)) => {
                            self.judge_function(*function_id);
                        }
                        _ => {}
                    }
                }
            }
            // A qualified or external access (`Base.viewFn()`, `Oracle(addr).price()`): the
            // member ident carries no resolution, so type the base; when it is a contract,
            // judge its same-name functions and variables. A contract name used as the base
            // is a type-namespace item, so its type comes wrapped as `Type(Contract(..))`,
            // while a contract-typed value comes bare.
            ExprKind::Member(base, member) => {
                let base_ty = self.gcx.type_of_expr(base.peel_parens().id).map(|ty| {
                    let ty = ty.peel_refs();
                    match ty.kind {
                        TyKind::Type(inner) => inner.peel_refs(),
                        _ => ty,
                    }
                });
                if let Some(ty) = base_ty
                    && let TyKind::Contract(contract_id) = ty.kind
                {
                    for item_id in self.hir.contract(contract_id).items {
                        match item_id {
                            ItemId::Variable(variable_id)
                                if self
                                    .hir
                                    .variable(*variable_id)
                                    .name
                                    .is_some_and(|name| name.name == member.name) =>
                            {
                                self.judge_variable(*variable_id);
                            }
                            ItemId::Function(function_id)
                                if self
                                    .hir
                                    .function(*function_id)
                                    .name
                                    .is_some_and(|name| name.name == member.name) =>
                            {
                                self.judge_function(*function_id);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl ImpureRefFinder<'_> {
    /// A read of another state variable: its initializer may not have run yet.
    fn judge_variable(&mut self, variable_id: VariableId) {
        let variable = self.hir.variable(variable_id);
        if variable.is_state_variable() && !variable.is_constant() {
            self.found = true;
        }
    }

    /// A non-pure function observes the same partial state. A variable referenced through its
    /// synthesized getter is judged as a read of the variable itself, so a public constant
    /// stays fine.
    fn judge_function(&mut self, function_id: FunctionId) {
        let function = self.hir.function(function_id);
        if let Some(variable_id) = function.gettee {
            self.judge_variable(variable_id);
        } else if function.state_mutability != StateMutability::Pure {
            self.found = true;
        }
    }
}
