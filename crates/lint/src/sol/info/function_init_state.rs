use super::FunctionInitState;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::StateMutability,
    sema::{
        Gcx,
        hir::{
            self, CallArgsKind, ContractId, Expr, ExprKind, FunctionId, Hir, ItemId, Res,
            VariableId, Visit,
        },
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
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        id: ContractId,
    ) {
        // State variable initializers run at construction, before the constructor body, in
        // base-to-derived order: reading another non-constant state variable or calling a
        // non-pure function there observes that partial state. Constants are fixed at compile
        // time, so both constant declarations and references to constants are fine.
        let contract = hir.contract(id);
        for item_id in contract.items {
            if let ItemId::Variable(variable_id) = item_id {
                let variable = hir.variable(*variable_id);
                // A constant's initializer is restricted to compile-time constant expressions.
                if variable.is_state_variable()
                    && !variable.is_constant()
                    && let Some(initializer) = variable.initializer
                {
                    let mut finder = ImpureRefFinder {
                        gcx,
                        hir,
                        source: contract.source,
                        contract: id,
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
}

/// Looks for a reference to a non-constant state variable or to a non-pure function anywhere in
/// an initializer expression, arguments of nested calls included.
struct ImpureRefFinder<'hir> {
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    /// The source and contract of the initializer, the viewpoint for `using for` lookups.
    source: hir::SourceId,
    contract: ContractId,
    found: bool,
}

impl<'hir> Visit<'hir> for ImpureRefFinder<'hir> {
    type BreakValue = Infallible;

    fn hir(&self) -> &'hir Hir<'hir> {
        self.hir
    }

    fn visit_expr(&mut self, expr: &'hir Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        match &expr.kind {
            // A call: the type checker already resolved the one function it dispatches to
            // (overload selection by argument types, override shadowing, `super.` and the
            // qualified forms), so judge that target alone and walk the rest manually,
            // skipping the default walk that would re-judge the callee by name matching.
            ExprKind::Call(callee, args, opts) => {
                if let Some(function_id) = self.resolved_function(callee) {
                    self.judge_function(function_id);
                }
                match &callee.peel_parens().kind {
                    // The callee name can also resolve to a variable: a call through a
                    // function pointer stored in state reads that variable.
                    ExprKind::Ident(resolutions) => {
                        for res in *resolutions {
                            if let Res::Item(ItemId::Variable(variable_id)) = res {
                                self.judge_variable(*variable_id);
                            }
                        }
                    }
                    // The receiver of a member call can read state itself.
                    ExprKind::Member(base, _) => {
                        let _ = self.visit_expr(base);
                    }
                    _ => {
                        let _ = self.visit_expr(callee);
                    }
                }
                if let Some(opts) = opts {
                    for arg in opts.args {
                        let _ = self.visit_expr(&arg.value);
                    }
                }
                match &args.kind {
                    CallArgsKind::Unnamed(exprs) => {
                        for arg in *exprs {
                            let _ = self.visit_expr(arg);
                        }
                    }
                    CallArgsKind::Named(named) => {
                        for arg in *named {
                            let _ = self.visit_expr(&arg.value);
                        }
                    }
                }
                return ControlFlow::Continue(());
            }
            // A plain reference to a name outside a call position.
            ExprKind::Ident(resolutions) => self.judge_resolutions(resolutions),
            // A member reference used as a value has a resolved target too (`x.f` selects an
            // override like `x.f()` would): judge it when the type checker gives one, and
            // scan by name only when there is no resolved function.
            ExprKind::Member(base, member) => {
                if let Some(function_id) = self.resolved_function(expr) {
                    self.judge_function(function_id);
                } else {
                    self.judge_member(base, member);
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}

impl ImpureRefFinder<'_> {
    /// The single function an expression resolves to, for a callee or for a member reference
    /// used as a value. `type_of_expr` is the function the type checker resolved, so overload
    /// selection by argument types, override shadowing, `super.` and the qualified and
    /// `using for` forms are already accounted for.
    fn resolved_function(&self, expr: &Expr<'_>) -> Option<FunctionId> {
        let ty = self.gcx.type_of_expr(expr.peel_parens().id)?;
        match ty.kind {
            TyKind::Fn(function_ty) => function_ty.function_id,
            _ => None,
        }
    }

    fn judge_resolutions(&mut self, resolutions: &[Res]) {
        for res in resolutions {
            match res {
                Res::Item(ItemId::Variable(variable_id)) => self.judge_variable(*variable_id),
                Res::Item(ItemId::Function(function_id)) => {
                    self.judge_function(*function_id);
                }
                _ => {}
            }
        }
    }

    /// Judges a member read with no resolved function type (`Base.stateVar`): the member
    /// ident carries no resolution, so type the base and scan by name. Calls and function
    /// references never come here, their target is resolved from the expression type.
    fn judge_member(&mut self, base: &Expr<'_>, member: &solar::ast::Ident) {
        let Some(ty) = self.gcx.type_of_expr(base.peel_parens().id) else { return };
        // A contract name used as the base is a type-namespace item, so its type comes wrapped
        // as `Type(Contract(..))`, while a contract-typed value comes bare.
        let ty = ty.peel_refs();
        let ty = match ty.kind {
            TyKind::Type(inner) => inner.peel_refs(),
            _ => ty,
        };
        if let TyKind::Contract(contract_id) = ty.kind {
            // Walk the linearization: an inherited function or getter is not among the
            // contract's own items.
            for base_id in self.hir.contract(contract_id).linearized_bases {
                for item_id in self.hir.contract(*base_id).items {
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
        } else {
            // A `using for` binding read as a value: the bound library function is a member of
            // the value type.
            for member_entry in self.gcx.members_of(ty, self.source, Some(self.contract)) {
                if member_entry.name == member.name
                    && let TyKind::Fn(function_ty) = member_entry.ty.kind
                    && let Some(function_id) = function_ty.function_id
                {
                    self.judge_function(function_id);
                }
            }
        }
    }

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
