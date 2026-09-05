use super::FunctionInitState;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{resolved_function, ty_contract_id},
    },
};
use solar::{
    ast::StateMutability,
    interface::Symbol,
    sema::{
        Gcx,
        hir::{self, ContractId, Expr, ExprKind, FunctionId, Hir, ItemId, VariableId, Visit},
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
                let mut finder = ImpureRefFinder {
                    gcx,
                    source: contract.source,
                    contract: id,
                    callee: None,
                    found: false,
                };
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
    /// The source and contract of the initializer, the viewpoint for `using for` lookups.
    source: hir::SourceId,
    contract: ContractId,
    /// The callee of the call being walked: its target was already judged through the type
    /// checker's resolution, so it must not be re-judged by name matching.
    callee: Option<hir::ExprId>,
    found: bool,
}

impl<'gcx> Visit<'gcx> for ImpureRefFinder<'gcx> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        let is_callee = self.callee == Some(expr.id);
        match &expr.kind {
            // The type checker already resolved the one function a call dispatches to (overload
            // selection, override shadowing, `super.`, the qualified and `using for` forms).
            ExprKind::Call(callee, ..) => {
                if let Some(function_id) = resolved_function(self.gcx, callee) {
                    self.judge_function(function_id);
                }
                self.callee = Some(callee.peel_parens().id);
            }
            // A callee name can also resolve to a variable: a call through a function pointer
            // stored in state reads that variable.
            ExprKind::Ident(resolutions) => {
                for res in *resolutions {
                    match res.as_variable() {
                        Some(variable_id) => self.judge_variable(variable_id),
                        None => {
                            if !is_callee && let Some(function_id) = res.as_function() {
                                self.judge_function(function_id);
                            }
                        }
                    }
                }
            }
            // A member reference used as a value has a resolved target too (`x.f` selects an
            // override like `x.f()` would); scan by name only when there is none.
            ExprKind::Member(base, member) if !is_callee => {
                match resolved_function(self.gcx, expr) {
                    Some(function_id) => self.judge_function(function_id),
                    None => self.judge_member(base, member.name),
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl ImpureRefFinder<'_> {
    /// Judges a member read with no resolved function type (`Base.stateVar`): the member ident
    /// carries no resolution, so type the base and scan by name.
    fn judge_member(&mut self, base: &Expr<'_>, member: Symbol) {
        let gcx = self.gcx;
        let Some(ty) = gcx.type_of_expr(base.peel_parens().id) else { return };
        if let Some(contract_id) = ty_contract_id(ty) {
            // Walk the linearization: an inherited function or getter is not among the
            // contract's own items.
            for &base_id in gcx.hir.contract(contract_id).linearized_bases {
                for &item_id in gcx.hir.contract(base_id).items {
                    match item_id {
                        ItemId::Variable(id)
                            if gcx.hir.variable(id).name.is_some_and(|n| n.name == member) =>
                        {
                            self.judge_variable(id)
                        }
                        ItemId::Function(id)
                            if gcx.hir.function(id).name.is_some_and(|n| n.name == member) =>
                        {
                            self.judge_function(id)
                        }
                        _ => {}
                    }
                }
            }
        } else {
            // A `using for` binding read as a value: the bound library function is a member of
            // the value type. `members_of` needs reference types to keep their data location.
            for entry in gcx.members_of(ty, self.source, Some(self.contract)) {
                if entry.name == member
                    && let Some(function_id) = entry.ty.function_id()
                {
                    self.judge_function(function_id);
                }
            }
        }
    }

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
