use super::ReentrancyEth;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{StateMutability, UnOpKind, Visibility},
    interface::Span,
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{
            self, EffectiveBodyCx, ExprKind, ExprUse, JoinSemiLattice, StorageAliasState,
            StorageFlowAdapter, StorageFlowAnalysis, StorageRoots, VariableId,
            analyze_effective_body_flow_dispatches,
        },
        ty::{CallGas, CallInfo, CallKind},
    },
};
use std::collections::{BTreeSet, HashSet};

declare_forge_lint!(
    REENTRANCY_ETH,
    Severity::High,
    "reentrancy-eth",
    "state read before ETH transfer is written after the transfer"
);

declare_forge_lint!(
    REENTRANCY_NO_ETH,
    Severity::Med,
    "reentrancy-no-eth",
    "state read before external call is written after the call"
);

impl<'hir> LateLintPass<'hir> for ReentrancyEth {
    fn check_nested_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        function_id: hir::FunctionId,
    ) {
        let function = hir.function(function_id);
        if !is_entry_point(function)
            || !ctx.is_lint_enabled(REENTRANCY_ETH.id) && !ctx.is_lint_enabled(REENTRANCY_NO_ETH.id)
        {
            return;
        }

        let mut analysis = StorageFlowAdapter::new(Analysis { ctx, gcx, emitted: HashSet::new() });
        let _ = analyze_effective_body_flow_dispatches(
            gcx,
            function_id,
            FlowState::default(),
            &mut analysis,
        );
    }
}

fn is_entry_point(function: &hir::Function<'_>) -> bool {
    !matches!(function.state_mutability, StateMutability::Pure | StateMutability::View)
        && !function.is_constructor()
        && (function.is_special()
            || function.kind.is_function()
                && matches!(function.visibility, Visibility::Public | Visibility::External))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReentrantCallKind {
    Eth,
    NoEth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingCall {
    span: Span,
    kind: ReentrantCallKind,
    state_reads: BTreeSet<VariableId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FlowState {
    state_reads: BTreeSet<VariableId>,
    pending_calls: Vec<PendingCall>,
    storage: StorageAliasState,
}

impl FlowState {
    fn push_call(&mut self, span: Span, kind: ReentrantCallKind) {
        if self.state_reads.is_empty() {
            return;
        }
        if let Some(existing) =
            self.pending_calls.iter_mut().find(|call| call.span == span && call.kind == kind)
        {
            existing.state_reads.extend(self.state_reads.iter().copied());
        } else {
            self.pending_calls.push(PendingCall {
                span,
                kind,
                state_reads: self.state_reads.clone(),
            });
        }
    }
}

impl JoinSemiLattice for FlowState {
    fn join(&mut self, other: &Self) -> bool {
        let old = self.clone();
        self.state_reads.extend(other.state_reads.iter().copied());
        for call in &other.pending_calls {
            if let Some(existing) = self
                .pending_calls
                .iter_mut()
                .find(|existing| existing.span == call.span && existing.kind == call.kind)
            {
                existing.state_reads.extend(call.state_reads.iter().copied());
            } else {
                self.pending_calls.push(call.clone());
            }
        }
        _ = self.storage.join(&other.storage);
        *self != old
    }
}

struct Analysis<'ctx, 'sess, 'config, 'hir> {
    ctx: &'ctx LintContext<'sess, 'config>,
    gcx: Gcx<'hir>,
    emitted: HashSet<Span>,
}

impl<'hir> Analysis<'_, '_, '_, 'hir> {
    fn call_kind(
        &self,
        cx: EffectiveBodyCx<'hir>,
        call: &'hir hir::Expr<'hir>,
    ) -> Option<ReentrantCallKind> {
        let info = cx.call_info(call)?;
        self.call_kind_with_info(info, call)
    }

    fn call_kind_with_info(
        &self,
        info: CallInfo<'hir>,
        call: &'hir hir::Expr<'hir>,
    ) -> Option<ReentrantCallKind> {
        let gas = self.gcx.call_gas(call);
        let stipend_transfer = gas.is_some_and(CallGas::is_stipend);
        if self.ctx.is_lint_enabled(REENTRANCY_ETH.id)
            && matches!(info.kind(), CallKind::External | CallKind::Call)
            && !stipend_transfer
            && self.gcx.call_transferred_value(call).is_some_and(|value| !self.is_zero(value))
            && gas.is_some_and(CallGas::is_forwarded)
        {
            return Some(ReentrantCallKind::Eth);
        }

        if !self.ctx.is_lint_enabled(REENTRANCY_NO_ETH.id)
            || info.is_contract_creation()
            || stipend_transfer
            || self.gcx.call_transferred_value(call).is_some_and(|value| !self.is_zero(value))
            || !info.is_state_mutating_external_interaction()
        {
            return None;
        }
        Some(ReentrantCallKind::NoEth)
    }

    fn is_zero(&self, expr: &hir::Expr<'_>) -> bool {
        self.gcx.try_eval_const_value(expr).is_ok_and(|value| value.is_zero())
    }

    fn emit_writes(&mut self, reports_enabled: bool, state: &FlowState, written: &[VariableId]) {
        if !reports_enabled {
            return;
        }
        for call in &state.pending_calls {
            let (lint, message) = match call.kind {
                ReentrantCallKind::Eth => {
                    (&REENTRANCY_ETH, "uncapped ETH transfer can be reentered before")
                }
                ReentrantCallKind::NoEth => {
                    (&REENTRANCY_NO_ETH, "external call can be reentered before")
                }
            };
            if !self.ctx.is_lint_enabled(lint.id) || self.emitted.contains(&call.span) {
                continue;
            }
            let Some(&variable) =
                written.iter().find(|&&variable| call.state_reads.contains(&variable))
            else {
                continue;
            };
            let name = self
                .gcx
                .hir
                .variable(variable)
                .name
                .map(|name| name.as_str().to_string())
                .unwrap_or_else(|| "state".to_string());
            self.ctx.emit_with_msg(lint, call.span, format!("{message} `{name}` is updated"));
            self.emitted.insert(call.span);
        }
    }

    fn emit_target_writes(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        state: &FlowState,
        target: &'hir hir::Expr<'hir>,
    ) {
        let roots = state.storage.write_roots(self.gcx, target);
        let written = self.written_roots(cx, &roots);
        self.emit_writes(cx.reports_enabled(), state, &written);
    }

    fn written_roots(&self, cx: EffectiveBodyCx<'hir>, roots: &StorageRoots) -> Vec<VariableId> {
        match roots.known_roots() {
            Some(roots) => roots.to_vec(),
            None => cx.mutable_state_variables().to_vec(),
        }
    }

    fn add_read_roots(
        &self,
        cx: EffectiveBodyCx<'hir>,
        state: &mut FlowState,
        roots: &StorageRoots,
        unknown_is_storage: bool,
    ) {
        if roots.may_be_unknown() && unknown_is_storage {
            state.state_reads.extend(cx.mutable_state_variables());
        } else {
            state.state_reads.extend(roots.iter_known());
        }
    }
}

impl<'hir> StorageFlowAnalysis<'hir> for Analysis<'_, '_, '_, 'hir> {
    type Domain = FlowState;

    fn storage(state: &Self::Domain) -> &StorageAliasState {
        &state.storage
    }

    fn storage_mut(state: &mut Self::Domain) -> &mut StorageAliasState {
        &mut state.storage
    }

    fn inspect_expr_before(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir hir::Expr<'hir>,
        use_: ExprUse,
        state: &mut Self::Domain,
    ) {
        let roots = state.storage.read_roots(self.gcx, expr, &use_);
        self.add_read_roots(cx, state, &roots, true);
        match &expr.kind {
            ExprKind::Assign(target, ..) | ExprKind::Delete(target) => {
                self.emit_target_writes(cx, state, target);
            }
            ExprKind::Unary(operation, target)
                if matches!(
                    operation.kind,
                    UnOpKind::PreInc | UnOpKind::PreDec | UnOpKind::PostInc | UnOpKind::PostDec
                ) =>
            {
                self.emit_target_writes(cx, state, target);
            }
            ExprKind::Call(..) => {
                if let Some(kind) = self.call_kind(cx, expr) {
                    state.push_call(expr.span, kind);
                }
                if cx.call_info(expr).is_some_and(|info| {
                    matches!(
                        info.builtin(),
                        Some(Builtin::ArrayPush0 | Builtin::ArrayPush | Builtin::ArrayPop)
                    )
                }) && let Some(receiver) = self.gcx.call_receiver(expr)
                {
                    self.emit_target_writes(cx, state, receiver);
                }
                if cx.call_info(expr).is_some_and(|info| info.builtin() == Some(Builtin::YulSstore))
                    && let Some(slot) = self.gcx.call_arg(expr, 0)
                {
                    let roots = state.storage.storage_access_roots(self.gcx, slot);
                    let written = self.written_roots(cx, &roots);
                    self.emit_writes(cx.reports_enabled(), state, &written);
                }
                if cx.call_info(expr).is_some_and(|info| info.builtin() == Some(Builtin::YulSload))
                    && let Some(slot) = self.gcx.call_arg(expr, 0)
                {
                    let roots = state.storage.storage_access_roots(self.gcx, slot);
                    self.add_read_roots(cx, state, &roots, true);
                }
            }
            _ => {}
        }
    }

    fn apply_indirect_internal_call_effect(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        call: &'hir hir::Expr<'hir>,
        state: &mut Self::Domain,
    ) {
        let variables = cx.mutable_state_variables();
        self.emit_writes(cx.reports_enabled(), state, variables);
        state.state_reads.extend(variables);
        state.push_call(call.span, ReentrantCallKind::NoEth);
        state.push_call(call.span, ReentrantCallKind::Eth);
        state.storage.forget();
    }
}
