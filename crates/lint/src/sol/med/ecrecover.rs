use super::Ecrecover;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::{U256, uint};
use solar::{
    ast::{BinOpKind, ElementaryType},
    interface::Span,
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, Comparison, EffectiveBodyCx, ExprKind, ExprUse, JoinSemiLattice, Place, StmtKind,
            TypeKind, ValueFlowAdapter, ValueFlowAnalysis, ValueFlowState, ValueOrigin, ValueSet,
            analyze_effective_body_flow_dispatches,
        },
    },
};
use std::collections::HashMap;

declare_forge_lint!(
    ECRECOVER,
    Severity::Med,
    "ecrecover",
    "ecrecover should reject malleable signatures"
);

/// Largest canonical secp256k1 `s` value, `n / 2`.
const SECP256K1_HALF_ORDER: U256 =
    uint!(0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0_U256);

impl<'hir> LateLintPass<'hir> for Ecrecover {
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

        let analysis = Analysis::new(gcx, function.returns);
        let mut initial = FlowState::default();
        analysis.seed_constant_values(&mut initial);
        let mut analysis = ValueFlowAdapter::new(analysis);
        let results =
            analyze_effective_body_flow_dispatches(gcx, function_id, initial, &mut analysis);
        for result in results {
            if let Some(mut state) = result.fallthrough().cloned() {
                analysis.analysis_mut().use_return_values(&mut state);
            }
        }
        for span in analysis.into_analysis().hits {
            ctx.emit(&ECRECOVER, span);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingRecovery {
    signature: ValueSet<bool>,
    span: Span,
    reportable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FlowState {
    values: ValueFlowState<bool>,
    pending: HashMap<ValueOrigin, Vec<PendingRecovery>>,
}

impl Default for FlowState {
    fn default() -> Self {
        Self { values: ValueFlowState::new(false), pending: HashMap::new() }
    }
}

impl JoinSemiLattice for FlowState {
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = self.values.join(&other.values);
        for (origin, recoveries) in &other.pending {
            let pending = self.pending.entry(origin.clone()).or_default();
            for recovery in recoveries {
                if !pending.contains(recovery) {
                    pending.push(recovery.clone());
                    changed = true;
                }
            }
        }
        changed
    }
}

struct Analysis<'hir> {
    gcx: Gcx<'hir>,
    returns: &'hir [hir::VariableId],
    hits: Vec<Span>,
}

impl<'hir> Analysis<'hir> {
    const fn new(gcx: Gcx<'hir>, returns: &'hir [hir::VariableId]) -> Self {
        Self { gcx, returns, hits: Vec::new() }
    }

    fn hit(&mut self, span: Span) {
        if !self.hits.contains(&span) {
            self.hits.push(span);
        }
    }

    fn seed_constant_values(&self, state: &mut FlowState) {
        for variable in self.gcx.hir.variable_ids() {
            let declaration = self.gcx.hir.variable(variable);
            if declaration.is_constant()
                && declaration.initializer.is_some_and(|value| self.const_is_low_s(value))
            {
                state.values.seed(variable, true);
            }
        }
    }

    fn value(&self, state: &ValueFlowState<bool>, expr: &'hir hir::Expr<'hir>) -> ValueSet<bool> {
        let expr = self.peel_value_cast(expr);
        match &expr.kind {
            ExprKind::Assign(_, None, rhs) => self.value(state, rhs),
            ExprKind::Ternary(_, then_expr, else_expr) => {
                let mut values = self.value(state, then_expr);
                _ = values.join(&self.value(state, else_expr));
                values
            }
            _ => state.expr(self.gcx, expr, self.const_is_low_s(expr)),
        }
    }

    fn copied_value(
        &self,
        state: &ValueFlowState<bool>,
        expr: &'hir hir::Expr<'hir>,
    ) -> ValueSet<bool> {
        let expr = self.peel_value_cast(expr);
        match &expr.kind {
            ExprKind::Assign(_, None, rhs) => self.copied_value(state, rhs),
            ExprKind::Ternary(_, then_expr, else_expr) => {
                let mut values = self.copied_value(state, then_expr);
                _ = values.join(&self.copied_value(state, else_expr));
                values
            }
            _ => self.value(state, expr),
        }
    }

    fn peel_value_cast(&self, expr: &'hir hir::Expr<'hir>) -> &'hir hir::Expr<'hir> {
        self.peel_signature_cast(self.gcx.peel_injective_type_conversions(expr))
    }

    fn peel_signature_cast(&self, mut expr: &'hir hir::Expr<'hir>) -> &'hir hir::Expr<'hir> {
        loop {
            expr = expr.peel_parens();
            let ExprKind::Call(callee, args, options) = &expr.kind else { return expr };
            if options.is_some() || !is_transparent_signature_cast(callee) || args.len() != 1 {
                return expr;
            }
            expr = args.exprs().next().unwrap();
        }
    }

    fn const_is_low_s(&self, expr: &'hir hir::Expr<'hir>) -> bool {
        self.const_value(expr).is_some_and(|value| value <= SECP256K1_HALF_ORDER)
    }

    fn const_value(&self, expr: &'hir hir::Expr<'hir>) -> Option<U256> {
        self.gcx.try_eval_const_u256_wrapping(self.peel_signature_cast(expr))
    }

    fn invalidate_mutable_state(
        &self,
        cx: EffectiveBodyCx<'hir>,
        state: &mut FlowState,
        call: hir::ExprId,
    ) {
        for &variable in cx.mutable_state_variables() {
            state.values.invalidate_variable(variable, call, false);
        }
    }

    fn record_recovery(
        &mut self,
        state: &mut FlowState,
        call: &'hir hir::Expr<'hir>,
        signature: ValueSet<bool>,
        reportable: bool,
    ) {
        let recovery = PendingRecovery { signature, span: call.span, reportable };
        let pending = state.pending.entry(ValueOrigin::Expr(call.id)).or_default();
        if !pending.contains(&recovery) {
            pending.push(recovery);
        }
    }

    fn use_value(&mut self, state: &mut FlowState, value: ValueSet<bool>) {
        let origins: Vec<_> = value.iter().map(|value| value.origin()).collect();
        for origin in origins {
            let Some(recoveries) = state.pending.remove(&origin) else { continue };
            for recovery in recoveries {
                if recovery.reportable && !recovery.signature.is_proven(|value| *value.property()) {
                    self.hit(recovery.span);
                }
            }
        }
    }

    fn use_return_values(&mut self, state: &mut FlowState) {
        let values: Vec<_> = self
            .returns
            .iter()
            .map(|&variable| state.values.values_read_from_place(&Place::from_local(variable)))
            .collect();
        for value in values {
            self.use_value(state, value);
        }
    }

    fn mark_low_s(&self, state: &mut FlowState, expr: &'hir hir::Expr<'hir>) {
        let expr = self.peel_signature_cast(expr);
        let Some(origins) = state.values.refine_place(self.gcx, expr, |property| *property = true)
        else {
            return;
        };
        let origins: Vec<_> = origins.iter().map(|value| value.origin()).collect();
        let evaluated = state.values.evaluated_sites();
        for recoveries in state.pending.values_mut() {
            for recovery in recoveries {
                recovery.signature.update_matching(
                    |origin| {
                        origins
                            .iter()
                            .any(|candidate| evaluated.origins_are_correlatable(candidate, &origin))
                    },
                    |property| *property = true,
                );
            }
        }
    }

    fn add_comparison_fact(&self, comparison: Comparison<'hir>, state: &mut FlowState) {
        for comparison in comparison.orientations() {
            let Some(bound) = self.const_value(comparison.rhs) else { continue };
            let proves_low = match comparison.op {
                BinOpKind::Lt => bound <= SECP256K1_HALF_ORDER + U256::from(1),
                BinOpKind::Le | BinOpKind::Eq => bound <= SECP256K1_HALF_ORDER,
                _ => false,
            };
            if proves_low {
                self.mark_low_s(state, comparison.lhs);
            }
        }
    }

    fn use_is_observable(&self, use_: &ExprUse) -> bool {
        match use_ {
            ExprUse::Value | ExprUse::Callee | ExprUse::Discard => true,
            ExprUse::Projection | ExprUse::Place => false,
            ExprUse::Store(Some(place)) => place.is_state_backed(&self.gcx.hir),
            ExprUse::Store(None) => true,
        }
    }

    fn use_pending(&mut self, state: &mut FlowState) {
        let pending = std::mem::take(&mut state.pending);
        for recoveries in pending.into_values().flatten() {
            if recoveries.reportable && !recoveries.signature.is_proven(|value| *value.property()) {
                self.hit(recoveries.span);
            }
        }
    }
}

impl<'hir> ValueFlowAnalysis<'hir> for Analysis<'hir> {
    type Property = bool;
    type Domain = FlowState;

    fn flow(state: &Self::Domain) -> &ValueFlowState<Self::Property> {
        &state.values
    }

    fn flow_mut(state: &mut Self::Domain) -> &mut ValueFlowState<Self::Property> {
        &mut state.values
    }

    fn value_of(
        &self,
        _cx: EffectiveBodyCx<'hir>,
        values: &ValueFlowState<Self::Property>,
        expr: &'hir hir::Expr<'hir>,
    ) -> ValueSet<Self::Property> {
        self.copied_value(values, expr)
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
        use_: ExprUse,
        state: &mut Self::Domain,
    ) {
        if use_ == ExprUse::Projection && self.gcx.expr_place(expr).is_some() {
            self.use_value(state, self.value(&state.values, expr));
        } else if self.use_is_observable(&use_)
            && let Some(value) = state.values.values_read_from_expr(self.gcx, expr)
        {
            self.use_value(state, value);
        }

        if let ExprKind::Assign(target, ..) = &expr.kind
            && use_ != ExprUse::Discard
            && self.use_is_observable(&use_)
        {
            self.use_value(state, self.value(&state.values, target));
        }

        if cx.call_info(expr).is_some_and(|info| info.builtin() == Some(Builtin::EcRecover))
            && let Some(signature) = self.gcx.call_arg(expr, 3)
        {
            let signature = self.value(&state.values, signature);
            if !signature.is_proven(|value| *value.property()) {
                let reportable = cx.reports_enabled() && cx.is_root();
                let defer_recovery = matches!(
                    &use_,
                    ExprUse::Store(Some(place))
                        if !place.is_state_backed(&self.gcx.hir)
                            && state.values.can_track_place(self.gcx, place)
                );
                if defer_recovery {
                    self.record_recovery(state, expr, signature, reportable);
                } else if reportable {
                    self.hit(expr.span);
                }
            }
        }

        if cx
            .call_info(expr)
            .is_some_and(|info| info.may_write_state() && !info.is_direct_internal())
        {
            self.invalidate_mutable_state(cx, state, expr.id);
        }
    }

    fn apply_comparison_effect(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        comparison: Comparison<'hir>,
        state: &mut Self::Domain,
    ) {
        self.add_comparison_fact(comparison, state);
    }

    fn inspect_statement_after(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        statement: &'hir hir::Stmt<'hir>,
        state: &mut Self::Domain,
    ) {
        match statement.kind {
            StmtKind::Return(None) if cx.is_root() => self.use_return_values(state),
            StmtKind::AssemblyBlock(_) | StmtKind::Switch(_) | StmtKind::Err(_) => {
                self.use_pending(state);
                state.values.forget_values();
            }
            _ => {}
        }
    }

    fn apply_indirect_internal_call_effect(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        _call: &'hir hir::Expr<'hir>,
        state: &mut Self::Domain,
    ) {
        self.use_pending(state);
        state.values.forget_values();
    }
}

fn is_transparent_signature_cast(callee: &hir::Expr<'_>) -> bool {
    matches!(
        &callee.peel_parens().kind,
        ExprKind::Type(hir::Type {
            kind: TypeKind::Elementary(
                ElementaryType::UInt(size) | ElementaryType::FixedBytes(size)
            ),
            ..
        }) if size.bits() == 256
    )
}
