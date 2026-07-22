use super::ControlledDelegatecall;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, ElementaryType, LitKind},
    interface::{Span, sym},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, Comparison, EffectiveBodyCx, ExprKind, ExprUse, ItemId, JoinSemiLattice, Res,
            StmtKind, TypeKind, ValueFlowAdapter, ValueFlowAnalysis, ValueFlowState, ValueOrigin,
            ValueSet, analyze_effective_body_flow_dispatches,
        },
        ty::{Ty, TyKind},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    CONTROLLED_DELEGATECALL,
    Severity::High,
    "controlled-delegatecall",
    "delegatecall target is not provably trusted"
);

impl<'hir> LateLintPass<'hir> for ControlledDelegatecall {
    fn check_nested_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        function_id: hir::FunctionId,
    ) {
        let function = hir.function(function_id);
        if function.body.is_none() {
            return;
        }

        let mut analysis = ValueFlowAdapter::new(Analysis::new(gcx));
        let _ = analyze_effective_body_flow_dispatches(
            gcx,
            function_id,
            ValueFlowState::new(false),
            &mut analysis,
        );
        for span in analysis.into_analysis().hits {
            ctx.emit(&CONTROLLED_DELEGATECALL, span);
        }
    }
}

type FlowState = ValueFlowState<bool>;

struct Analysis<'hir> {
    gcx: Gcx<'hir>,
    hits: HashSet<Span>,
}

impl<'hir> Analysis<'hir> {
    fn new(gcx: Gcx<'hir>) -> Self {
        Self { gcx, hits: HashSet::new() }
    }

    fn value(&self, state: &FlowState, expr: &'hir hir::Expr<'hir>) -> ValueSet<bool> {
        let outer = expr.peel_parens();
        let address_result = receiver_is_address(self.gcx, outer);
        let expr = self.gcx.peel_type_conversions(outer);

        // A compile-time address is attacker-independent even when its source expression uses a
        // lossy conversion. Lossy conversions are excluded only from equality proofs below.
        if address_result && self.gcx.try_eval_const_value(expr).is_ok() {
            return ValueSet::singleton(ValueOrigin::Expr(outer.id), true);
        }
        if self.gcx.expr_place(expr).is_some_and(|place| !place.projection().is_empty())
            && let Some(value) = state.place_value(self.gcx, expr)
        {
            return value;
        }

        match &expr.kind {
            ExprKind::Lit(literal)
                if matches!(literal.kind, LitKind::Address(_))
                    || matches!(&literal.kind, LitKind::Number(number) if number.is_zero()) =>
            {
                ValueSet::singleton(ValueOrigin::Expr(expr.id), true)
            }
            ExprKind::Ident(resolutions) => {
                if resolutions
                    .iter()
                    .any(|resolution| matches!(resolution, Res::Builtin(builtin) if builtin.name() == sym::this))
                {
                    return ValueSet::singleton(ValueOrigin::Expr(expr.id), true);
                }
                let variable = resolutions.iter().find_map(|resolution| resolution.as_variable());
                match variable {
                    Some(variable) if self.variable_is_constant_target(variable) => {
                        ValueSet::singleton(ValueOrigin::Initial(variable), true)
                    }
                    Some(variable) => state.variable(variable),
                    None => ValueSet::singleton(ValueOrigin::Expr(expr.id), false),
                }
            }
            ExprKind::Assign(_, None, rhs) => self.value(state, rhs),
            ExprKind::Ternary(_, if_true, if_false) => {
                let mut values = self.value(state, if_true);
                _ = values.join(&self.value(state, if_false));
                values
            }
            ExprKind::Call(..) => state
                .call_result(expr.id, 0)
                .cloned()
                .unwrap_or_else(|| ValueSet::singleton(ValueOrigin::Expr(expr.id), false)),
            _ => state.expr(self.gcx, expr, false),
        }
    }

    fn is_trusted_target(&self, state: &FlowState, expr: &'hir hir::Expr<'hir>) -> bool {
        self.value(state, expr).is_proven(|value| *value.property())
    }

    fn variable_is_constant_target(&self, variable: hir::VariableId) -> bool {
        let variable = self.gcx.hir.variable(variable);
        variable.is_constant() && variable_is_address_like(variable)
    }

    fn is_trusted_fact_target(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        let Some(place) = self.gcx.expr_place(expr) else { return false };
        let variable = self.gcx.hir.variable(place.local());
        (!place.is_state_backed(&self.gcx.hir) || variable.is_constant())
            && receiver_is_address(self.gcx, expr)
    }

    fn tracks_value(&self, variable: hir::VariableId) -> bool {
        !self.gcx.hir.variable(variable).kind.is_state()
    }
}

impl<'hir> ValueFlowAnalysis<'hir> for Analysis<'hir> {
    type Property = bool;
    type Domain = FlowState;

    fn flow(state: &Self::Domain) -> &ValueFlowState<Self::Property> {
        state
    }

    fn flow_mut(state: &mut Self::Domain) -> &mut ValueFlowState<Self::Property> {
        state
    }

    fn tracks_variable(&self, _cx: EffectiveBodyCx<'hir>, variable: hir::VariableId) -> bool {
        self.tracks_value(variable)
    }

    fn value_of(
        &self,
        _cx: EffectiveBodyCx<'hir>,
        state: &ValueFlowState<Self::Property>,
        expr: &'hir hir::Expr<'hir>,
    ) -> ValueSet<Self::Property> {
        self.value(state, expr)
    }

    fn unknown_property(&self) -> Self::Property {
        false
    }

    fn deleted_property(&self) -> Self::Property {
        true
    }

    fn inspect_expr_after(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir hir::Expr<'hir>,
        _use_: ExprUse,
        state: &mut Self::Domain,
    ) {
        if cx.reports_enabled()
            && cx.is_root()
            && cx
                .call_info(expr)
                .is_some_and(|info| info.builtin() == Some(Builtin::AddressDelegatecall))
            && let Some(receiver) = self.gcx.call_receiver(expr)
            && !self.is_trusted_target(state, receiver)
        {
            self.hits.insert(expr.span);
        }
    }

    fn apply_comparison_effect(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        comparison: Comparison<'hir>,
        state: &mut Self::Domain,
    ) {
        if comparison.op != BinOpKind::Eq {
            return;
        }
        for comparison in comparison.orientations() {
            let target = self.gcx.peel_injective_type_conversions(comparison.rhs);
            if self.is_trusted_target(state, comparison.lhs) && self.is_trusted_fact_target(target)
            {
                _ = state.refine_place(self.gcx, target, |property| *property = true);
            }
        }
    }

    fn inspect_statement_after(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        statement: &'hir hir::Stmt<'hir>,
        state: &mut Self::Domain,
    ) {
        if matches!(
            statement.kind,
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_)
        ) {
            state.forget_values();
        }
    }
}

const fn variable_is_address_like(variable: &hir::Variable<'_>) -> bool {
    matches!(
        variable.ty.kind,
        TypeKind::Elementary(ElementaryType::Address(_)) | TypeKind::Custom(ItemId::Contract(_))
    )
}

fn receiver_is_address<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(type_is_address)
}

fn type_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
}
