use super::LockedEther;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{ContractKind, StateMutability},
    sema::{
        Gcx,
        hir::{
            self, EffectiveBodyCx, EffectiveFlowAnalysis, ExprUse, InternalCallMode, OperandOrder,
            analyze_effective_body_flow_in_contract,
        },
    },
};

declare_forge_lint!(
    LOCKED_ETHER,
    Severity::Med,
    "locked-ether",
    "contract can receive ETH but has no mechanism to send it out"
);

impl<'hir> LateLintPass<'hir> for LockedEther {
    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        contract_id: hir::ContractId,
    ) {
        if !ctx.is_lint_enabled(LOCKED_ETHER.id) {
            return;
        }

        let contract = hir.contract(contract_id);
        if !matches!(contract.kind, ContractKind::Contract | ContractKind::AbstractContract)
            || contract.linearization_failed()
        {
            return;
        }

        let mut has_runtime_inflow = false;
        for &entry in gcx.runtime_entry_points(contract_id) {
            let result = analyze_effective_body_flow_in_contract(
                gcx,
                entry,
                contract_id,
                false,
                &mut NativeExitAnalysis,
            );
            let Some(can_exit) = result.normal_exit() else { continue };
            if can_exit {
                return;
            }
            has_runtime_inflow |= hir.function(entry).state_mutability == StateMutability::Payable;
        }

        // Constructor exits do not make runtime deposits withdrawable. Analyze the constructor
        // only to determine whether deployment value can enter successfully.
        let has_constructor_inflow = contract.ctor.is_some_and(|constructor| {
            hir.function(constructor).state_mutability == StateMutability::Payable
                && analyze_effective_body_flow_in_contract(
                    gcx,
                    constructor,
                    contract_id,
                    false,
                    &mut NativeExitAnalysis,
                )
                .normal_exit()
                .is_some()
        });

        if has_runtime_inflow || has_constructor_inflow {
            ctx.emit(&LOCKED_ETHER, contract.name.span);
        }
    }
}

struct NativeExitAnalysis;

impl<'hir> EffectiveFlowAnalysis<'hir> for NativeExitAnalysis {
    type Domain = bool;

    fn operand_order(&self) -> OperandOrder {
        OperandOrder::Unspecified
    }

    fn apply_expr_effect(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir hir::Expr<'hir>,
        _use_: ExprUse,
        can_exit: &mut Self::Domain,
    ) {
        *can_exit |=
            cx.call_info(expr).is_some_and(|call| call.may_transfer_native_value(cx.gcx(), expr));
    }

    fn internal_call_mode(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        _call: &'hir hir::Expr<'hir>,
        _callee: hir::FunctionId,
        _state: &Self::Domain,
    ) -> InternalCallMode {
        InternalCallMode::AnalyzeWithoutReports
    }

    fn apply_indirect_internal_call_effect(
        &mut self,
        _cx: EffectiveBodyCx<'hir>,
        _call: &'hir hir::Expr<'hir>,
        can_exit: &mut Self::Domain,
    ) {
        *can_exit = true;
    }
}
