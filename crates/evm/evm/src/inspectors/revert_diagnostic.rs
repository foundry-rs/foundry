use alloy_primitives::Address;
use foundry_evm_core::{
    backend::DatabaseExt,
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    decode::DetailedRevertReason,
};
use revm::{
    interpreter::{CallInputs, CallOutcome, CallScheme, InstructionResult},
    precompile::{PrecompileSpecId, Precompiles},
    primitives::SpecId,
    Database, EvmContext, Inspector,
};

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// An inspector that tracks call context to enhances revert diagnostics.
/// Useful for understanding reverts that are not linked to custom errors or revert strings.
#[derive(Clone, Debug, Default)]
pub struct RevertDiagnostic {
    /// Tracks calls with calldata that target an address without executable code
    pub non_contract_call: Option<(Address, CallScheme)>,

    /// Tracks whether a failed call has been spotted or not.
    pub reverted: bool,
}

impl RevertDiagnostic {
    fn is_delegatecall(&self, scheme: CallScheme) -> bool {
        matches!(
            scheme,
            CallScheme::DelegateCall | CallScheme::ExtDelegateCall | CallScheme::CallCode
        )
    }

    fn is_precompile(&self, spec_id: SpecId, target: Address) -> bool {
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id));
        precompiles.contains(&target)
    }

    pub fn no_code_target_address(&self, inputs: &mut CallInputs) -> Address {
        if self.is_delegatecall(inputs.scheme) {
            inputs.bytecode_address
        } else {
            inputs.target_address
        }
    }

    pub fn reason(&self) -> Option<DetailedRevertReason> {
        if !self.reverted {
            return None;
        }

        let reason = match self.non_contract_call {
            Some((addr, scheme)) => {
                if self.is_delegatecall(scheme) {
                    DetailedRevertReason::DelegateCallToNonContract(addr)
                } else {
                    DetailedRevertReason::CallToNonContract(addr)
                }
            }

            None => return None,
        };

        Some(reason)
    }
}

impl<DB: Database + DatabaseExt> Inspector<DB> for RevertDiagnostic {
    /// Tracks the first call, with non-zero calldata, that targeted a non-contract address.
    /// Excludes precompiles and test addresses.
    fn call(&mut self, ctx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let target = self.no_code_target_address(inputs);

        if IGNORE.contains(&target) || self.is_precompile(ctx.spec_id(), target) {
            return None;
        }

        if let Ok(state) = ctx.code(target) {
            if state.is_empty() && !inputs.input.is_empty() && !self.reverted {
                self.non_contract_call = Some((target, inputs.scheme));
            }
        }
        None
    }

    /// Records whether the call reverted or not
    fn call_end(
        &mut self,
        _ctx: &mut EvmContext<DB>,
        _inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        if outcome.result.result == InstructionResult::Revert {
            self.reverted = true
        };

        outcome
    }
}
