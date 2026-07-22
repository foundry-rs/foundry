use super::ReentrancyEvents;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::Span,
    sema::{
        Gcx,
        hir::{
            EffectiveBodyCx, EffectiveFlowAnalysis, Expr, FunctionId, Hir, InternalCallMode,
            OperandOrder, Stmt, StmtKind, analyze_effective_body_flow_dispatches,
        },
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    REENTRANCY_EVENTS,
    Severity::Low,
    "reentrancy-events",
    "event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on"
);

impl<'hir> LateLintPass<'hir> for ReentrancyEvents {
    fn check_nested_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir Hir<'hir>,
        function_id: FunctionId,
    ) {
        let mut analysis = ReentrancyAnalysis { ctx, emitted: HashSet::new() };
        let _ = analyze_effective_body_flow_dispatches(gcx, function_id, false, &mut analysis);
    }
}

struct ReentrancyAnalysis<'ctx, 'sess, 'config> {
    ctx: &'ctx LintContext<'sess, 'config>,
    emitted: HashSet<Span>,
}

impl<'hir> EffectiveFlowAnalysis<'hir> for ReentrancyAnalysis<'_, '_, '_> {
    type Domain = bool;

    fn operand_order(&self) -> OperandOrder {
        OperandOrder::Unspecified
    }

    fn apply_expr_effect(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir Expr<'hir>,
        _use_: solar::sema::hir::ExprUse,
        external_call_seen: &mut Self::Domain,
    ) {
        if cx.call_info(expr).is_some_and(|info| info.is_state_mutating_external_interaction()) {
            *external_call_seen = true;
        }
    }

    fn apply_statement_effect(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        stmt: &'hir Stmt<'hir>,
        external_call_seen: &mut Self::Domain,
    ) {
        match stmt.kind {
            StmtKind::Emit(_)
                if *external_call_seen
                    && cx.reports_enabled()
                    && self.emitted.insert(stmt.span) =>
            {
                self.ctx.emit(&REENTRANCY_EVENTS, stmt.span);
            }
            StmtKind::Err(_) => *external_call_seen = true,
            _ => {}
        }
    }

    fn internal_call_mode(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        _call: &'hir Expr<'hir>,
        _callee: FunctionId,
        external_call_seen: &Self::Domain,
    ) -> InternalCallMode {
        if cx.reports_enabled() && *external_call_seen {
            InternalCallMode::Analyze
        } else {
            InternalCallMode::AnalyzeWithoutReports
        }
    }

    fn apply_indirect_internal_call_effect(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        _call: &'hir Expr<'hir>,
        external_call_seen: &mut Self::Domain,
    ) {
        *external_call_seen = true;
    }
}
